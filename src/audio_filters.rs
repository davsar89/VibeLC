//! The one place that builds mpv's audio filter chain (`af`): stereo mode, equalizer with
//! preamp, and volume normalization, as a single libavfilter graph.

use serde::{Deserialize, Serialize};

/// Centre frequencies of the 10 octave bands (ISO standard bands).
pub const EQ_BANDS_HZ: [u32; 10] = [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];
pub const EQ_RANGE_DB: f32 = 20.0;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Equalizer {
    pub enabled: bool,
    pub preamp_db: f32,
    pub gains_db: [f32; 10],
}

impl Default for Equalizer {
    fn default() -> Self {
        Equalizer { enabled: false, preamp_db: 0.0, gains_db: [0.0; 10] }
    }
}

pub struct EqPreset {
    pub name: &'static str,
    pub preamp_db: f32,
    pub gains_db: [f32; 10],
}

/// VibeLC's own presets. A negative preamp leaves headroom where bands are boosted.
pub const EQ_PRESETS: &[EqPreset] = &[
    EqPreset { name: "Flat", preamp_db: 0.0, gains_db: [0.0; 10] },
    EqPreset { name: "Bass boost", preamp_db: -5.0, gains_db: [6.0, 5.0, 4.0, 2.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0] },
    EqPreset { name: "Treble boost", preamp_db: -5.0, gains_db: [0.0, 0.0, 0.0, 0.0, 0.0, 0.5, 2.0, 4.0, 5.0, 6.0] },
    EqPreset {
        name: "Bass and treble",
        preamp_db: -4.0,
        gains_db: [5.0, 4.0, 2.0, 0.0, -1.0, -1.0, 0.0, 2.0, 4.0, 5.0],
    },
    EqPreset {
        name: "Clear voices",
        preamp_db: -3.0,
        gains_db: [-3.0, -2.0, -1.0, 1.0, 3.0, 4.0, 3.5, 1.5, 0.0, -1.0],
    },
    EqPreset { name: "Warm", preamp_db: -2.0, gains_db: [2.5, 2.0, 1.5, 0.5, 0.0, -0.5, -1.0, -1.5, -2.0, -3.0] },
    EqPreset {
        name: "Small speakers",
        preamp_db: -3.0,
        gains_db: [-5.0, -3.0, 0.0, 2.0, 3.0, 3.0, 2.0, 1.0, 0.5, 0.0],
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum StereoMode {
    #[default]
    Stereo,
    Mono,
    Left,
    Right,
    Reverse,
}

impl StereoMode {
    pub const ALL: [StereoMode; 5] =
        [StereoMode::Stereo, StereoMode::Mono, StereoMode::Left, StereoMode::Right, StereoMode::Reverse];

    pub fn label(self) -> &'static str {
        match self {
            StereoMode::Stereo => "Stereo",
            StereoMode::Mono => "Mono",
            StereoMode::Left => "Left channel only",
            StereoMode::Right => "Right channel only",
            StereoMode::Reverse => "Swap left and right",
        }
    }

    /// `pan` routing after downmixing to stereo (so mono and 5.1 sources work too).
    fn pan(self) -> Option<&'static str> {
        match self {
            StereoMode::Stereo => None,
            StereoMode::Mono => Some("pan=stereo|c0=0.5*c0+0.5*c1|c1=0.5*c0+0.5*c1"),
            StereoMode::Left => Some("pan=stereo|c0=c0|c1=c0"),
            StereoMode::Right => Some("pan=stereo|c0=c1|c1=c1"),
            StereoMode::Reverse => Some("pan=stereo|c0=c1|c1=c0"),
        }
    }
}

/// Value for mpv's `af` property; empty clears all filters.
pub fn af_chain(eq: &Equalizer, stereo: StereoMode, normalize: bool) -> String {
    let mut graph: Vec<String> = Vec::new();
    if let Some(pan) = stereo.pan() {
        graph.push("aformat=channel_layouts=stereo".into());
        graph.push(pan.into());
    }
    if eq.enabled {
        let clamp = |g: f32| g.clamp(-EQ_RANGE_DB, EQ_RANGE_DB);
        if clamp(eq.preamp_db).abs() >= 0.05 {
            graph.push(format!("volume={:.1}dB", clamp(eq.preamp_db)));
        }
        for (hz, gain) in EQ_BANDS_HZ.iter().zip(eq.gains_db) {
            if clamp(gain).abs() >= 0.05 {
                // One-octave-wide peaking filter per band.
                graph.push(format!("equalizer=f={hz}:t=o:w=1:g={:.1}", clamp(gain)));
            }
        }
    }
    if normalize {
        // Short frames and window keep the look-ahead (and start-up delay) near half a second.
        graph.push("dynaudnorm=f=100:g=9:p=0.5".into());
    }
    if graph.is_empty() { String::new() } else { format!("lavfi=[{}]", graph.join(",")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_enabled_clears_the_chain() {
        assert_eq!(af_chain(&Equalizer::default(), StereoMode::Stereo, false), "");
        let flat = Equalizer { enabled: true, ..Default::default() };
        assert_eq!(af_chain(&flat, StereoMode::Stereo, false), "");
    }

    #[test]
    fn builds_one_lavfi_graph() {
        let eq = Equalizer { enabled: true, preamp_db: -5.0, gains_db: EQ_PRESETS[1].gains_db };
        let af = af_chain(&eq, StereoMode::Reverse, true);
        assert_eq!(
            af,
            "lavfi=[aformat=channel_layouts=stereo,pan=stereo|c0=c1|c1=c0,volume=-5.0dB,\
             equalizer=f=31:t=o:w=1:g=6.0,equalizer=f=62:t=o:w=1:g=5.0,equalizer=f=125:t=o:w=1:g=4.0,\
             equalizer=f=250:t=o:w=1:g=2.0,equalizer=f=500:t=o:w=1:g=0.5,dynaudnorm=f=100:g=9:p=0.5]"
        );
    }

    #[test]
    fn gains_are_clamped() {
        let eq = Equalizer { enabled: true, preamp_db: 99.0, gains_db: [-99.0; 10] };
        let af = af_chain(&eq, StereoMode::Stereo, false);
        assert!(af.contains("volume=20.0dB"));
        assert!(af.contains("g=-20.0"));
        assert!(!af.contains("99"));
    }

    #[test]
    fn presets_stay_in_range() {
        for p in EQ_PRESETS {
            assert!(p.gains_db.iter().chain([&p.preamp_db]).all(|g| g.abs() <= EQ_RANGE_DB), "{}", p.name);
        }
    }
}
