//! The window's contents, rebuilt every frame (egui is immediate mode). The video itself is
//! drawn by mpv underneath; the central area stays transparent.

mod controls;
mod dialogs;
mod menus;
mod playlist;

use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{
    Align2, CentralPanel, Color32, Context, CursorIcon, FontData, FontDefinitions, FontFamily, FontId, Panel, Rect,
    RichText, Sense, Ui, Vec2, vec2,
};

use crate::action::{self, Action, Dialog};
use crate::app::App;
use crate::player::State;
use crate::settings::{Settings, Theme};

pub const ACCENT: Color32 = Color32::from_rgb(0x2f, 0x80, 0xed);
const OSD_TIME: Duration = Duration::from_millis(1500);
const ERROR_TIME: Duration = Duration::from_secs(6);
const RESUME_TIME: Duration = Duration::from_secs(8);
const CONTROLS_TIME: Duration = Duration::from_millis(2500);

/// Lucide glyphs in `assets/fonts/lucide-subset.ttf`.
pub mod icon {
    pub const PLAY: char = '\u{e13c}';
    pub const PAUSE: char = '\u{e12e}';
    pub const STOP: char = '\u{e167}';
    pub const PREVIOUS: char = '\u{e15f}';
    pub const NEXT: char = '\u{e160}';
    pub const FULLSCREEN: char = '\u{e112}';
    pub const LEAVE_FULLSCREEN: char = '\u{e11a}';
    pub const EFFECTS: char = '\u{e29a}';
    pub const PLAYLIST: char = '\u{e2e2}';
    pub const LOOP: char = '\u{e146}';
    pub const LOOP_ONE: char = '\u{e1fd}';
    pub const SHUFFLE: char = '\u{e15e}';
    pub const VOLUME_HIGH: char = '\u{e1ab}';
    pub const VOLUME_LOW: char = '\u{e1aa}';
    pub const VOLUME_OFF: char = '\u{e1a9}';
    pub const MUTED: char = '\u{e1ac}';
    pub const CHECK: char = '\u{e06c}';
    pub const ADD: char = '\u{e13d}';
    pub const CLEAR: char = '\u{e18e}';
    #[cfg(test)]
    pub const ALL: [char; 19] = [
        PLAY,
        PAUSE,
        STOP,
        PREVIOUS,
        NEXT,
        FULLSCREEN,
        LEAVE_FULLSCREEN,
        EFFECTS,
        PLAYLIST,
        LOOP,
        LOOP_ONE,
        SHUFFLE,
        VOLUME_HIGH,
        VOLUME_LOW,
        VOLUME_OFF,
        MUTED,
        CHECK,
        ADD,
        CLEAR,
    ];
}

pub fn icon_text(glyph: char, size: f32) -> RichText {
    RichText::new(glyph.to_string()).family(FontFamily::Name("icons".into())).size(size)
}

/// View state that isn't playback state or a setting.
#[derive(Default)]
pub struct View {
    pub minimal: bool,
    pub on_top: bool,
    pub dialogs: Vec<Dialog>,
    pub url: String,
    pub jump: String,
    pub info_tab: usize,
    pub effects_tab: usize,
    pub log_level: usize,
    pub osd: Option<(String, Instant)>,
    pub error: Option<(String, Instant)>,
    /// Offer to continue `location` from a position.
    pub resume: Option<(String, f64, Instant)>,
    pub seek_drag: Option<f64>,
    pub show_remaining: bool,
    /// Last pointer movement, which shows the fullscreen controls.
    pub activity: Option<Instant>,
    pub video_rect: Option<Rect>,
    pub seek_width_px: f64,
    pub playlist_selected: Option<usize>,
    /// The text field of a dialog that just opened takes the keyboard focus.
    pub focus: bool,
    scroll: f32,
    theme: Option<Theme>,
    logo: Option<egui::TextureHandle>,
}

impl View {
    pub fn show(&mut self, dialog: Dialog, s: &State) {
        if dialog == Dialog::JumpToTime {
            self.jump = crate::format::time(s.time);
        }
        if !self.dialogs.contains(&dialog) {
            self.dialogs.push(dialog);
            self.focus = matches!(dialog, Dialog::OpenUrl | Dialog::JumpToTime);
        }
    }

