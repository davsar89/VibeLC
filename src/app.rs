//! The application: reacts to mpv events and user actions. The window shell runs it; the UI
//! (`ui`) draws it and queues [`Action`]s.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use crate::action::Action;
use crate::media;
use crate::mpv::{EndReason, Event};
use crate::player::{ASPECTS, Player, REPLY_SNAPSHOT, REPLY_SUBTITLE, track_label};
use crate::probe::{self, Prober};
use crate::settings::{LoopMode, Resume, Settings, WindowGeometry};
use crate::ui::View;

/// Something only the window shell can do.
#[derive(Debug, PartialEq)]
pub enum Request {
    Fullscreen(bool),
    OnTop(bool),
    /// Inner window size in physical pixels.
    Size(f64, f64),
    Title(String),
    Pick(Pick),
    ReadClipboard,
    Quit,
}

/// File dialogs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pick {
    Open,
    Add,
    Folder,
    Subtitle,
    SavePlaylist,
    SnapshotFolder,
}

/// Window facts the shell updates before each frame.
#[derive(Debug, Default, Clone, Copy)]
pub struct WindowInfo {
    pub fullscreen: bool,
    pub maximized: bool,
    /// Inner size in physical pixels.
    pub size: (f64, f64),
    pub scale: f64,
    pub monitor: Option<(f64, f64)>,
    pub geometry: Option<WindowGeometry>,
}

const POLL_INTERVAL: Duration = Duration::from_secs(1);
const MARGINS: [&str; 4] =
    ["video-margin-ratio-left", "video-margin-ratio-right", "video-margin-ratio-top", "video-margin-ratio-bottom"];

pub struct App {
    pub player: Player,
    pub settings: Settings,
    settings_path: Option<PathBuf>,
    pub view: View,
    pub win: WindowInfo,
    pub requests: Vec<Request>,
    /// Actions queued by the UI during a frame.
    pub actions: Vec<Action>,
    prober: Prober,
    pub probed: HashMap<String, probe::Info>,
    awake: Sender<bool>,
    awake_on: bool,
    /// Last known position of the current file, for resuming it later.
    resume_point: Option<(String, f64, Option<f64>)>,
    /// File whose video size the window was last fitted to.
    fitted: Option<String>,
    title: String,
    margins: [f64; 4],
    next_poll: Instant,
    played: bool,
    pub quit_at_end: bool,
    pub shuffled: bool,
    pub stats: bool,
    pub redraws: u64,
}

impl App {
    pub fn new(player: Player, settings: Settings, settings_path: Option<PathBuf>, prober: Prober) -> App {
        App {
            player,
            settings,
            settings_path,
            view: View::default(),
            win: WindowInfo::default(),
            requests: Vec::new(),
            actions: Vec::new(),
            prober,
            probed: HashMap::new(),
            awake: keep_awake_worker(),
            awake_on: false,
            resume_point: None,
            fitted: None,
            title: String::new(),
            margins: [-1.0; 4],
            next_poll: Instant::now(),
            played: false,
            quit_at_end: false,
            shuffled: false,
            stats: false,
            redraws: 0,
        }
    }

    pub fn save_settings(&self) {
        if let Some(path) = &self.settings_path
            && let Err(e) = self.settings.save(path)
        {
            log::warn!("can't save settings to {}: {e}", path.display());
        }
    }

    pub fn osd(&mut self, text: impl Into<String>) {
        self.view.osd = Some((text.into(), Instant::now()));
    }

