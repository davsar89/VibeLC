//! Fills in playlist durations and titles from a second, headless mpv core on a worker thread.

use std::sync::Arc;
use std::sync::mpsc::{Sender, channel};
use std::time::{Duration, Instant};

use crate::mpv::{Api, Event, Mpv};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Info {
    pub duration: Option<f64>,
    pub title: Option<String>,
}

pub struct Prober(Sender<String>);

impl Prober {
    /// `done` runs on the worker thread with each result.
    pub fn start(api: Arc<Api>, done: impl Fn(String, Info) + Send + 'static) -> Prober {
        let (tx, rx) = channel::<String>();
        let spawned = std::thread::Builder::new().name("probe".into()).spawn(move || {
            // The core starts with the first request: most runs open one file and never need it.
            let Ok(first) = rx.recv() else { return };
            // No outputs, tracks or built-in scripts: mpv opens the file, reads its headers and waits.
            let mut options = vec![("vo", "null"), ("ao", "null"), ("track-auto-selection", "no"), ("sub-auto", "no")];
            options.extend(crate::player::UNUSED_SCRIPTS.map(|s| (s, "no")));
            let mpv = match Mpv::new(api, &options) {
                Ok(mpv) => mpv,
                Err(e) => return log::warn!("duration probe unavailable: {e}"),
            };
            for location in std::iter::once(first).chain(rx) {
                let info = probe(&mpv, &location);
                done(location, info);
            }
        });
        if let Err(e) = spawned {
            log::warn!("duration probe unavailable: {e}");
        }
        Prober(tx)
    }

    pub fn probe(&self, location: String) {
        let _ = self.0.send(location);
    }
}

fn wait_for(mpv: &Mpv, mut stop: impl FnMut(&Event) -> bool) -> Option<Event> {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Some(ev) = mpv.next_event(0.5).filter(&mut stop) {
            return Some(ev);
        }
    }
    None
}

fn probe(mpv: &Mpv, location: &str) -> Info {
    if mpv.command(&["loadfile", location]).is_err() {
        return Info::default();
    }
    let ended = |e: &Event| matches!(e, Event::EndFile { .. });
    if wait_for(mpv, |e| matches!(e, Event::FileLoaded) || ended(e)) != Some(Event::FileLoaded) {
        return Info::default();
    }
    let info = Info {
        duration: mpv.get_property("duration").ok().and_then(|n| n.as_f64()),
        title: mpv.get_property("metadata/by-key/title").ok().and_then(|n| n.as_str().map(str::to_owned)),
    };
    // Wait for the stop to finish, so its end event isn't mistaken for the next file's.
    if mpv.command(&["stop"]).is_ok() {
        wait_for(mpv, ended);
    }
    info
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_duration_and_title() {
        let Ok(api) = Api::load() else {
            return eprintln!("skipped, libmpv not available");
        };
        let (tx, rx) = channel();
        let prober = Prober::start(Arc::new(api), move |loc, info| tx.send((loc, info)).unwrap());
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/");
        for f in ["tone.ogg", "missing.mkv", "second.mp4"] {
            prober.probe(format!("{dir}{f}"));
        }
        let got: Vec<(String, Info)> = rx.iter().take(3).collect();
        let (tone, missing, second) = (&got[0].1, &got[1].1, &got[2].1);
        assert!((tone.duration.unwrap() - 2.0).abs() < 0.1);
        assert_eq!(tone.title.as_deref(), Some("VibeLC tone"));
        assert_eq!(missing, &Info::default());
        assert_eq!(second.title.as_deref(), Some("Second fixture"));
    }
}
