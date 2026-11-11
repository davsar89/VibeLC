//! Persistent settings: `settings.toml` in the platform's config folder (`~/.config/vibelc`,
//! `%APPDATA%\VibeLC\config`). Missing keys take their defaults; a file that doesn't parse is
//! kept aside as `settings.toml.bad`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::audio_filters::{Equalizer, StereoMode};

const MAX_RECENT: usize = 10;
const MAX_POSITIONS: usize = 200;
/// Positions this close to the start or the end are not worth resuming.
const RESUME_MARGIN_SECS: f64 = 10.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

/// Video scaling quality, as explicit option lists so every mpv version behaves the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RenderProfile {
    #[default]
    Fast,
    Balanced,
    HighQuality,
}

impl RenderProfile {
    pub const ALL: [RenderProfile; 3] = [RenderProfile::Fast, RenderProfile::Balanced, RenderProfile::HighQuality];

    pub fn label(self) -> &'static str {
        match self {
            RenderProfile::Fast => "Fast (best for older computers)",
            RenderProfile::Balanced => "Balanced",
            RenderProfile::HighQuality => "High quality (needs a strong GPU)",
        }
    }

    pub fn mpv_options(self) -> [(&'static str, &'static str); 8] {
        let (scale, cscale, dscale, heavy) = match self {
            RenderProfile::Fast => ("bilinear", "bilinear", "bilinear", "no"),
            RenderProfile::Balanced => ("spline36", "bilinear", "mitchell", "no"),
            RenderProfile::HighQuality => ("ewa_lanczossharp", "ewa_lanczossharp", "mitchell", "yes"),
        };
        let fast = self == RenderProfile::Fast;
        [
            ("scale", scale),
            ("cscale", cscale),
            ("dscale", dscale),
            ("dither-depth", if fast { "no" } else { "auto" }),
            ("correct-downscaling", if fast { "no" } else { "yes" }),
            // With these off and bilinear scalers, mpv renders in one pass without intermediate
            // framebuffers ("dumb mode").
            ("linear-downscaling", if fast { "no" } else { "yes" }),
            ("sigmoid-upscaling", heavy),
            ("deband", heavy),
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Resume {
    #[default]
    Ask,
    Always,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LoopMode {
    #[default]
    Off,
    All,
    One,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindowGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedPosition {
    pub location: String,
    pub seconds: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub volume: f64,
    pub muted: bool,
    pub theme: Theme,
    pub window: Option<WindowGeometry>,
    pub hardware_decoding: bool,
    pub profile: RenderProfile,
    pub resize_to_video: bool,
    pub single_instance: bool,
    pub resume: Resume,
    pub show_playlist: bool,
    pub show_status_bar: bool,
    pub loop_mode: LoopMode,
    pub equalizer: Equalizer,
    pub stereo_mode: StereoMode,
    pub normalize_volume: bool,
    /// Preferred languages as comma-separated codes, e.g. "nor,eng".
    pub audio_languages: String,
    pub subtitle_languages: String,
    pub subtitle_scale: f64,
    pub screenshot_folder: Option<PathBuf>,
    pub recent: Vec<String>,
    pub positions: Vec<SavedPosition>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            volume: 100.0,
            muted: false,
            theme: Theme::System,
            window: None,
            hardware_decoding: true,
            profile: RenderProfile::Fast,
            resize_to_video: true,
            single_instance: true,
            resume: Resume::Ask,
            show_playlist: false,
            show_status_bar: false,
            loop_mode: LoopMode::Off,
            equalizer: Equalizer::default(),
            stereo_mode: StereoMode::Stereo,
            normalize_volume: false,
            audio_languages: String::new(),
            subtitle_languages: String::new(),
            subtitle_scale: 1.0,
            screenshot_folder: None,
            recent: Vec::new(),
            positions: Vec::new(),
        }
    }
}

/// Folder for settings and the log; `VIBELC_CONFIG_DIR` overrides it.
pub fn config_dir() -> Option<PathBuf> {
    std::env::var_os("VIBELC_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| directories::ProjectDirs::from("", "", "VibeLC").map(|d| d.config_dir().to_path_buf()))
}

impl Settings {
    /// mpv's `hwdec` mode: the graphics card's decoder where mpv knows it to be reliable (zero-copy
    /// when the OpenGL interop allows it, else copy-back), software otherwise.
    pub fn hwdec(&self) -> &'static str {
        if self.hardware_decoding { "auto-safe" } else { "no" }
    }

    pub fn load(path: &Path) -> Settings {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Settings::default();
        };
        toml::from_str(&text).unwrap_or_else(|e| {
            let bad = path.with_extension("toml.bad");
            log::warn!("{} is not valid ({e}); moved to {} and using defaults", path.display(), bad.display());
            let _ = std::fs::rename(path, bad);
            Settings::default()
        })
    }

    /// Writes a temporary file and renames it, so a crash never leaves a half-written file.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, toml::to_string(self).map_err(std::io::Error::other)?)?;
        std::fs::rename(&tmp, path)
    }

    pub fn add_recent(&mut self, location: &str) {
        self.recent.retain(|r| r != location);
        self.recent.insert(0, location.to_string());
        self.recent.truncate(MAX_RECENT);
    }

    /// Remembers where playback of `location` stopped, or forgets it if that's not worth resuming.
    pub fn remember_position(&mut self, location: &str, seconds: f64, duration: Option<f64>) {
        self.positions.retain(|p| p.location != location);
        if seconds > RESUME_MARGIN_SECS && duration.is_none_or(|d| seconds < d - RESUME_MARGIN_SECS) {
            self.positions.insert(0, SavedPosition { location: location.to_string(), seconds });
            self.positions.truncate(MAX_POSITIONS);
        }
    }

    pub fn position_for(&self, location: &str) -> Option<f64> {
        self.positions.iter().find(|p| p.location == location).map(|p| p.seconds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str, content: Option<&str>) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vibelc-settings-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.toml");
        if let Some(c) = content {
            std::fs::write(&path, c).unwrap();
        }
        path
    }

    #[test]
    fn round_trip_partial_and_corrupt_files() {
        let path = temp_file("roundtrip", None);
        assert_eq!(Settings::load(&path), Settings::default());
        let mut s = Settings { volume: 42.0, theme: Theme::Dark, ..Default::default() };
        s.add_recent("/v/a.mkv");
        s.remember_position("/v/a.mkv", 61.5, Some(3600.0));
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path), s);

        let path = temp_file("partial", Some("volume = 55.0\nsome_future_key = true\n"));
        let s = Settings::load(&path);
        assert_eq!((s.volume, s.profile), (55.0, RenderProfile::Fast));

        let path = temp_file("corrupt", Some("volume = [oops"));
        assert_eq!(Settings::load(&path), Settings::default());
        assert!(path.with_extension("toml.bad").exists() && !path.exists());
    }

    #[test]
    fn recents_are_unique_newest_first_and_capped() {
        let mut s = Settings::default();
        for i in 0..15 {
            s.add_recent(&format!("f{i}"));
        }
        s.add_recent("f10");
        assert_eq!(s.recent.len(), MAX_RECENT);
        assert_eq!(s.recent[0], "f10");
        assert_eq!(s.recent.iter().filter(|r| *r == "f10").count(), 1);
    }

    #[test]
    fn positions_only_when_worth_resuming() {
        let mut s = Settings::default();
        s.remember_position("a", 5.0, Some(100.0));
        assert_eq!(s.position_for("a"), None, "too close to the start");
        s.remember_position("a", 95.0, Some(100.0));
        assert_eq!(s.position_for("a"), None, "too close to the end");
        s.remember_position("a", 50.0, Some(100.0));
        assert_eq!(s.position_for("a"), Some(50.0));
        s.remember_position("b", 30.0, None);
        assert_eq!(s.position_for("b"), Some(30.0), "a stream without duration still counts");
        s.remember_position("a", 2.0, Some(100.0));
        assert_eq!(s.position_for("a"), None, "a later stop near the start forgets it");
        for i in 0..250 {
            s.remember_position(&format!("x{i}"), 60.0, None);
        }
        assert_eq!(s.positions.len(), MAX_POSITIONS);
        assert_eq!((s.position_for("x249"), s.position_for("x0")), (Some(60.0), None), "oldest dropped first");
    }
}
