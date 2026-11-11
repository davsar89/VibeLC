//! VibeLC's side of the mpv core. [`State`] is filled only from mpv's property notifications;
//! mpv owns playback and the playlist, so nothing here mirrors them.

use std::ffi::OsStr;
use std::sync::Arc;

use crate::audio_filters::af_chain;
use crate::mpv::{Api, Error, Mpv, Node, Value};
use crate::settings::{LoopMode, Settings};

/// Properties shown in the UI; mpv reports every change.
const OBSERVED: &[&str] = &[
    "path",
    "media-title",
    "idle-active",
    "pause",
    "time-pos",
    "duration",
    "volume",
    "mute",
    "speed",
    "track-list",
    "chapter-list",
    "chapter",
    "playlist",
    "playlist-pos",
    "hwdec-current",
    "video-params",
    "vo-configured",
    "seekable",
    "paused-for-cache",
    "cache-buffering-state",
    "demuxer-via-network",
    "ab-loop-a",
    "ab-loop-b",
    "audio-delay",
    "sub-delay",
    "sub-scale",
    "video-aspect-override",
    "video-rotate",
    "panscan",
    "deinterlace",
    "audio-device",
    "audio-device-list",
    "metadata",
    "file-format",
    "file-size",
    "brightness",
    "contrast",
    "saturation",
    "gamma",
    "hue",
    "loop-file",
    "loop-playlist",
    "mpv-version",
];

/// Properties that change with every frame; read once a second while something shows them.
pub const POLLED: &[&str] = &[
    "estimated-vf-fps",
    "frame-drop-count",
    "decoder-frame-drop-count",
    "video-bitrate",
    "audio-bitrate",
    "avsync",
    "demuxer-cache-state",
];

/// Aspect ratios offered besides the file's own.
pub const ASPECTS: &[&str] = &["16:9", "4:3", "1:1", "16:10", "2.21:1", "2.35:1", "2.39:1", "5:4"];

#[derive(Default)]
pub struct State {
    pub path: Option<String>,
    pub title: Option<String>,
    pub idle: bool,
    pub paused: bool,
    pub time: Option<f64>,
    pub duration: Option<f64>,
    pub volume: f64,
    pub muted: bool,
    pub speed: f64,
    pub tracks: Node,
    pub chapters: Node,
    pub chapter: Option<i64>,
    pub playlist: Node,
    pub playlist_pos: Option<usize>,
    pub hwdec: Option<String>,
    pub video_params: Node,
    pub has_video: bool,
    pub seekable: bool,
    pub buffering: Option<i64>,
    pub network: bool,
    pub ab_loop: (Option<f64>, Option<f64>),
    pub audio_delay: f64,
    pub sub_delay: f64,
    pub sub_scale: f64,
    pub aspect: Option<f64>,
    pub rotate: i64,
    pub panscan: f64,
    pub deinterlace: bool,
    pub audio_device: String,
    pub audio_devices: Node,
    pub metadata: Node,
    pub file_format: Option<String>,
    pub file_size: Option<i64>,
    /// brightness, contrast, saturation, gamma, hue (-100..100).
    pub picture: [i64; 5],
    pub loop_file: bool,
    pub loop_playlist: bool,
    pub mpv_version: String,
    /// Latest values of [`POLLED`], in that order.
    pub polled: [Node; POLLED.len()],
}

pub const PICTURE: [&str; 5] = ["brightness", "contrast", "saturation", "gamma", "hue"];