    /// Whether something on screen shows the polled statistics.
    pub fn shows_stats(&self, settings: &Settings) -> bool {
        settings.show_status_bar || (self.dialogs.contains(&Dialog::MediaInfo) && self.info_tab == 2)
    }

    pub fn next_deadline(&self, fullscreen: bool) -> Option<Instant> {
        let controls = if fullscreen { self.activity.map(|t| t + CONTROLS_TIME) } else { None };
        [
            self.osd.as_ref().map(|o| o.1 + OSD_TIME),
            self.error.as_ref().map(|e| e.1 + ERROR_TIME),
            self.resume.as_ref().map(|r| r.2 + RESUME_TIME),
            controls,
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// One volume step per mouse wheel notch; touchpads count 50 points as a notch.
    pub fn wheel_volume(&mut self, ui: &Ui, actions: &mut Vec<Action>) {
        self.scroll += ui.input(|i| {
            i.events
                .iter()
                .map(|e| match e {
                    egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta, .. } => delta.y / 50.0,
                    egui::Event::MouseWheel { delta, .. } => delta.y,
                    _ => 0.0,
                })
                .sum::<f32>()
        });
        while self.scroll.abs() >= 1.0 {
            let step = self.scroll.signum();
            self.scroll -= step;
            actions.push(Action::Volume(step as i8));
        }
    }

    fn controls_visible(&self, now: Instant) -> bool {
        self.activity.is_some_and(|t| now < t + CONTROLS_TIME)
    }
}

/// Fonts and the light and dark styles; called once.
pub fn setup(ctx: &Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "inter".into(),
        Arc::new(FontData::from_static(include_bytes!("../../assets/fonts/Inter-Regular.ttf"))),
    );
    fonts.font_data.insert(
        "icons".into(),
        Arc::new(FontData::from_static(include_bytes!("../../assets/fonts/lucide-subset.ttf"))),
    );
    fonts.families.insert(FontFamily::Proportional, vec!["inter".into()]);
    fonts.families.insert(FontFamily::Monospace, vec!["inter".into()]);
    fonts.families.insert(FontFamily::Name("icons".into()), vec!["icons".into()]);
    ctx.set_fonts(fonts);
    ctx.all_styles_mut(|style| {
        style.visuals.selection.bg_fill = ACCENT;
        style.visuals.selection.stroke.color = Color32::WHITE;
        style.visuals.hyperlink_color = ACCENT;
        style.spacing.button_padding = vec2(6.0, 3.0);
    });
    load_cjk_fallback(ctx.clone());
}

/// Adds a system font for Chinese, Japanese and Korean text, if one is installed. It is large
/// (about 20 MB), so it is mapped rather than read: only the glyphs drawn come into memory.
fn load_cjk_fallback(ctx: Context) {
    const CANDIDATES: &[&str] = &[
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
        "C:\\Windows\\Fonts\\msyh.ttc",
        "C:\\Windows\\Fonts\\YuGothM.ttc",
        "C:\\Windows\\Fonts\\malgun.ttf",
    ];
    let _ = std::thread::Builder::new().name("fonts".into()).spawn(move || {
        // SAFETY: system fonts are not modified while in use; if one were, glyphs would be wrong.
        let map = |p: &&str| std::fs::File::open(p).and_then(|f| unsafe { memmap2::Mmap::map(&f) }).ok();
        let Some(font) = CANDIDATES.iter().find_map(map) else {
            return;
        };
        ctx.add_font(egui::epaint::text::FontInsert {
            name: "cjk".into(),
            data: FontData::from_blob(std::sync::Arc::new(font), 0),
            families: vec![egui::epaint::text::InsertFontFamily {
                family: FontFamily::Proportional,
                priority: egui::epaint::text::FontPriority::Lowest,
            }],
        });
        ctx.request_repaint();
    });
}

