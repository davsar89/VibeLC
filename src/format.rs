//! Time display and parsing.

/// "02:05" below an hour, "1:02:05" from an hour on; "--:--" for unknown or invalid times.
pub fn time(secs: Option<f64>) -> String {
    let Some(s) = secs.filter(|s| s.is_finite() && *s >= 0.0) else {
        return "--:--".into();
    };
    let total = s.floor() as u64;
    let (h, m, sec) = (total / 3600, (total / 60) % 60, total % 60);
    if h > 0 { format!("{h}:{m:02}:{sec:02}") } else { format!("{m:02}:{sec:02}") }
}

/// Parses "h:mm:ss", "mm:ss" or plain seconds, each part optionally fractional in the last field.
pub fn parse_time(text: &str) -> Option<f64> {
    let parts: Vec<&str> = text.trim().split(':').collect();
    if parts.is_empty() || parts.len() > 3 || parts.iter().any(|p| p.trim().is_empty()) {
        return None;
    }
    let mut total = 0.0;
    for (i, part) in parts.iter().enumerate() {
        let last = i + 1 == parts.len();
        let v: f64 = if last { part.trim().parse().ok()? } else { part.trim().parse::<u32>().ok()?.into() };
        if !v.is_finite() || v < 0.0 || (i > 0 && v >= 60.0) {
            return None;
        }
        total = total * 60.0 + v;
    }
    Some(total)
}

/// File size as "12.3 MB".
pub fn bytes(n: i64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n.max(0) as f64;
    let mut unit = 0;
    while v >= 1000.0 && unit + 1 < UNITS.len() {
        v /= 1000.0;
        unit += 1;
    }
    if unit == 0 { format!("{n} B") } else { format!("{v:.1} {}", UNITS[unit]) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_times() {
        assert_eq!(time(None), "--:--");
        assert_eq!(time(Some(f64::NAN)), "--:--");
        assert_eq!(time(Some(-1.0)), "--:--");
        assert_eq!(time(Some(0.0)), "00:00");
        assert_eq!(time(Some(59.99)), "00:59");
        assert_eq!(time(Some(125.0)), "02:05");
        assert_eq!(time(Some(3600.0)), "1:00:00");
        assert_eq!(time(Some(36_000.0 + 62.5)), "10:01:02");
    }

    #[test]
    fn parses_times() {
        assert_eq!(parse_time("90"), Some(90.0));
        assert_eq!(parse_time("1:30"), Some(90.0));
        assert_eq!(parse_time(" 1:02:03 "), Some(3723.0));
        assert_eq!(parse_time("0:01.5"), Some(1.5));
        assert_eq!(parse_time("1:60"), None);
        assert_eq!(parse_time("1::2"), None);
        assert_eq!(parse_time("1:2:3:4"), None);
        assert_eq!(parse_time("abc"), None);
        assert_eq!(parse_time("-5"), None);
        assert_eq!(parse_time(""), None);
    }

    #[test]
    fn round_trips() {
        for s in [0.0, 5.0, 59.0, 60.0, 3599.0, 3600.0, 86_399.0] {
            assert_eq!(parse_time(&time(Some(s))), Some(s));
        }
    }

    #[test]
    fn formats_sizes() {
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(73_065), "73.1 KB");
        assert_eq!(bytes(4_500_000_000), "4.5 GB");
    }
}