    /// Drains mpv's event queue. Returns whether anything visible changed.
    pub fn on_mpv_events(&mut self) -> bool {
        let mut redraw = false;
        while let Some(event) = self.player.mpv.next_event(0.0) {
            redraw |= match event {
                Event::PropertyChange { name, value } => self.on_property(&name, value),
                Event::GetPropertyReply { error, name, value } if error >= 0 => {
                    self.player.state.apply(&name, value);
                    self.view.shows_stats(&self.settings)
                }
                Event::Reply { id, error } => self.on_reply(id, error),
                Event::FileLoaded => self.on_file_loaded(),
                Event::EndFile { reason, error } => self.on_end_file(reason, error),
                Event::Log { prefix, level, text } => {
                    crate::logging::mpv_message(&prefix, &level, &text);
                    false
                }
                Event::Shutdown => {
                    self.requests.push(Request::Quit);
                    false
                }
                _ => false,
            };
        }
        let s = &self.player.state;
        let awake = s.is_playing() && s.has_video;
        if awake != self.awake_on {
            self.awake_on = awake;
            let _ = self.awake.send(awake);
        }
        let title = match (&s.title, s.idle) {
            (Some(t), false) => format!("{t} - VibeLC media player"),
            _ => "VibeLC media player".to_string(),
        };
        if title != self.title {
            self.title = title.clone();
            self.requests.push(Request::Title(title));
        }
        redraw
    }

    fn on_property(&mut self, name: &str, value: crate::mpv::Node) -> bool {
        let s = &mut self.player.state;
        if name == "time-pos" {
            // Redraw only when the shown second or the seek bar's pixel changes; video frames
            // redraw anyway.
            let width = self.view.seek_width_px;
            let mark =
                |t: Option<f64>, d: Option<f64>| (t.map(|t| t as i64), t.zip(d).map(|(t, d)| (t / d * width) as i64));
            let before = mark(s.time, s.duration);
            s.apply(name, value);
            if let (Some(path), Some(t)) = (&s.path, s.time) {
                self.resume_point = Some((path.clone(), t, s.duration));
            }
            return mark(s.time, s.duration) != before;
        }
        s.apply(name, value);
        match name {
            "volume" => self.settings.volume = s.volume,
            "mute" => self.settings.muted = s.muted,
            "idle-active" if s.idle && self.played && self.quit_at_end => self.requests.push(Request::Quit),
            "video-params" if self.settings.resize_to_video && self.fitted != s.path && s.video_size().is_some() => {
                self.fitted = s.path.clone();
                self.fit_window(1.0);
            }
            "playlist" => {
                for entry in s.playlist.items() {
                    if let Some(f) = entry.str_of("filename")
                        && !media::is_url(f)
                        && !self.probed.contains_key(f)
                    {
                        self.probed.insert(f.to_string(), probe::Info::default());
                        self.prober.probe(f.to_string());
                    }
                }
            }
            _ => {}
        }
        true
    }

    fn on_reply(&mut self, id: u64, error: i32) -> bool {
        let message = (error < 0).then(|| self.player.mpv.error_message(error));
        match (id, message) {
            (REPLY_SNAPSHOT, None) => self.osd("Snapshot saved"),
            (REPLY_SNAPSHOT, Some(e)) => self.view.error = Some((format!("Snapshot failed: {e}"), Instant::now())),
            (REPLY_SUBTITLE, Some(e)) => {
                self.view.error = Some((format!("Can't load the subtitle file: {e}"), Instant::now()))
            }
            (_, Some(e)) => log::warn!("mpv: {e}"),
            _ => return false,
        }
        true
    }

    fn on_file_loaded(&mut self) -> bool {
        self.played = true;
        self.view.resume = None;
        let Some(path) = self.player.state.path.clone() else {
            return true;
        };
        self.settings.add_recent(&path);
        if let Some(pos) = self.settings.position_for(&path) {
            match self.settings.resume {
                Resume::Always => self.player.command(&["seek", &pos.to_string(), "absolute"]),
                Resume::Ask => self.view.resume = Some((path, pos, Instant::now())),
                Resume::Never => {}
            }
        }
        true
    }

    fn on_end_file(&mut self, reason: EndReason, error: i32) -> bool {
        let started = self.resume_point.take();
        if let Some((path, t, d)) = &started {
            // A file played to its end starts from the beginning next time.
            let t = if reason == EndReason::Eof { 0.0 } else { *t };
            self.settings.remember_position(path, t, *d);
            self.save_settings();
        }
        if reason == EndReason::Error {
            let name = self.player.state.path.as_deref().map(media::display_name).unwrap_or_default();
            let text = format!("Can't play {name}: {}", self.player.mpv.error_message(error));
            log::warn!("{text}");
            // Once playback has started (without an audio device, say), the log entry is enough.
            if started.is_none() {
                self.view.error = Some((text, Instant::now()));
            }
        }
        true
    }

