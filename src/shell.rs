//! The window: winit events, the OpenGL context, and each frame (egui layout, mpv draws the
//! video into the window's framebuffer, egui paints on top, swap). Nothing here blocks on mpv:
//! its callbacks only post events to this loop.

use std::error::Error;
use std::ffi::OsString;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use glow::HasContext;
use glutin::config::{ConfigTemplateBuilder, GlConfig};
use glutin::context::{ContextApi, ContextAttributesBuilder, NotCurrentGlContext, PossiblyCurrentContext};
use glutin::display::{GetGlDisplay, GlDisplay};
use glutin::surface::{GlSurface, Surface, SwapInterval, WindowSurface};
use glutin_winit::{ApiPreference, DisplayBuilder, GlWindow};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, NamedKey};
use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle};
use winit::window::{Fullscreen, Icon, Window, WindowAttributes, WindowId, WindowLevel};

use crate::action::Action;
use crate::app::{App, Pick, Request, WindowInfo};
use crate::media::{MEDIA_EXTENSIONS, PLAYLIST_EXTENSIONS, SUBTITLE_EXTENSIONS};
use crate::mpv::{NativeDisplay, RenderContext, Target};
use crate::settings::WindowGeometry;

pub enum UserEvent {
    MpvEvents,
    MpvFrame,
    Repaint(Duration),
    Open(Vec<OsString>),
    Probed(String, crate::probe::Info),
    Picked(Pick, Vec<PathBuf>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GlApi {
    /// Linux: EGL, or GLX if EGL fails. Windows: Direct3D 11, or WGL if it fails.
    Auto,
    Egl,
    /// GLX first; on Windows, the driver's own OpenGL (WGL).
    Glx,
}

/// Field order is drop order: mpv's renderer before the graphics objects it draws with, and
/// those before the window.
struct Gfx {
    render: RenderContext,
    backend: Backend,
    egui: egui_winit::State,
    window: Window,
}

enum Backend {
    /// The painter needs the GL context, which needs the surface (fields drop in order).
    Gl {
        painter: egui_glow::Painter,
        gl: Arc<glow::Context>,
        surface: Surface<WindowSurface>,
        context: PossiblyCurrentContext,
    },
    #[cfg(windows)]
    D3d11(crate::d3d11::Gpu),
}

/// Behind the interface when no video is shown.
const BACKGROUND: [f32; 4] = [0.05, 0.05, 0.05, 1.0];

impl Gfx {
    /// mpv's timing moves on without drawing (for a hidden window).
    fn skip_frame(&mut self) {
        let target = match &mut self.backend {
            Backend::Gl { .. } => Some(Target::Gl { fbo: 0, w: 1, h: 1 }),
            #[cfg(windows)]
            Backend::D3d11(gpu) => gpu.back_buffer().ok().map(Target::D3d11),
        };
        if let Some(target) = target {
            // SAFETY: the graphics context is usable on this thread (see `Gfx`).
            unsafe { self.render.render(target, true) };
        }
    }
}

pub struct Shell {
    app: App,
    proxy: EventLoopProxy<UserEvent>,
    /// Set while an mpv callback's event is queued, so a burst of callbacks posts one event.
    events_pending: Arc<AtomicBool>,
    frame_pending: Arc<AtomicBool>,
    gl_api: GlApi,
    /// Files and fullscreen to apply once the window (and so mpv's video output) exists.
    start: Option<(Vec<OsString>, bool)>,
    gfx: Option<Gfx>,
    egui: egui::Context,
    repaint_at: Option<Instant>,
    drops: Vec<OsString>,
    hidden: bool,
    /// The UI as last laid out. A redraw for a new video frame alone paints it again instead of
    /// rebuilding it; anything else that asks for a redraw marks it stale.
    ui: Vec<egui::ClippedPrimitive>,
    ui_scale: f32,
    ui_stale: bool,
    /// Logged once, for start-up measurements.
    first_frame: bool,
}

pub fn event_loop(wayland: bool) -> Result<EventLoop<UserEvent>, winit::error::EventLoopError> {
    let mut builder = EventLoop::<UserEvent>::with_user_event();
    // X11 (XWayland under Wayland), as most players do: winit has no file drag-and-drop on
    // Wayland, and Wayland forbids always-on-top.
    #[cfg(target_os = "linux")]
    if !wayland && std::env::var_os("DISPLAY").is_some() {
        winit::platform::x11::EventLoopBuilderExtX11::with_x11(&mut builder);
    }
    let _ = wayland;
    builder.build()
}

impl Shell {
    pub fn new(
        app: App,
        proxy: EventLoopProxy<UserEvent>,
        gl_api: GlApi,
        files: Vec<OsString>,
        fullscreen: bool,
    ) -> Shell {
        let events_pending = Arc::new(AtomicBool::new(false));
        let (p, pending) = (proxy.clone(), events_pending.clone());
        app.player.mpv.set_wakeup_callback(move || {
            if !pending.swap(true, Ordering::AcqRel) {
                let _ = p.send_event(UserEvent::MpvEvents);
            }
        });
        let egui = egui::Context::default();
        crate::ui::setup(&egui);
        let p = proxy.clone();
        egui.set_request_repaint_callback(move |info| {
            let _ = p.send_event(UserEvent::Repaint(info.delay));
        });
        Shell {
            app,
            proxy,
            events_pending,
            frame_pending: Arc::new(AtomicBool::new(false)),
            gl_api,
            start: Some((files, fullscreen)),
            gfx: None,
            egui,
            repaint_at: None,
            drops: Vec::new(),
            hidden: false,
            ui: Vec::new(),
            ui_scale: 1.0,
            ui_stale: true,
            first_frame: false,
        }
    }

    fn window_attributes(&self) -> WindowAttributes {
        let mut attrs = Window::default_attributes()
            .with_title("VibeLC")
            .with_inner_size(LogicalSize::new(1024.0, 640.0))
            .with_min_inner_size(LogicalSize::new(480.0, 300.0))
            .with_window_icon(window_icon());
        if let Some(g) = self.app.settings.window {
            attrs = attrs
                .with_inner_size(PhysicalSize::new(g.width.max(320), g.height.max(200)))
                .with_position(PhysicalPosition::new(g.x, g.y))
                .with_maximized(g.maximized);
        }
        #[cfg(target_os = "linux")]
        {
            attrs = winit::platform::x11::WindowAttributesExtX11::with_name(attrs, "vibelc", "VibeLC");
        }
        attrs
    }

    /// Linux: EGL, else GLX. Windows: Direct3D 11, which shows D3D11VA frames in place, else the
    /// driver's OpenGL (WGL first, so that a stray libEGL.dll on the PATH is never loaded).
    fn create_gfx(&self, el: &ActiveEventLoop) -> Result<Gfx, Box<dyn Error>> {
        #[cfg(windows)]
        return self.create_d3d11(el).or_else(|e| {
            log::warn!("Direct3D 11 failed ({e}); trying OpenGL");
            self.create_gl(el, ApiPreference::FallbackEgl)
        });
        #[cfg(not(windows))]
        self.create_gl(el, ApiPreference::PreferEgl).or_else(|e| {
            log::warn!("EGL failed ({e}); trying GLX");
            self.create_gl(el, ApiPreference::FallbackEgl)
        })
    }

    #[cfg(windows)]
    fn create_d3d11(&self, el: &ActiveEventLoop) -> Result<Gfx, Box<dyn Error>> {
        let window = el.create_window(self.window_attributes())?;
        let winit::raw_window_handle::RawWindowHandle::Win32(handle) = window.window_handle()?.as_raw() else {
            return Err("not a Win32 window".into());
        };
        let size = window.inner_size();
        let gpu = crate::d3d11::Gpu::new(handle.hwnd.get(), size.width, size.height)?;
        // SAFETY: the device is valid, and `Gfx` uses it and the render context on this thread only.
        let render = unsafe { RenderContext::new_d3d11(&self.app.player.mpv, gpu.device_ptr())? };
        log::info!("Direct3D 11 on {}", gpu.adapter_name());
        let max_texture_side = gpu.max_texture_side();
        Ok(self.finish(window, Backend::D3d11(gpu), render, max_texture_side))
    }

    /// The parts every backend shares: egui's window state and mpv's frame callback.
    fn finish(&self, window: Window, backend: Backend, mut render: RenderContext, max_texture_side: usize) -> Gfx {
        let egui = egui_winit::State::new(
            self.egui.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(max_texture_side),
        );
        let (proxy, pending) = (self.proxy.clone(), self.frame_pending.clone());
        render.set_update_callback(move || {
            if !pending.swap(true, Ordering::AcqRel) {
                let _ = proxy.send_event(UserEvent::MpvFrame);
            }
        });
        Gfx { render, backend, egui, window }
    }

    fn create_gl(&self, el: &ActiveEventLoop, preference: ApiPreference) -> Result<Gfx, Box<dyn Error>> {
        let template = ConfigTemplateBuilder::new().with_transparency(false);
        let (window, config) = DisplayBuilder::new()
            .with_preference(preference)
            .with_window_attributes(Some(self.window_attributes()))
            .build(el, template, |configs| {
                // Opaque and without multisampling: mpv and egui both draw to the default framebuffer.
                configs.min_by_key(|c| (c.alpha_size(), c.num_samples())).expect("no OpenGL configuration")
            })?;
        let window = window.ok_or("no window")?;
        let display = config.display();
        let raw = Some(window.window_handle()?.as_raw());
        // SAFETY: the window outlives the context (field order in `Gfx`).
        let context = unsafe {
            display.create_context(&config, &ContextAttributesBuilder::new().build(raw)).or_else(|_| {
                display.create_context(
                    &config,
                    &ContextAttributesBuilder::new().with_context_api(ContextApi::Gles(None)).build(raw),
                )
            })?
        };
        let surface_attrs = window.build_surface_attributes(Default::default())?;
        // SAFETY: as above.
        let surface = unsafe { display.create_window_surface(&config, &surface_attrs)? };
        let context = context.make_current(&surface)?;
        if let Err(e) = surface.set_swap_interval(&context, SwapInterval::Wait(NonZeroU32::MIN)) {
            log::warn!("no vsync: {e}");
        }
        // SAFETY: the context is current on this thread.
        let gl = Arc::new(unsafe { glow::Context::from_loader_function_cstr(|s| display.get_proc_address(s)) });
        let painter = egui_glow::Painter::new(gl.clone(), "", None, false).map_err(|e| e.to_string())?;
        let native = match window.display_handle()?.as_raw() {
            RawDisplayHandle::Xlib(h) => h.display.map(|d| NativeDisplay::X11(d.as_ptr())),
            RawDisplayHandle::Wayland(h) => Some(NativeDisplay::Wayland(h.display.as_ptr())),
            _ => None,
        };
        let resolver = display.clone();
        // SAFETY: the context is current here and whenever `Gfx` is used or dropped (one window,
        // one context, all on this thread); the display connection outlives the window.
        let render = unsafe {
            RenderContext::new_gl(&self.app.player.mpv, Box::new(move |name| resolver.get_proc_address(name)), native)?
        };
        // mpv's default for direct rendering trusts only AMD's and NVIDIA's drivers.
        let direct = mapped_buffers_cached(&gl);
        if direct {
            self.app.player.set("vd-lavc-dr", "yes");
        }
        log::info!("OpenGL {:?} on {}, cached mapped buffers: {direct}", display.version_string(), gl_renderer(&gl));
        let max_texture_side = painter.max_texture_side();
        Ok(self.finish(window, Backend::Gl { painter, gl, surface, context }, render, max_texture_side))
    }

    fn update_window_info(&mut self) {
        let Some(g) = &self.gfx else { return };
        let w = &g.window;
        let (fullscreen, maximized) = (w.fullscreen().is_some(), w.is_maximized());
        let size = w.inner_size();
        let previous = self.app.win.geometry;
        self.app.win = WindowInfo {
            fullscreen,
            maximized,
            size: (f64::from(size.width), f64::from(size.height)),
            scale: w.scale_factor(),
            monitor: w.current_monitor().map(|m| (f64::from(m.size().width), f64::from(m.size().height))),
            // The normal-size geometry is kept while maximized or fullscreen.
            geometry: match (fullscreen || maximized, w.outer_position()) {
                (false, Ok(p)) => {
                    Some(WindowGeometry { x: p.x, y: p.y, width: size.width, height: size.height, maximized: false })
                }
                _ => previous.map(|g| WindowGeometry { maximized, ..g }),
            },
        };
    }

    fn request_redraw(&mut self) {
        self.ui_stale = true;
        if let Some(g) = &self.gfx {
            g.window.request_redraw();
        }
    }

    fn redraw(&mut self) {
        let Some(g) = &mut self.gfx else { return };
        let size = g.window.inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        let mut textures = egui::TexturesDelta::default();
        if std::mem::take(&mut self.ui_stale) {
            let input = g.egui.take_egui_input(&g.window);
            let app = &mut self.app;
            let output = self.egui.run_ui(input, |ui| crate::ui::frame(ui, app));
            g.egui.handle_platform_output(&g.window, output.platform_output);
            self.app.sync_margins();
            self.ui = self.egui.tessellate(output.shapes, output.pixels_per_point);
            self.ui_scale = output.pixels_per_point;
            textures = output.textures_delta;
        }
        let video = self.app.player.state.has_video;
        match &mut g.backend {
            Backend::Gl { painter, gl, surface, context } => {
                // SAFETY: the GL context is current (see `Gfx`).
                unsafe {
                    if video {
                        g.render.render(Target::Gl { fbo: 0, w: size.width as i32, h: size.height as i32 }, false);
                    }
                    // mpv may leave its own buffers bound; egui uploads textures and draws to the window.
                    gl.bind_framebuffer(glow::FRAMEBUFFER, None);
                    gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, None);
                    if !video {
                        let [r, g, b, a] = BACKGROUND;
                        gl.viewport(0, 0, size.width as i32, size.height as i32);
                        gl.clear_color(r, g, b, a);
                        gl.clear(glow::COLOR_BUFFER_BIT);
                    }
                }
                painter.paint_and_update_textures([size.width, size.height], self.ui_scale, &self.ui, &mut textures);
                if let Err(e) = surface.swap_buffers(context) {
                    log::warn!("swap failed: {e}");
                }
            }
            #[cfg(windows)]
            Backend::D3d11(gpu) => {
                let target = match gpu.back_buffer() {
                    Ok(t) => t,
                    Err(e) => return log::warn!("no back buffer: {e}"),
                };
                if video {
                    // SAFETY: the texture is the device's current back buffer (see `Gfx`).
                    unsafe { g.render.render(Target::D3d11(target), false) };
                } else {
                    gpu.clear(BACKGROUND);
                }
                gpu.paint([size.width, size.height], self.ui_scale, &self.ui, &textures);
                gpu.present();
            }
        }
        if video {
            g.render.report_swap();
            if !std::mem::replace(&mut self.first_frame, true) {
                log::info!("first video frame");
            }
        }
        self.app.redraws += 1;
    }

    /// Carries out what the app asked of the window.
    fn apply_requests(&mut self, el: &ActiveEventLoop) {
        let requests = std::mem::take(&mut self.app.requests);
        let Some(g) = &mut self.gfx else { return };
        for request in requests {
            match request {
                Request::Fullscreen(on) => g.window.set_fullscreen(on.then_some(Fullscreen::Borderless(None))),
                Request::OnTop(on) => {
                    g.window.set_window_level(if on { WindowLevel::AlwaysOnTop } else { WindowLevel::Normal })
                }
                Request::Size(w, h) => {
                    let _ = g.window.request_inner_size(PhysicalSize::new(w, h));
                }
                Request::Title(title) => g.window.set_title(&title),
                Request::ReadClipboard => match g.egui.clipboard_text() {
                    Some(text) => self.app.open_text(&text),
                    None => self.app.osd("The clipboard is empty"),
                },
                Request::Pick(kind) => pick(&g.window, kind, self.proxy.clone()),
                Request::Quit => el.exit(),
            }
        }
        self.ui_stale = true;
        g.window.request_redraw();
    }
}

impl ApplicationHandler<UserEvent> for Shell {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        let gfx = match self.gl_api {
            GlApi::Glx => self.create_gl(el, ApiPreference::FallbackEgl),
            GlApi::Egl => self.create_gl(el, ApiPreference::PreferEgl),
            GlApi::Auto => self.create_gfx(el),
        };
        match gfx {
            Ok(gfx) => self.gfx = Some(gfx),
            Err(e) => {
                log::error!("can't open a window with OpenGL: {e}");
                el.exit();
                return;
            }
        }
        self.update_window_info();
        if let Some((files, fullscreen)) = self.start.take() {
            if !files.is_empty() {
                self.app.open(files, false);
            }
            if fullscreen {
                self.app.run(Action::Fullscreen);
            }
        }
        self.apply_requests(el);
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(g) = &mut self.gfx else { return };
        match &event {
            WindowEvent::CloseRequested => return el.exit(),
            WindowEvent::RedrawRequested => return self.redraw(),
            WindowEvent::DroppedFile(path) => {
                return self.drops.push(path.clone().into_os_string());
            }
            WindowEvent::Occluded(hidden) => self.hidden = *hidden,
            WindowEvent::Resized(size) => {
                match &mut g.backend {
                    Backend::Gl { surface, context, .. } => {
                        if let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) {
                            surface.resize(context, w, h);
                        }
                    }
                    #[cfg(windows)]
                    Backend::D3d11(gpu) => gpu.resize(size.width, size.height),
                }
                self.update_window_info();
            }
            WindowEvent::Moved(_) | WindowEvent::ScaleFactorChanged { .. } => self.update_window_info(),
            WindowEvent::KeyboardInput { event: key, .. } if key.state == ElementState::Pressed => {
                // egui has no media keys.
                let action = match key.logical_key {
                    Key::Named(NamedKey::MediaPlayPause | NamedKey::MediaPlay | NamedKey::MediaPause) => {
                        Some(Action::PlayPause)
                    }
                    Key::Named(NamedKey::MediaStop) => Some(Action::Stop),
                    Key::Named(NamedKey::MediaTrackNext) => Some(Action::Next),
                    Key::Named(NamedKey::MediaTrackPrevious) => Some(Action::Previous),
                    _ => None,
                };
                if let Some(action) = action {
                    self.app.run(action);
                    return self.apply_requests(el);
                }
            }
            _ => {}
        }
        let Some(g) = &mut self.gfx else { return };
        if g.egui.on_window_event(&g.window, &event).repaint {
            self.ui_stale = true;
            g.window.request_redraw();
        }
    }

    fn user_event(&mut self, el: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::MpvEvents => {
                // Cleared first, so a wakeup while draining posts a new event.
                self.events_pending.store(false, Ordering::Release);
                if self.app.on_mpv_events() {
                    self.request_redraw();
                }
            }
            UserEvent::MpvFrame => {
                self.frame_pending.store(false, Ordering::Release);
                let Some(g) = &mut self.gfx else { return };
                if g.render.frame_due() {
                    if self.hidden || g.window.is_minimized() == Some(true) {
                        // Nothing to show, but mpv's timing moves on.
                        g.skip_frame();
                    } else {
                        g.window.request_redraw();
                    }
                }
            }
            UserEvent::Repaint(delay) => {
                let at = Instant::now() + delay.min(Duration::from_secs(3600));
                self.repaint_at = Some(self.repaint_at.map_or(at, |t| t.min(at)));
            }
            UserEvent::Open(items) => {
                self.app.open(items, false);
                if let Some(g) = &self.gfx {
                    g.window.set_minimized(false);
                    g.window.focus_window();
                }
            }
            UserEvent::Probed(location, info) => {
                self.app.probed.insert(location, info);
                if self.app.settings.show_playlist {
                    self.request_redraw();
                }
            }
            UserEvent::Picked(kind, paths) => self.app.picked(kind, paths),
        }
        if !self.app.requests.is_empty() {
            self.apply_requests(el);
        }
    }

    fn new_events(&mut self, _el: &ActiveEventLoop, cause: StartCause) {
        if let StartCause::ResumeTimeReached { .. } = cause {
            let now = Instant::now();
            if self.app.tick(now) {
                self.request_redraw();
            }
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        if !self.drops.is_empty() {
            let items = std::mem::take(&mut self.drops);
            self.app.open(items, false);
            self.request_redraw();
        }
        let now = Instant::now();
        if self.repaint_at.is_some_and(|t| t <= now) {
            self.repaint_at = None;
            self.request_redraw();
        }
        if !self.app.requests.is_empty() {
            self.apply_requests(el);
        }
        let deadline = [self.repaint_at, self.app.next_deadline()].into_iter().flatten().min();
        el.set_control_flow(deadline.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
    }

    fn exiting(&mut self, _el: &ActiveEventLoop) {
        self.app.shutdown();
        if let Some(Gfx { backend: Backend::Gl { painter, .. }, .. }) = &mut self.gfx {
            painter.destroy();
        }
        self.gfx = None;
    }
}

/// Shows a file dialog on its own thread (it is modal and would otherwise stop the video).
fn pick(window: &Window, kind: Pick, proxy: EventLoopProxy<UserEvent>) {
    let dialog = rfd::FileDialog::new().set_parent(window);
    let dialog = match kind {
        Pick::Open | Pick::Add => dialog
            .set_title(if kind == Pick::Open { "Open Files" } else { "Add Files to Playlist" })
            .add_filter("Media and playlists", &[MEDIA_EXTENSIONS, PLAYLIST_EXTENSIONS].concat())
            .add_filter("All files", &["*"]),
        Pick::Subtitle => dialog.set_title("Add Subtitle File").add_filter("Subtitles", SUBTITLE_EXTENSIONS),
        Pick::SavePlaylist => {
            dialog.set_title("Save Playlist").add_filter("M3U playlist", &["m3u8"]).set_file_name("playlist.m3u8")
        }
        Pick::Folder => dialog.set_title("Open Folder"),
        Pick::SnapshotFolder => dialog.set_title("Folder for Snapshots"),
    };
    let _ = std::thread::Builder::new().name("file-dialog".into()).spawn(move || {
        let paths = match kind {
            Pick::Open | Pick::Add | Pick::Subtitle => dialog.pick_files().unwrap_or_default(),
            Pick::Folder | Pick::SnapshotFolder => dialog.pick_folder().into_iter().collect(),
            Pick::SavePlaylist => dialog.save_file().into_iter().collect(),
        };
        if !paths.is_empty() {
            let _ = proxy.send_event(UserEvent::Picked(kind, paths));
        }
    });
}

fn window_icon() -> Option<Icon> {
    let (rgba, w, h) = crate::ui::decode_png(include_bytes!("../assets/icon/vibelc-64.png"))?;
    Icon::from_rgba(rgba, w, h).ok()
}

/// Whether the driver's persistently mapped buffers are CPU-cached. Software decoders can then
/// write frames straight into them (mpv's direct rendering), one full-frame copy less each; in
/// uncached memory, their reads of reference frames would crawl. Uncached reads are many times
/// slower than cached ones, so timing one settles it (a wrong "no" only keeps mpv's default).
fn mapped_buffers_cached(gl: &glow::Context) -> bool {
    const SIZE: usize = 4 << 20;
    let has = |ext: &str| gl.supported_extensions().contains(ext);
    let v = gl.version();
    if !(has("GL_ARB_buffer_storage") || has("GL_EXT_buffer_storage") || !v.is_embedded && (v.major, v.minor) >= (4, 4))
    {
        return false;
    }
    let read_time = |data: &[u8]| {
        let start = Instant::now();
        std::hint::black_box(data.iter().fold(0u8, |a, &b| a ^ b));
        start.elapsed()
    };
    // The flags mpv's OpenGL backend uses for these buffers.
    let map = glow::MAP_READ_BIT | glow::MAP_WRITE_BIT | glow::MAP_PERSISTENT_BIT | glow::MAP_COHERENT_BIT;
    // SAFETY: the context is current (see `Gfx`); the slice covers the mapping and is dropped
    // before the buffer is unmapped.
    unsafe {
        let Ok(buffer) = gl.create_buffer() else { return false };
        gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, Some(buffer));
        let storage = map | glow::DYNAMIC_STORAGE_BIT | glow::CLIENT_STORAGE_BIT;
        gl.buffer_storage(glow::PIXEL_UNPACK_BUFFER, SIZE as i32, None, storage);
        let ptr = gl.map_buffer_range(glow::PIXEL_UNPACK_BUFFER, 0, SIZE as i32, map);
        let cached = !ptr.is_null() && {
            let mapped = std::slice::from_raw_parts_mut(ptr, SIZE);
            mapped.fill(1);
            let heap = vec![1u8; SIZE];
            let heap_time = (0..3).map(|_| read_time(&heap)).min().unwrap_or_default();
            read_time(mapped) < heap_time * 4
        };
        if !ptr.is_null() {
            gl.unmap_buffer(glow::PIXEL_UNPACK_BUFFER);
        }
        gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, None);
        gl.delete_buffer(buffer);
        cached
    }
}

fn gl_renderer(gl: &glow::Context) -> String {
    // SAFETY: a plain query on the current context.
    unsafe { gl.get_parameter_string(glow::RENDERER) }
}