impl State {
    pub fn apply(&mut self, name: &str, v: Node) {
        let string = |v: &Node| v.as_str().filter(|s| !s.is_empty()).map(str::to_owned);
        match name {
            "path" => self.path = string(&v),
            "media-title" => self.title = string(&v),
            "idle-active" => self.idle = v.as_bool().unwrap_or(false),
            "pause" => self.paused = v.as_bool().unwrap_or(false),
            "time-pos" => self.time = v.as_f64(),
            "duration" => self.duration = v.as_f64(),
            "volume" => self.volume = v.as_f64().unwrap_or(100.0),
            "mute" => self.muted = v.as_bool().unwrap_or(false),
            "speed" => self.speed = v.as_f64().unwrap_or(1.0),
            "track-list" => self.tracks = v,
            "chapter-list" => self.chapters = v,
            "chapter" => self.chapter = v.as_i64().filter(|c| *c >= 0),
            "playlist" => self.playlist = v,
            "playlist-pos" => self.playlist_pos = v.as_i64().and_then(|p| usize::try_from(p).ok()),
            "hwdec-current" => self.hwdec = string(&v).filter(|h| h != "no"),
            "video-params" => self.video_params = v,
            "vo-configured" => self.has_video = v.as_bool().unwrap_or(false),
            "seekable" => self.seekable = v.as_bool().unwrap_or(false),
            "paused-for-cache" if v.as_bool() != Some(true) => self.buffering = None,
            "cache-buffering-state" => self.buffering = v.as_i64(),
            "demuxer-via-network" => self.network = v.as_bool().unwrap_or(false),
            "ab-loop-a" => self.ab_loop.0 = v.as_f64(),
            "ab-loop-b" => self.ab_loop.1 = v.as_f64(),
            "audio-delay" => self.audio_delay = v.as_f64().unwrap_or(0.0),
            "sub-delay" => self.sub_delay = v.as_f64().unwrap_or(0.0),
            "sub-scale" => self.sub_scale = v.as_f64().unwrap_or(1.0),
            "video-aspect-override" => self.aspect = v.as_f64().filter(|a| *a > 0.0),
            "video-rotate" => self.rotate = v.as_i64().unwrap_or(0),
            "panscan" => self.panscan = v.as_f64().unwrap_or(0.0),
            "deinterlace" => self.deinterlace = v.as_bool().unwrap_or(false),
            "audio-device" => self.audio_device = string(&v).unwrap_or_default(),
            "audio-device-list" => self.audio_devices = v,
            "metadata" => self.metadata = v,
            "file-format" => self.file_format = string(&v),
            "file-size" => self.file_size = v.as_i64(),
            // "no" is reported as false, "inf" as a string and a count as an integer.
            "loop-file" => self.loop_file = !matches!(v, Node::Flag(false) | Node::None),
            "loop-playlist" => self.loop_playlist = !matches!(v, Node::Flag(false) | Node::None),
            "mpv-version" => self.mpv_version = string(&v).unwrap_or_default(),
            _ => {
                if let Some(i) = PICTURE.iter().position(|p| *p == name) {
                    self.picture[i] = v.as_i64().unwrap_or(0);
                } else if let Some(i) = POLLED.iter().position(|p| *p == name) {
                    self.polled[i] = v;
                }
            }
        }
    }

    pub fn polled(&self, name: &str) -> &Node {
        POLLED.iter().position(|p| *p == name).map_or(&Node::None, |i| &self.polled[i])
    }

    /// Tracks of one type ("video", "audio" or "sub").
    pub fn tracks_of<'a>(&'a self, kind: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.tracks.items().iter().filter(move |t| t.str_of("type") == Some(kind))
    }

    /// Id of the selected track of a type.
    pub fn selected(&self, kind: &str) -> Option<i64> {
        self.tracks_of(kind).find(|t| t.bool_of("selected") == Some(true)).and_then(|t| t.i64_of("id"))
    }

    /// The picture's display size, after aspect correction and rotation.
    pub fn video_size(&self) -> Option<(f64, f64)> {
        let (w, h) = (self.video_params.f64_of("dw")?, self.video_params.f64_of("dh")?);
        let rotated = self.video_params.i64_of("rotate").unwrap_or(0) % 180 != 0;
        (w > 0.0 && h > 0.0).then_some(if rotated { (h, w) } else { (w, h) })
    }

    pub fn is_playing(&self) -> bool {
        !self.idle && !self.paused && self.path.is_some()
    }
}

