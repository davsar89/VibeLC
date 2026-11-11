//! Single instance: media opened while VibeLC runs goes to the running window, over a local
//! socket (a file in `$XDG_RUNTIME_DIR` on Linux, a named pipe on Windows).

use std::ffi::OsString;
use std::io::{Read, Write};

use interprocess::local_socket::{GenericNamespaced, ListenerOptions, Name, Stream, prelude::*};

fn name() -> std::io::Result<Name<'static>> {
    #[cfg(unix)]
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        return std::path::Path::new(&dir)
            .join("vibelc.sock")
            .to_fs_name::<interprocess::local_socket::GenericFilePath>()
            .map(Name::into_owned);
    }
    let user = std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_default();
    format!("vibelc-{user}").to_ns_name::<GenericNamespaced>().map(Name::into_owned)
}

/// Sends `items` to a running instance; false if there is none.
pub fn forward(items: &[OsString]) -> bool {
    let Ok(mut conn) = name().and_then(Stream::connect) else {
        return false;
    };
    let mut buf = Vec::new();
    for item in items {
        let bytes = to_bytes(item);
        buf.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        buf.extend_from_slice(&bytes);
    }
    conn.write_all(&buf).is_ok()
}

/// Accepts items from later instances on a background thread.
pub fn listen(on_items: impl Fn(Vec<OsString>) + Send + 'static) {
    // Overwriting only removes a stale socket file: `forward` found no live listener.
    let listener = match name().and_then(|n| ListenerOptions::new().name(n).try_overwrite(true).create_sync()) {
        Ok(l) => l,
        Err(e) => return log::warn!("single instance disabled: {e}"),
    };
    let spawned = std::thread::Builder::new().name("instance".into()).spawn(move || {
        for conn in listener.incoming().filter_map(Result::ok) {
            let mut data = Vec::new();
            if conn.take(1 << 24).read_to_end(&mut data).is_ok() {
                let items = decode(&data);
                if !items.is_empty() {
                    on_items(items);
                }
            }
        }
    });
    if let Err(e) = spawned {
        log::warn!("single instance disabled: {e}");
    }
}

fn decode(mut data: &[u8]) -> Vec<OsString> {
    let mut items = Vec::new();
    while let Some((len, rest)) = data.split_first_chunk::<4>() {
        let len = u32::from_le_bytes(*len) as usize;
        let Some((item, rest)) = rest.split_at_checked(len) else {
            break;
        };
        items.push(from_bytes(item.to_vec()));
        data = rest;
    }
    items
}

#[cfg(unix)]
fn to_bytes(s: &OsString) -> Vec<u8> {
    std::os::unix::ffi::OsStrExt::as_bytes(s.as_os_str()).to_vec()
}

#[cfg(unix)]
fn from_bytes(b: Vec<u8>) -> OsString {
    std::os::unix::ffi::OsStringExt::from_vec(b)
}

#[cfg(not(unix))]
fn to_bytes(s: &OsString) -> Vec<u8> {
    s.to_string_lossy().into_owned().into_bytes()
}

#[cfg(not(unix))]
fn from_bytes(b: Vec<u8>) -> OsString {
    String::from_utf8_lossy(&b).into_owned().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_round_trip_and_truncation_is_safe() {
        let items: Vec<OsString> = ["/v/a b.mkv", "https://x.org/s.m3u8", "/v/Fjøs.mkv", ""].map(Into::into).to_vec();
        let mut buf = Vec::new();
        for item in &items {
            let b = to_bytes(item);
            buf.extend_from_slice(&(b.len() as u32).to_le_bytes());
            buf.extend_from_slice(&b);
        }
        assert_eq!(decode(&buf), items);
        assert_eq!(decode(&buf[..buf.len() - 1]).len(), 3, "a cut-off item is dropped");
        assert!(decode(&[255, 255, 255, 255, 1]).is_empty());
    }
}
