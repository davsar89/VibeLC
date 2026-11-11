//! Log to stderr, to `vibelc.log` in the config folder (the previous run's is kept as
//! `vibelc.old.log`) and to a buffer that the Messages window shows.

use std::collections::VecDeque;
use std::io::Write;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use log::Level;

const KEEP_LINES: usize = 2000;

pub struct Line {
    pub level: Level,
    pub text: String,
}

struct Logger {
    start: Instant,
    file: Mutex<Option<std::fs::File>>,
    lines: Mutex<VecDeque<Line>>,
}

static LOGGER: OnceLock<Logger> = OnceLock::new();

impl log::Log for Logger {
    fn enabled(&self, m: &log::Metadata) -> bool {
        // Debug detail only from VibeLC and mpv, not from the GUI libraries.
        let ours = m.target().starts_with("vibelc") || m.target() == "mpv";
        m.level() <= Level::Warn || (ours && m.level() <= log::max_level())
    }

    fn log(&self, r: &log::Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let text = format!("{}: {}", r.target().trim_start_matches("vibelc::"), r.args()).trim_end().to_string();
        let line = format!("[{:9.3}] {:5} {text}\n", self.start.elapsed().as_secs_f64(), r.level());
        let _ = std::io::stderr().write_all(line.as_bytes());
        if let Some(f) = self.file.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            let _ = f.write_all(line.as_bytes());
        }
        let mut lines = self.lines.lock().unwrap_or_else(|e| e.into_inner());
        if lines.len() == KEEP_LINES {
            lines.pop_front();
        }
        lines.push_back(Line { level: r.level(), text });
    }

    fn flush(&self) {}
}

pub fn init(verbose: bool) {
    let file = crate::settings::config_dir().and_then(|dir| {
        std::fs::create_dir_all(&dir).ok()?;
        let _ = std::fs::rename(dir.join("vibelc.log"), dir.join("vibelc.old.log"));
        std::fs::File::create(dir.join("vibelc.log")).ok()
    });
    let logger = LOGGER.get_or_init(|| Logger {
        start: Instant::now(),
        file: Mutex::new(file),
        lines: Mutex::new(VecDeque::new()),
    });
    if log::set_logger(logger).is_ok() {
        log::set_max_level(if verbose { log::LevelFilter::Debug } else { log::LevelFilter::Info });
    }
}

/// Runs `f` on the buffered lines, oldest first.
pub fn with_lines<R>(f: impl FnOnce(&mut VecDeque<Line>) -> R) -> Option<R> {
    LOGGER.get().map(|l| f(&mut l.lines.lock().unwrap_or_else(|e| e.into_inner())))
}

/// Passes one of mpv's own log messages on.
pub fn mpv_message(prefix: &str, level: &str, text: &str) {
    let level = match level {
        "fatal" | "error" => Level::Error,
        "warn" => Level::Warn,
        "info" => Level::Info,
        "v" => Level::Debug,
        _ => Level::Trace,
    };
    log::log!(target: "mpv", level, "[{prefix}] {}", text.trim_end());
}
