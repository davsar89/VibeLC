//! Media locations: files vs URLs, folder expansion, natural ordering and M3U output.

use std::cmp::Ordering;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// Extensions treated as playable when expanding folders (lower case).
pub const MEDIA_EXTENSIONS: &[&str] = &[
    "3g2", "3gp", "aac", "ac3", "aif", "aiff", "amr", "ape", "asf", "avi", "dts", "eac3", "f4v", "flac", "flv", "m1v",
    "m2t", "m2ts", "m2v", "m4a", "m4b", "m4v", "mka", "mkv", "mov", "mp2", "mp3", "mp4", "mpeg", "mpg", "mts", "mxf",
    "nut", "oga", "ogg", "ogm", "ogv", "opus", "qt", "ra", "rm", "rmvb", "spx", "ts", "vob", "wav", "weba", "webm",
    "wma", "wmv", "wv", "y4m",
];

/// Playlist files, handed to mpv's `loadlist`.
pub const PLAYLIST_EXTENSIONS: &[&str] = &["m3u", "m3u8", "pls"];

/// Subtitle files for "Add Subtitle File".
pub const SUBTITLE_EXTENSIONS: &[&str] = &["srt", "ass", "ssa", "vtt", "sub", "idx", "sup", "smi", "txt"];

/// How deep "Open Folder" descends, which also bounds symlink loops.
const MAX_FOLDER_DEPTH: usize = 8;

fn extension_in(path: &Path, list: &[&str]) -> bool {
    path.extension().and_then(OsStr::to_str).is_some_and(|e| list.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

pub fn is_media_file(path: &Path) -> bool {
    extension_in(path, MEDIA_EXTENSIONS)
}

pub fn is_playlist_file(path: &Path) -> bool {
    extension_in(path, PLAYLIST_EXTENSIONS)
}

pub fn is_subtitle_file(path: &Path) -> bool {
    extension_in(path, SUBTITLE_EXTENSIONS)
}

/// "scheme://..." with a plausible scheme (letters, digits, `+`, `-`, `.`), e.g. https, rtsp,
/// ytdl. A Windows path such as `C:\x` is not a URL.
pub fn is_url(text: &str) -> bool {
    let Some((scheme, rest)) = text.split_once("://") else {
        return false;
    };
    scheme.len() >= 2
        && !rest.is_empty()
        && scheme.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// What to show for a playlist entry without a title: the file name, or the URL as given.
pub fn display_name(location: &str) -> String {
    if is_url(location) {
        return location.to_string();
    }
    Path::new(location).file_name().map_or_else(|| location.to_string(), |n| n.to_string_lossy().into_owned())
}

/// Turns a dropped/opened item into what mpv should load: URLs unchanged, files made absolute
/// (so the playlist survives working-directory changes and M3U export stays meaningful).
pub fn normalize(item: &OsStr) -> OsString {
    if item.to_str().is_some_and(is_url) {
        return item.to_owned();
    }
    std::path::absolute(item).map_or_else(|_| item.to_owned(), PathBuf::into_os_string)
}

/// Playable files below `dir`, in natural order of their relative paths.
pub fn expand_folder(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect(dir, 0, &mut out);
    out.sort_by(|a, b| natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()));
    out
}

fn collect(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > MAX_FOLDER_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let hidden = path.file_name().and_then(OsStr::to_str).is_some_and(|n| n.starts_with('.'));
        if hidden {
            continue;
        }
        if path.is_dir() {
            collect(&path, depth + 1, out);
        } else if is_media_file(&path) {
            out.push(path);
        }
    }
}

/// Compares strings so that digit runs compare by value ("ep2" < "ep10") and letters without
/// case; case and leading zeros only break ties.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    let mut tie = Ordering::Equal;
    loop {
        let (x, y) = match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return tie,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => (x, y),
        };
        let ord = if x.is_ascii_digit() && y.is_ascii_digit() {
            let take = |it: &mut std::iter::Peekable<std::str::Chars<'_>>| {
                let mut s = String::new();
                while let Some(c) = it.next_if(char::is_ascii_digit) {
                    s.push(c);
                }
                s
            };
            let (na, nb) = (take(&mut a), take(&mut b));
            let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
            tie = tie.then(na.len().cmp(&nb.len()));
            ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb))
        } else {
            a.next();
            b.next();
            tie = tie.then(x.cmp(&y));
            x.to_lowercase().cmp(y.to_lowercase())
        };
        if ord != Ordering::Equal {
            return ord;
        }
    }
}

/// An extended M3U playlist (UTF-8). Titles are written only when known.
pub fn to_m3u(entries: &[(String, Option<String>)]) -> String {
    let mut out = String::from("#EXTM3U\n");
    for (location, title) in entries {
        if let Some(t) = title.as_deref().filter(|t| !t.is_empty()) {
            out.push_str("#EXTINF:-1,");
            out.push_str(&t.replace(['\r', '\n'], " "));
            out.push('\n');
        }
        out.push_str(location);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_urls() {
        for u in ["https://example.com/a.mp4", "rtsp://cam/stream", "ytdl://abc", "udp://@239.0.0.1:1234"] {
            assert!(is_url(u), "{u}");
        }
        for p in ["C:\\Videos\\a.mkv", "/home/x/a.mkv", "a.mkv", "://x", "1x://y", "http://"] {
            assert!(!is_url(p), "{p}");
        }
    }

    #[test]
    fn natural_order() {
        let mut v = vec!["Ep10.mkv", "ep2.mkv", "Ep1.mkv", "ep02.mkv", "a.mkv", "B.mkv"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["a.mkv", "B.mkv", "Ep1.mkv", "ep2.mkv", "ep02.mkv", "Ep10.mkv"]);
        assert_eq!(natural_cmp("x9", "x10"), Ordering::Less);
        assert_eq!(natural_cmp("x", "x"), Ordering::Equal);
    }

    #[test]
    fn m3u_keeps_non_ascii_names_and_urls() {
        let entries = vec![
            ("/v/Fjøsnissen – jul.mkv".to_string(), Some("Fjøsnissen".to_string())),
            ("https://example.com/live.m3u8".to_string(), None),
            ("/v/日本語.mp4".to_string(), Some("multi\nline".to_string())),
        ];
        assert_eq!(
            to_m3u(&entries),
            "#EXTM3U\n#EXTINF:-1,Fjøsnissen\n/v/Fjøsnissen – jul.mkv\nhttps://example.com/live.m3u8\n\
             #EXTINF:-1,multi line\n/v/日本語.mp4\n"
        );
    }

    #[test]
    fn folder_expansion_filters_and_orders() {
        let dir = std::env::temp_dir().join(format!("vibelc-folder-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        for f in ["ep10.mkv", "ep2.MP4", "notes.txt", ".hidden.mkv", "sub/extra.mp3", "cover.jpg"] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        let names: Vec<String> = expand_folder(&dir)
            .iter()
            .map(|p| p.strip_prefix(&dir).unwrap().to_string_lossy().replace('\\', "/"))
            .collect();
        assert_eq!(names, ["ep2.MP4", "ep10.mkv", "sub/extra.mp3"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn display_names() {
        assert_eq!(display_name("/a/b/movie.mkv"), "movie.mkv");
        assert_eq!(display_name("https://x.org/v"), "https://x.org/v");
    }
}
