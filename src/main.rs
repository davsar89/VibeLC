//! VibeLC: fully vibe coded lightweight vlc like video player. Playback is libmpv's; the window, controls,
//! playlist handling and settings are VibeLC's.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod action;
mod app;
mod audio_filters;
#[cfg(windows)]
mod d3d11;
mod format;
mod instance;
mod logging;
mod media;
mod mpv;
mod player;
mod probe;
mod settings;
mod shell;
mod ui;

use std::ffi::OsString;
use std::process::ExitCode;
use std::sync::Arc;

use settings::{RenderProfile, Settings};
use shell::{GlApi, Shell, UserEvent};

const HELP: &str = "\
Usage: vibelc [OPTIONS] [FILE | FOLDER | URL]...

Options:
  -f, --fullscreen    Start in fullscreen
      --quit-at-end   Quit when the playlist ends
      --hwdec=MODE    mpv hardware decoding mode (default auto-safe; no = software)
      --profile=NAME  Video quality: fast, balanced or high
      --gl=API        Graphics API. Linux: egl or glx (default: EGL, falling back to GLX).
                      Windows: d3d11 or wgl (default: Direct3D 11, falling back to the
                      driver's OpenGL, WGL)
      --wayland       Use Wayland even when X11 is available
      --safe          Software decoding and the fast profile, for troublesome graphics drivers
      --new-window    Don't hand the files to a VibeLC window that is already open
      --stats         Log redraws and dropped frames every second
  -v, --verbose       Detailed log, including mpv's
  -h, --help          Show this help
  -V, --version       Show the version
";

#[derive(Default)]
struct Cli {
    files: Vec<OsString>,
    fullscreen: bool,
    quit_at_end: bool,
    hwdec: Option<String>,
    profile: Option<RenderProfile>,
    gl: Option<GlApi>,
    wayland: bool,
    safe: bool,
    new_window: bool,
    stats: bool,
    verbose: bool,
}

/// `None` when only help or the version was asked for.
fn parse_args() -> Result<Option<Cli>, lexopt::Error> {
    use lexopt::prelude::*;
    let mut cli = Cli::default();
    let mut parser = lexopt::Parser::from_env();
    while let Some(arg) = parser.next()? {
        match arg {
            Short('f') | Long("fullscreen") => cli.fullscreen = true,
            Long("quit-at-end") => cli.quit_at_end = true,
            Long("hwdec") => cli.hwdec = Some(parser.value()?.string()?),
            Long("profile") => {
                cli.profile = Some(match parser.value()?.string()?.as_str() {
                    "fast" => RenderProfile::Fast,
                    "balanced" => RenderProfile::Balanced,
                    "high" => RenderProfile::HighQuality,
                    other => return Err(format!("unknown profile {other:?}").into()),
                })
            }
            Long("gl") => {
                cli.gl = Some(match parser.value()?.string()?.as_str() {
                    "d3d11" => GlApi::Auto,
                    "egl" => GlApi::Egl,
                    // On Windows, "glx" is the driver's own OpenGL (WGL).
                    "glx" | "wgl" => GlApi::Glx,
                    other => return Err(format!("unknown OpenGL API {other:?}").into()),
                })
            }
            Long("wayland") => cli.wayland = true,
            Long("safe") => cli.safe = true,
            Long("new-window") => cli.new_window = true,
            Long("stats") => cli.stats = true,
            Short('v') | Long("verbose") => cli.verbose = true,
            Short('h') | Long("help") => {
                print!("{HELP}");
                return Ok(None);
            }
            Short('V') | Long("version") => {
                println!("vibelc {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            Value(v) => cli.files.push(v),
            _ => return Err(arg.unexpected()),
        }
    }
    Ok(Some(cli))
}

fn fail(message: &str) -> ExitCode {
    log::error!("{message}");
    rfd::MessageDialog::new().set_level(rfd::MessageLevel::Error).set_title("VibeLC").set_description(message).show();
    ExitCode::FAILURE
}

fn main() -> ExitCode {
    // libmpv falls back to SDL audio when there is no sound server, and SDL then takes over SIGINT
    // and SIGTERM, so VibeLC would ignore them.
    // SAFETY: no other thread exists yet.
    unsafe { std::env::set_var("SDL_NO_SIGNAL_HANDLERS", "1") };
    let cli = match parse_args() {
        Ok(Some(cli)) => cli,
        Ok(None) => return ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("vibelc: {e}\n\n{HELP}");
            return ExitCode::from(2);
        }
    };
    logging::init(cli.verbose);
    std::panic::set_hook(Box::new(|info| log::error!("{info}\n{}", std::backtrace::Backtrace::force_capture())));

    let settings_path = settings::config_dir().map(|d| d.join("settings.toml"));
    let settings = settings_path.as_deref().map(Settings::load).unwrap_or_default();
    let single = settings.single_instance && !cli.new_window;
    let other_window = single && instance::forward(&cli.files);
    if other_window && !cli.files.is_empty() {
        log::info!("handed {} item(s) to the open VibeLC window", cli.files.len());
        return ExitCode::SUCCESS;
    }

    let api = match mpv::Api::load() {
        Ok(api) => Arc::new(api),
        Err(e) => return fail(&e.to_string()),
    };
    log::info!("VibeLC {} with libmpv client API {}", env!("CARGO_PKG_VERSION"), api.version_string());

    // Command-line overrides apply to this run only, so they are not saved.
    let mut startup = settings.clone();
    if let Some(p) = cli.profile.or(cli.safe.then_some(RenderProfile::Fast)) {
        startup.profile = p;
    }
    startup.hardware_decoding &= !cli.safe;
    let hwdec = cli.hwdec.clone().unwrap_or_else(|| startup.hwdec().to_string());
    let player = match player::Player::new(api.clone(), &startup, &hwdec) {
        Ok(p) => p,
        Err(e) => return fail(&format!("mpv could not start: {e}")),
    };
    if let Err(e) = player.mpv.request_log_messages(if cli.verbose { "v" } else { "info" }) {
        log::warn!("no mpv log: {e}");
    }

    let event_loop = match shell::event_loop(cli.wayland) {
        Ok(el) => el,
        Err(e) => return fail(&format!("can't connect to the display: {e}")),
    };
    let proxy = event_loop.create_proxy();
    let probe_proxy = proxy.clone();
    let prober = probe::Prober::start(api, move |location, info| {
        let _ = probe_proxy.send_event(UserEvent::Probed(location, info));
    });
    if single && !other_window {
        let instance_proxy = proxy.clone();
        instance::listen(move |items| {
            let _ = instance_proxy.send_event(UserEvent::Open(items));
        });
    }

    let mut app = app::App::new(player, settings, settings_path, prober);
    app.quit_at_end = cli.quit_at_end;
    app.stats = cli.stats;
    let mut shell = Shell::new(app, proxy, cli.gl.unwrap_or(GlApi::Auto), cli.files, cli.fullscreen);
    match event_loop.run_app(&mut shell) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(&e.to_string()),
    }
}