/// "English tone [eng]" or "Track 2 [nor]", for menus.
pub fn track_label(t: &Node) -> String {
    let mut s = t.str_of("title").map_or_else(|| format!("Track {}", t.i64_of("id").unwrap_or(0)), str::to_owned);
    if let Some(lang) = t.str_of("lang").filter(|l| *l != "und") {
        s.push_str(&format!(" [{lang}]"));
    }
    if t.bool_of("external") == Some(true) {
        s.push_str(" (external)");
    }
    s
}

/// Reply ids for commands whose failure the user should see.
pub const REPLY_SNAPSHOT: u64 = 1;
pub const REPLY_SUBTITLE: u64 = 2;

pub struct Player {
    pub mpv: Mpv,
    pub state: State,
}

impl Player {
    pub fn new(api: Arc<Api>, settings: &Settings, hwdec: &str) -> Result<Player, Error> {
        let options = core_options(settings, hwdec);
        let options: Vec<(&str, &str)> = options.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let mpv = Mpv::new(api, &options)?;
        for name in OBSERVED {
            mpv.observe(name)?;
        }
        Ok(Player { mpv, state: State { volume: settings.volume, speed: 1.0, idle: true, ..Default::default() } })
    }

    /// Queues a command; failures are logged, or reported through `Event::Reply` for `id` > 0.
    pub fn command_id(&self, id: u64, args: &[impl AsRef<OsStr>]) {
        if let Err(e) = self.mpv.command_async(id, args) {
            log::warn!("mpv command {:?}: {e}", args.first().map(|a| a.as_ref().to_string_lossy()));
        }
    }

    pub fn command(&self, args: &[impl AsRef<OsStr>]) {
        self.command_id(0, args);
    }

    pub fn set<'a>(&self, name: &str, value: impl Into<Value<'a>>) {
        if let Err(e) = self.mpv.set_property_async(0, name, value) {
            log::warn!("mpv property {name}: {e}");
        }
    }

    /// Loads media (files, folders already expanded, URLs or playlist files), replacing the
    /// playlist or appending to it.
    pub fn load(&self, items: &[impl AsRef<OsStr>], append: bool) {
        for (i, item) in items.iter().enumerate() {
            let item = item.as_ref();
            let cmd = if crate::media::is_playlist_file(std::path::Path::new(item)) { "loadlist" } else { "loadfile" };
            let mode = if append || i > 0 { "append-play" } else { "replace" };
            self.command(&[OsStr::new(cmd), item, OsStr::new(mode)]);
        }
    }

    pub fn play_pause(&self) {
        let s = &self.state;
        if s.idle {
            if !s.playlist.items().is_empty() {
                let index = s.playlist.items().iter().position(|e| e.bool_of("current") == Some(true)).unwrap_or(0);
                self.command(&["playlist-play-index", &index.to_string()]);
            }
        } else {
            self.set("pause", !s.paused);
        }
    }

    pub fn set_loop(&self, mode: LoopMode) {
        self.set("loop-file", if mode == LoopMode::One { "inf" } else { "no" });
        self.set("loop-playlist", if mode == LoopMode::All { "inf" } else { "no" });
    }

    pub fn apply_audio_filters(&self, settings: &Settings) {
        self.set("af", af_chain(&settings.equalizer, settings.stereo_mode, settings.normalize_volume).as_str());
    }

    pub fn apply_profile(&self, settings: &Settings) {
        for (name, value) in settings.profile.mpv_options() {
            self.set(name, value);
        }
    }
}

/// mpv's built-in scripts each start a Lua VM and a thread; VibeLC draws its own interface and
/// uses none of them (ytdl, for URLs, stays). Options are set to "no".
pub const UNUSED_SCRIPTS: [&str; 7] = [
    "load-stats-overlay",
    "load-console",
    "load-select",
    "load-commands",
    "load-context-menu",
    "load-positioning",
    "load-auto-profiles",
];