/// Builds one frame.
pub fn frame(ui: &mut Ui, app: &mut App) {
    let ctx = ui.ctx().clone();
    if app.view.theme != Some(app.settings.theme) {
        app.view.theme = Some(app.settings.theme);
        ctx.set_theme(match app.settings.theme {
            Theme::System => egui::ThemePreference::System,
            Theme::Light => egui::ThemePreference::Light,
            Theme::Dark => egui::ThemePreference::Dark,
        });
    }
    // Shortcuts first, so a focused button or slider never sees their keys.
    let (actions, pasted) = action::take_shortcuts(&ctx);
    for a in actions {
        app.run(a);
    }
    if let Some(text) = pasted {
        app.open_text(&text);
    }

    let now = Instant::now();
    let v = &mut app.view;
    v.osd = v.osd.take().filter(|o| now < o.1 + OSD_TIME);
    v.error = v.error.take().filter(|e| now < e.1 + ERROR_TIME);
    v.resume = v.resume.take().filter(|r| now < r.2 + RESUME_TIME);
    v.activity = v.activity.filter(|t| now < *t + CONTROLS_TIME);
    // The seek bar sets this again when it is drawn, so a hidden bar asks for no redraws.
    v.seek_width_px = 0.0;
    if ctx.input(|i| i.pointer.delta() != Vec2::ZERO || i.pointer.any_down()) {
        v.activity = Some(now);
    }

    let fullscreen = app.win.fullscreen;
    if !fullscreen && !app.view.minimal {
        Panel::top("menu").show(ui, |ui| menus::bar(ui, app));
        if app.settings.show_status_bar {
            Panel::bottom("status").show(ui, |ui| status_bar(ui, &app.player.state));
        }
        Panel::bottom("controls").show(ui, |ui| controls::show(ui, app));
        if app.settings.show_playlist {
            Panel::right("playlist")
                .default_size(320.0)
                .size_range(200.0..=700.0)
                .show(ui, |ui| playlist::show(ui, app));
        }
    }
    CentralPanel::no_frame().show(ui, |ui| video_area(ui, app));

    if fullscreen {
        let pointer_on_controls = ctx.pointer_hover_pos().is_some_and(|p| p.y > ctx.content_rect().bottom() - 90.0);
        if pointer_on_controls {
            app.view.activity = Some(now);
        }
        if app.view.controls_visible(now) {
            let width = (ctx.content_rect().width() * 0.7).clamp(480.0, 1100.0);
            egui::Area::new(egui::Id::unique("fullscreen-controls"))
                .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -16.0))
                .show(&ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_width(width);
                        controls::show(ui, app);
                    });
                });
        } else {
            ctx.set_cursor_icon(CursorIcon::None);
        }
    }

    dialogs::show(&ctx, app);
    for a in std::mem::take(&mut app.actions) {
        app.run(a);
    }
}