    fn poll_deadline(&self) -> Option<Instant> {
        let poll = self.view.shows_stats(&self.settings) || self.stats || self.player.state.network;
        (poll && !self.player.state.idle).then_some(self.next_poll)
    }

    /// Time of the next timer (OSD expiry, controls hiding, statistics poll), if any.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.poll_deadline().into_iter().chain(self.view.next_deadline(self.win.fullscreen)).min()
    }

    /// Runs due timers. Returns whether one changed what is shown.
    pub fn tick(&mut self, now: Instant) -> bool {
        if self.poll_deadline().is_some_and(|d| d <= now) {
            self.next_poll = now + POLL_INTERVAL;
            for name in crate::player::POLLED {
                let _ = self.player.mpv.get_property_async(name);
            }
            if self.stats {
                let n = |name| self.player.state.polled(name).as_i64().unwrap_or(0);
                log::info!(
                    "stats: {} redraws/s, dropped frames {} (output) {} (decoder), hwdec {}",
                    std::mem::take(&mut self.redraws),
                    n("frame-drop-count"),
                    n("decoder-frame-drop-count"),
                    self.player.state.hwdec.as_deref().unwrap_or("no"),
                );
            }
        }
        self.view.next_deadline(self.win.fullscreen).is_some_and(|d| d <= now)
    }

    /// Fits the picture between the menu bar, the controls and the playlist.
    pub fn sync_margins(&mut self) {
        let (w, h) = self.win.size;
        let Some(r) = self.view.video_rect else {
            return;
        };
        let k = self.win.scale;
        let m = [
            r.min.x as f64 * k / w,
            1.0 - r.max.x as f64 * k / w,
            r.min.y as f64 * k / h,
            1.0 - r.max.y as f64 * k / h,
        ]
        .map(|v| (v.clamp(0.0, 1.0) * 1e4).round() / 1e4);
        if m != self.margins {
            self.margins = m;
            for (name, v) in MARGINS.iter().zip(m) {
                self.player.set(name, v);
            }
        }
    }

    /// Resizes the window so the picture shows at `scale` times its size (shrunk to fit the screen).
    fn fit_window(&mut self, scale: f64) {
        let Some((vw, vh)) = self.player.state.video_size() else {
            return;
        };
        let Some(r) = self.view.video_rect else {
            return;
        };
        if self.win.fullscreen || self.win.maximized || r.width() < 1.0 {
            return;
        }
        let k = self.win.scale;
        let chrome = (self.win.size.0 - r.width() as f64 * k, self.win.size.1 - r.height() as f64 * k);
        let (mut w, mut h) = (vw * scale, vh * scale);
        if let Some((mw, mh)) = self.win.monitor {
            let fit = ((mw * 0.9 - chrome.0) / w).min((mh * 0.85 - chrome.1) / h).min(1.0);
            (w, h) = (w * fit, h * fit);
        }
        self.requests.push(Request::Size((w + chrome.0).round(), (h + chrome.1).round()));
    }

    /// Opens dropped, picked, forwarded or pasted items: folders are expanded, playlist files
    /// loaded as lists, and subtitle files added to the playing video.
    pub fn open(&mut self, items: Vec<OsString>, append: bool) {
        let mut load = Vec::new();
        for item in items {
            let item = media::normalize(&item);
            let path = Path::new(&item);
            if path.is_dir() {
                load.extend(media::expand_folder(path).into_iter().map(PathBuf::into_os_string));
            } else if media::is_subtitle_file(path) && !self.player.state.idle {
                self.player.command_id(REPLY_SUBTITLE, &[OsStr::new("sub-add"), &item]);
            } else {
                load.push(item);
            }
        }
        if !load.is_empty() {
            self.player.load(&load, append);
            self.shuffled &= append;
        }
    }

    /// Opens text from the clipboard: one location per line.
    pub fn open_text(&mut self, text: &str) {
        let items: Vec<OsString> = text.lines().map(str::trim).filter(|l| !l.is_empty()).map(Into::into).collect();
        if items.is_empty() {
            self.osd("The clipboard holds no location");
        } else {
            self.open(items, false);
        }
    }

    pub fn picked(&mut self, kind: Pick, paths: Vec<PathBuf>) {
        let items: Vec<OsString> = paths.into_iter().map(PathBuf::into_os_string).collect();
        match kind {
            Pick::Open | Pick::Folder => self.open(items, false),
            Pick::Add => self.open(items, true),
            Pick::Subtitle => {
                for item in items {
                    self.player.command_id(REPLY_SUBTITLE, &[OsStr::new("sub-add"), &item]);
                }
            }
            Pick::SavePlaylist => {
                let Some(path) = items.first() else { return };
                let entries: Vec<(String, Option<String>)> = self
                    .player
                    .state
                    .playlist
                    .items()
                    .iter()
                    .filter_map(|e| Some((e.str_of("filename")?.to_string(), e.str_of("title").map(str::to_owned))))
                    .collect();
                if let Err(e) = std::fs::write(path, media::to_m3u(&entries)) {
                    self.view.error = Some((format!("Can't save the playlist: {e}"), Instant::now()));
                }
            }
            Pick::SnapshotFolder => {
                if let Some(dir) = items.into_iter().next() {
                    self.player.set("screenshot-directory", dir.to_string_lossy().as_ref());
                    self.settings.screenshot_folder = Some(dir.into());
                }
            }
        }
    }

    /// Saves what should survive a restart. Called once, when the window closes.
    pub fn shutdown(&mut self) {
        let s = &self.player.state;
        if let (Some(path), Some(t)) = (&s.path, s.time) {
            self.settings.remember_position(path, t, s.duration);
        }
        if let Some(g) = self.win.geometry {
            self.settings.window = Some(g);
        }
        self.save_settings();
    }

    pub fn run(&mut self, action: Action) {
        use Action::*;
        let p = &self.player;
        let s = &p.state;
        match action {
            OpenFiles => self.requests.push(Request::Pick(Pick::Open)),
            AddFiles => self.requests.push(Request::Pick(Pick::Add)),
            OpenFolder => self.requests.push(Request::Pick(Pick::Folder)),
            AddSubtitle => self.requests.push(Request::Pick(Pick::Subtitle)),
            SavePlaylist => self.requests.push(Request::Pick(Pick::SavePlaylist)),
            OpenClipboard => self.requests.push(Request::ReadClipboard),
            OpenRecent(i) => {
                if let Some(r) = self.settings.recent.get(i).cloned() {
                    self.open(vec![r.into()], false);
                }
            }
            ClearRecent => self.settings.recent.clear(),
            QuitAtEnd => self.quit_at_end ^= true,
            Quit => self.requests.push(Request::Quit),
            PlayPause if s.idle && s.playlist.items().is_empty() => self.requests.push(Request::Pick(Pick::Open)),
            PlayPause => p.play_pause(),
            Stop => p.command(&["stop", "keep-playlist"]),
            Previous => p.command(&["playlist-prev"]),
            Next => p.command(&["playlist-next"]),
            Seek(secs) => {
                p.command(&["seek", &secs.to_string()]);
                if let Some(t) = s.time {
                    let t = (t + f64::from(secs)).max(0.0);
                    let t = s.duration.map_or(t, |d| t.min(d));
                    self.osd(format!("{} / {}", crate::format::time(Some(t)), crate::format::time(s.duration)));
                }
            }
            FrameStep => p.command(&["frame-step"]),
            AbLoop => {
                p.command(&["ab-loop"]);
                let text = match s.ab_loop {
                    (None, _) => "Loop start set",
                    (Some(_), None) => "A-B loop on",
                    _ => "A-B loop off",
                };
                self.osd(text);
            }
            PreviousChapter => p.command(&["add", "chapter", "-1"]),
            NextChapter => p.command(&["add", "chapter", "1"]),
            Chapter(i) => p.set("chapter", i as i64),
            SpeedStep(step) => self.set_speed(((s.speed * 10.0).round() + f64::from(step)) / 10.0),
            SetSpeed(percent) => self.set_speed(f64::from(percent) / 100.0),
            CycleLoop => {
                let mode = match self.settings.loop_mode {
                    LoopMode::Off => LoopMode::All,
                    LoopMode::All => LoopMode::One,
                    LoopMode::One => LoopMode::Off,
                };
                self.set_loop(mode);
            }
            Shuffle => {
                p.command(&[if self.shuffled { "playlist-unshuffle" } else { "playlist-shuffle" }]);
                self.shuffled ^= true;
                self.osd(if self.shuffled { "Shuffle on" } else { "Shuffle off" });
            }
            AudioTrack(id) => self.select_track("audio", id),
            VideoTrack(id) => self.select_track("video", id),
            SubTrack(id) => self.select_track("sub", id),
            CycleAudio => self.cycle_track("audio"),
            CycleSub => self.cycle_track("sub"),
            AudioDevice(i) => {
                if let Some(name) = s.audio_devices.items().get(i).and_then(|d| d.str_of("name")) {
                    p.set("audio-device", name);
                }
            }
            Stereo(mode) => {
                self.settings.stereo_mode = mode;
                self.player.apply_audio_filters(&self.settings);
            }
            Volume(step) => {
                let v = ((s.volume / 5.0).round() * 5.0 + 5.0 * f64::from(step)).clamp(0.0, 125.0);
                p.set("volume", v);
                self.osd(format!("Volume {v:.0}%"));
            }
            Mute => {
                p.set("mute", !s.muted);
                self.osd(if s.muted { "Sound on" } else { "Muted" });
            }
            AudioDelay(ms) => self.add_delay("audio-delay", s.audio_delay, ms, "Audio delay"),
            SubDelay(ms) => self.add_delay("sub-delay", s.sub_delay, ms, "Subtitle delay"),
            SubSize(percent) => {
                let v = ((s.sub_scale * 100.0).round() + f64::from(percent)).clamp(10.0, 500.0) / 100.0;
                p.set("sub-scale", v);
                self.settings.subtitle_scale = v;
                self.osd(format!("Subtitle size {:.0}%", v * 100.0));
            }
            Fullscreen => self.requests.push(Request::Fullscreen(!self.win.fullscreen)),
            LeaveFullscreen if self.win.fullscreen => self.requests.push(Request::Fullscreen(false)),
            LeaveFullscreen => {
                self.view.dialogs.pop();
            }
            OnTop => {
                self.view.on_top ^= true;
                self.requests.push(Request::OnTop(self.view.on_top));
            }
            Zoom(percent) => self.fit_window(f64::from(percent) / 100.0),
            Fill => p.set("panscan", if s.panscan > 0.0 { 0.0 } else { 1.0 }),
            Aspect(i) => self.set_aspect(i),
            CycleAspect => {
                let current = aspect_index(s.aspect);
                self.set_aspect((current + 1) % (ASPECTS.len() + 1));
            }
            Rotate(degrees) => p.set("video-rotate", i64::from(degrees)),
            Deinterlace => p.set("deinterlace", !s.deinterlace),
            Snapshot => p.command_id(REPLY_SNAPSHOT, &["screenshot"]),
            Show(dialog) => self.view.show(dialog, s),
            Playlist => self.settings.show_playlist ^= true,
            StatusBar => self.settings.show_status_bar ^= true,
            Minimal => {
                self.view.minimal ^= true;
                if self.view.minimal {
                    let key = crate::action::hint(Minimal).unwrap_or_default();
                    self.osd(format!("Press {key} to show the controls again"));
                }
            }
            Theme(theme) => self.settings.theme = theme,
            ShowTime => {
                let text = format!("{} / {}", crate::format::time(s.time), crate::format::time(s.duration));
                self.osd(text);
            }
            PlayIndex(i) => p.command(&["playlist-play-index", &i.to_string()]),
            RemoveIndex(i) => p.command(&["playlist-remove", &i.to_string()]),
            ClearPlaylist => {
                p.command(&["stop"]);
                self.shuffled = false;
            }
        }
    }

    pub fn set_loop(&mut self, mode: LoopMode) {
        self.settings.loop_mode = mode;
        self.player.set_loop(mode);
        self.osd(match mode {
            LoopMode::Off => "Loop off",
            LoopMode::All => "Loop all",
            LoopMode::One => "Loop one",
        });
    }

    fn set_speed(&mut self, speed: f64) {
        let speed = speed.clamp(0.1, 8.0);
        self.player.set("speed", speed);
        self.osd(format!("Speed {speed:.2}x"));
    }

    fn add_delay(&mut self, property: &str, current: f64, ms: i32, label: &str) {
        let v = ((current * 1000.0).round() + f64::from(ms)) / 1000.0;
        self.player.set(property, v);
        self.osd(format!("{label} {:.0} ms", v * 1000.0));
    }

    fn set_aspect(&mut self, i: usize) {
        let value = if i == 0 { "-1" } else { ASPECTS.get(i - 1).copied().unwrap_or("-1") };
        self.player.set("video-aspect-override", value);
        self.osd(format!("Aspect ratio: {}", if i == 0 { "default" } else { value }));
    }

    fn select_track(&mut self, kind: &str, id: i64) {
        let property = match kind {
            "audio" => "aid",
            "video" => "vid",
            _ => "sid",
        };
        if id == 0 {
            self.player.set(property, "no");
        } else {
            self.player.set(property, id);
        }
    }

    /// Selects the next track of a type; subtitles cycle through "off" too.
    fn cycle_track(&mut self, kind: &str) {
        let s = &self.player.state;
        let mut ids: Vec<i64> = s.tracks_of(kind).filter_map(|t| t.i64_of("id")).collect();
        if kind == "sub" {
            ids.push(0);
        }
        let Some(next) = ids
            .iter()
            .position(|id| Some(*id) == s.selected(kind).or((kind == "sub").then_some(0)))
            .map_or(ids.first().copied(), |i| ids.get((i + 1) % ids.len()).copied())
        else {
            return self.osd(if kind == "sub" { "No subtitles" } else { "No audio tracks" });
        };
        let label =
            s.tracks_of(kind).find(|t| t.i64_of("id") == Some(next)).map_or_else(|| "off".to_string(), track_label);
        self.select_track(kind, next);
        self.osd(format!("{}: {label}", if kind == "sub" { "Subtitles" } else { "Audio" }));
    }
}

/// Position of the current aspect override in the Aspect menu (0 = the file's own).
pub fn aspect_index(aspect: Option<f64>) -> usize {
    let ratio = |a: &str| a.split_once(':').and_then(|(w, h)| Some(w.parse::<f64>().ok()? / h.parse::<f64>().ok()?));
    aspect
        .and_then(|a| ASPECTS.iter().position(|x| ratio(x).is_some_and(|r| (r - a).abs() < 0.01)))
        .map_or(0, |i| i + 1)
}

/// Keeps the screen on while video plays. The D-Bus or Win32 calls run on their own thread.
fn keep_awake_worker() -> Sender<bool> {
    let (tx, rx) = std::sync::mpsc::channel::<bool>();
    let _ = std::thread::Builder::new().name("keep-awake".into()).spawn(move || {
        let mut guard = None;
        for on in rx {
            if !on {
                guard = None;
            } else if guard.is_none() {
                guard = keepawake::Builder::default()
                    .display(true)
                    .idle(true)
                    .reason("Playing video")
                    .app_name("VibeLC")
                    .app_reverse_domain("io.github.vibelc")
                    .create()
                    .map_err(|e| log::warn!("can't keep the screen on: {e}"))
                    .ok();
            }
        }
    });
    tx
}