/// mpv options at start-up. libmpv already defaults to no config files, no terminal, no
/// built-in key bindings and idle mode.
fn core_options(s: &Settings, hwdec: &str) -> Vec<(&'static str, String)> {
    let yes_no = |b: bool| if b { "yes" } else { "no" }.to_string();
    let screenshots = s
        .screenshot_folder
        .clone()
        .or_else(|| directories::UserDirs::new().and_then(|d| d.picture_dir().map(|p| p.to_path_buf())))
        .map(|p| p.to_string_lossy().into_owned());
    let mut o = vec![
        ("vo", "libmpv".to_string()),
        ("hwdec", hwdec.to_string()),
        // mpv's list plus MPEG-2 and WMV 9 (HD TV recordings, camcorders, DVDs), as VLC does and
        // mpv did. MPEG-4 Part 2 stays in software: cheap there, and badly handled by GPUs. The
        // rest of mpv's list only has Vulkan or macOS decoders, which VibeLC's OpenGL output can't use.
        ("hwdec-codecs", "h264,hevc,vp8,vp9,av1,vc1,wmv3,mpeg2video".to_string()),
        // Frames decoded in software (or copied back from the GPU) go up through pixel buffers,
        // as VLC does: one plain copy, then the GPU's, instead of the driver's CPU upload.
        ("opengl-pbo", "yes".to_string()),
        // Software decoding is lighter: FFmpeg's non-bit-exact shortcuts, and no deblocking of the
        // frames no other frame refers to (H.264, HEVC). Not framedrop=decoder: once behind, it
        // dropped over a thousand frames of a 4K clip that plays smoothly without it.
        ("vd-lavc-fast", "yes".to_string()),
        ("vd-lavc-skiploopfilter", "nonref".to_string()),
        // Render calls return at once and frames are signalled when due, not 50 ms early.
        ("video-timing-offset", "0".to_string()),
        // VibeLC draws its own OSD; subtitles stay inside the picture, not under the controls.
        ("osd-level", "0".to_string()),
        ("sub-use-margins", "no".to_string()),
        ("sub-auto", "fuzzy".to_string()),
        ("ytdl", "yes".to_string()),
        ("reset-on-next-file", "pause".to_string()),
        ("volume-max", "125".to_string()),
        ("volume", s.volume.to_string()),
        ("mute", yes_no(s.muted)),
        ("sub-scale", s.subtitle_scale.to_string()),
        ("alang", s.audio_languages.clone()),
        ("slang", s.subtitle_languages.clone()),
        ("af", af_chain(&s.equalizer, s.stereo_mode, s.normalize_volume)),
        ("loop-file", (if s.loop_mode == LoopMode::One { "inf" } else { "no" }).to_string()),
        ("loop-playlist", (if s.loop_mode == LoopMode::All { "inf" } else { "no" }).to_string()),
        ("screenshot-template", "%F-%wH.%wM.%wS".to_string()),
        ("audio-client-name", "VibeLC".to_string()),
    ];
    if let Some(dir) = screenshots {
        o.push(("screenshot-directory", dir));
    }
    o.extend(UNUSED_SCRIPTS.map(|s| (s, "no".to_string())));
    // Compiled shaders are kept between runs (with no config, mpv would keep none): the Direct3D
    // renderer otherwise compiles them all again at each start.
    if let Some(dir) = crate::settings::config_dir() {
        o.push(("gpu-shader-cache-dir", dir.join("shader-cache").to_string_lossy().into_owned()));
    }
    // PulseAudio (and PipeWire's Pulse server) then refills twice as rarely as with mpv's 100 ms,
    // which halves the audio threads' CPU time. Volume and equalizer changes lag by up to 0.2 s.
    #[cfg(target_os = "linux")]
    o.push(("pulse-buffer", "200".to_string()));
    o.extend(s.profile.mpv_options().map(|(k, v)| (k, v.to_string())));
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mpv::Event;
    use std::time::{Duration, Instant};

    fn fixture(name: &str) -> String {
        format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
    }

    /// A headless player, or `None` (test skipped) when libmpv isn't installed, unless
    /// `VIBELC_REQUIRE_LIBMPV` is set.
    fn headless() -> Option<Player> {
        let api = match Api::load() {
            Ok(api) => Arc::new(api),
            Err(e) if std::env::var_os("VIBELC_REQUIRE_LIBMPV").is_none() => {
                eprintln!("skipped, libmpv not available: {e}");
                return None;
            }
            Err(e) => panic!("{e}"),
        };
        let mpv = Mpv::new(api, &[("vo", "null"), ("ao", "null"), ("pause", "yes")]).unwrap();
        for name in OBSERVED {
            mpv.observe(name).unwrap();
        }
        Some(Player { mpv, state: State { idle: true, ..Default::default() } })
    }

    /// Processes events until `done` holds for the state, failing after 10 s.
    fn wait(p: &mut Player, what: &str, done: impl Fn(&State) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done(&p.state) {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            match p.mpv.next_event(0.1) {
                Some(Event::PropertyChange { name, value }) => p.state.apply(&name, value),
                Some(Event::Reply { error, .. }) => {
                    assert!(error >= 0, "{}", p.mpv.error_message(error))
                }
                _ => {}
            }
        }
    }

    #[test]
    fn plays_fixture_with_tracks_chapters_and_playlist() {
        let Some(mut p) = headless() else { return };
        p.load(&[fixture("av.mkv"), fixture("second.mp4")], false);
        wait(&mut p, "first file", |s| {
            s.duration.is_some()
                && s.tracks_of("sub").count() == 1
                && !s.chapters.items().is_empty()
                && s.video_size().is_some()
        });
        let s = &p.state;
        assert_eq!(s.title.as_deref(), Some("VibeLC fixture"));
        assert!((s.duration.unwrap() - 3.0).abs() < 0.1);
        let audio: Vec<String> = s.tracks_of("audio").map(track_label).collect();
        assert_eq!(audio, ["English tone [eng]", "Norwegian tone [nor]"]);
        assert_eq!(s.chapters.items().len(), 2);
        assert_eq!(s.video_size(), Some((320.0, 180.0)));

        p.command(&["set", "aid", "2"]);
        wait(&mut p, "audio track 2", |s| s.selected("audio") == Some(2));
        p.command(&["sub-add", &fixture("extra.srt")]);
        wait(&mut p, "external subtitle", |s| s.tracks_of("sub").count() == 2 && s.selected("sub") == Some(2));

        // Stop keeps the playlist; play resumes from the current entry.
        p.command(&["playlist-next"]);
        wait(&mut p, "second file", |s| s.title.as_deref() == Some("Second fixture"));
        p.command(&["stop", "keep-playlist"]);
        wait(&mut p, "idle", |s| s.idle && s.playlist.items().len() == 2);
        p.play_pause();
        wait(&mut p, "playing again", |s| !s.idle && s.title.as_deref() == Some("Second fixture"));
    }

    #[test]
    fn every_audio_filter_setting_is_accepted() {
        use crate::audio_filters::{EQ_PRESETS, Equalizer, StereoMode};
        let Some(mut p) = headless() else { return };
        p.load(&[fixture("tone.ogg")], false);
        wait(&mut p, "audio file", |s| s.duration.is_some());
        for (i, mode) in StereoMode::ALL.iter().enumerate() {
            let preset = &EQ_PRESETS[i % EQ_PRESETS.len()];
            let eq = Equalizer { enabled: true, preamp_db: preset.preamp_db, gains_db: preset.gains_db };
            let chain = af_chain(&eq, *mode, i % 2 == 0);
            // A synchronous set fails if mpv can't build the filter graph.
            p.mpv.command(&["set", "af", &chain]).unwrap_or_else(|e| panic!("{chain}: {e}"));
        }
    }
}