fn video_area(ui: &mut Ui, app: &mut App) {
    let rect = ui.max_rect();
    app.view.video_rect = Some(rect);
    let response = ui.interact(rect, egui::Id::unique("video"), Sense::click());
    if response.double_clicked() {
        app.actions.push(Action::Fullscreen);
    }
    response.context_menu(|ui| menus::context(ui, app));
    if response.hovered() {
        app.view.wheel_volume(ui, &mut app.actions);
    }

    let painter = ui.painter_at(rect);
    let s = &app.player.state;
    if !s.has_video {
        painter.rect_filled(rect, 0.0, Color32::from_gray(12));
        let center = rect.center();
        let (title, sub) = match (&s.title, s.idle) {
            (Some(t), false) => {
                (t.clone(), s.metadata.str_of("artist").or(s.metadata.str_of("ARTIST")).unwrap_or("").to_string())
            }
            _ => (
                "VibeLC".to_string(),
                format!("Drop media here or press {}", action::hint(Action::OpenFiles).unwrap_or_default()),
            ),
        };
        let logo = app.view.logo.get_or_insert_with(|| {
            let (rgba, w, h) =
                decode_png(include_bytes!("../../assets/icon/vibelc-128.png")).unwrap_or((vec![0; 4], 1, 1));
            let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
            ui.ctx().load_texture("logo", image, egui::TextureOptions::LINEAR)
        });
        if rect.height() > 260.0 {
            let logo_rect = Rect::from_center_size(center - vec2(0.0, 60.0), vec2(96.0, 96.0));
            painter.image(
                logo.id(),
                logo_rect,
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        painter.text(
            center + vec2(0.0, 20.0),
            Align2::CENTER_CENTER,
            title,
            FontId::proportional(22.0),
            Color32::from_gray(230),
        );
        painter.text(
            center + vec2(0.0, 50.0),
            Align2::CENTER_CENTER,
            sub,
            FontId::proportional(14.0),
            Color32::from_gray(150),
        );
    }

    let top_left = rect.left_top() + vec2(16.0, 16.0);
    if let Some((text, _)) = &app.view.osd {
        bubble(&painter, top_left, Align2::LEFT_TOP, text, Color32::WHITE);
    } else if let Some(pct) = s.buffering.filter(|_| s.network && !s.idle) {
        bubble(&painter, top_left, Align2::LEFT_TOP, &format!("Buffering {pct}%"), Color32::WHITE);
    }
    if let Some((text, _)) = &app.view.error {
        bubble(
            &painter,
            rect.center_bottom() - vec2(0.0, 24.0),
            Align2::CENTER_BOTTOM,
            text,
            Color32::from_rgb(255, 170, 160),
        );
    }
    if let Some((location, pos, _)) = app.view.resume.clone() {
        egui::Area::new(egui::Id::unique("resume"))
            .fixed_pos(rect.center_top() + vec2(0.0, 16.0))
            .pivot(Align2::CENTER_TOP)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(format!("Continue from {}?", crate::format::time(Some(pos))));
                        if ui.button("Continue").clicked() {
                            if app.player.state.path.as_deref() == Some(location.as_str()) {
                                app.player.command(&["seek", &pos.to_string(), "absolute"]);
                            }
                            app.view.resume = None;
                        }
                        if ui.button("Start over").clicked() {
                            app.view.resume = None;
                        }
                    });
                });
            });
    }
}

/// Text on a dark rounded background, readable over any picture.
fn bubble(painter: &egui::Painter, pos: egui::Pos2, align: Align2, text: &str, color: Color32) {
    let galley = painter.layout_no_wrap(text.to_string(), FontId::proportional(18.0), color);
    let rect = align.anchor_size(pos, galley.size() + vec2(20.0, 12.0));
    painter.rect_filled(rect, 6.0, Color32::from_black_alpha(170));
    painter.galley(rect.min + vec2(10.0, 6.0), galley, color);
}

fn status_bar(ui: &mut Ui, s: &State) {
    ui.horizontal(|ui| {
        let mut parts = vec![match &s.hwdec {
            Some(h) => format!("Hardware decoding ({h})"),
            None if s.has_video => "Software decoding".to_string(),
            None => "No video".to_string(),
        }];
        if let Some((w, h)) = s.video_size() {
            parts.push(format!("{w:.0}×{h:.0}"));
        }
        if let Some(fps) = s.polled("estimated-vf-fps").as_f64() {
            parts.push(format!("{fps:.2} fps"));
        }
        let drops = s.polled("frame-drop-count").as_i64().unwrap_or(0)
            + s.polled("decoder-frame-drop-count").as_i64().unwrap_or(0);
        parts.push(format!("{drops} frames dropped"));
        parts.push(format!("speed {:.2}x", s.speed));
        if s.network
            && let Some(cache) = s.polled("demuxer-cache-state").f64_of("cache-duration")
        {
            parts.push(format!("{cache:.0} s buffered"));
        }
        ui.label(RichText::new(parts.join("   ·   ")).small());
    });
}

/// Decodes an 8-bit RGBA PNG (the bundled icons).
pub fn decode_png(bytes: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    (info.color_type == png::ColorType::Rgba && info.bit_depth == png::BitDepth::Eight).then(|| {
        buf.truncate(info.buffer_size());
        (buf, info.width, info.height)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_in_the_font_and_the_logo_decodes() {
        let ctx = Context::default();
        setup(&ctx);
        ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
        let font = FontId::new(16.0, FontFamily::Name("icons".into()));
        for glyph in icon::ALL {
            assert!(ctx.fonts_mut(|f| f.has_glyph(&font, glyph)), "U+{:04X} missing", glyph as u32);
        }
        assert_eq!(
            decode_png(include_bytes!("../../assets/icon/vibelc-128.png")).map(|d| (d.1, d.2)),
            Some((128, 128))
        );
    }
}
