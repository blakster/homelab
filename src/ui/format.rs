//! Number, time and width formatting. Numbers are fixed-width and right-aligned
//! so ticks never shift a column (the terminal form of `tabular-nums`).

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::model::{Latency, Millis};

pub fn pct(v: f32) -> String {
    if v.is_finite() {
        format!("{:>3.0}%", v.clamp(0.0, 100.0))
    } else {
        "  –%".into()
    }
}

pub fn gb(bytes: u64) -> String {
    let g = bytes as f64 / (1u64 << 30) as f64;
    if g >= 100.0 || (g - g.round()).abs() < 0.05 {
        format!("{g:.0}")
    } else {
        format!("{g:.1}")
    }
}

/// `2d 4h`, `12m`, `59s`. Two units at most.
pub fn dur(ms: Millis) -> String {
    let s = (ms.max(0) / 1000) as u64;
    let (d, h, m) = (s / 86_400, s % 86_400 / 3_600, s % 3_600 / 60);
    match () {
        _ if d > 0 && h > 0 && d < 10 => format!("{d}d {h}h"),
        _ if d > 0 => format!("{d}d"),
        _ if h > 0 => format!("{h}h {m}m"),
        _ if m > 0 => format!("{m}m"),
        _ => format!("{s}s"),
    }
}

pub fn ago(then: Millis, now: Millis) -> String {
    if then >= now {
        "just now".into()
    } else {
        format!("{} ago", dur(now - then))
    }
}

pub fn latency(l: &Latency) -> String {
    match l.ms() {
        Some(ms) => format!("{:>4.0} ms", ms),
        None => "   – ms".into(),
    }
}

/// Truncate to `width` terminal cells, ending in `…` when cut.
pub fn fit(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut w = 0;
    for ch in s.chars() {
        let cw = ch.width().unwrap_or(0);
        if w + cw + 1 > width {
            break;
        }
        out.push(ch);
        w += cw;
    }
    out.push('…');
    out
}

/// Pad (or truncate) to exactly `width` cells.
pub fn pad(s: &str, width: usize) -> String {
    let f = fit(s, width);
    let w = f.width();
    format!("{f}{}", " ".repeat(width.saturating_sub(w)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(dur(59_000), "59s");
        assert_eq!(dur(12 * 60_000), "12m");
        assert_eq!(dur((2 * 86_400 + 4 * 3_600 + 600) * 1000), "2d 4h");
        assert_eq!(dur(400 * 86_400_000), "400d");
        assert_eq!(dur(-5), "0s");
        assert_eq!(ago(10, 5), "just now");
    }

    #[test]
    fn fit_respects_wide_chars() {
        assert_eq!(fit("株式会社サーバー", 7), "株式会…");
        assert_eq!(fit("株式会社サーバー", 7).width(), 7);
        assert_eq!(fit("short", 10), "short");
        assert_eq!(pad("ab", 4), "ab  ");
        assert_eq!(fit("abc", 0), "");
    }

    #[test]
    fn numbers_are_fixed_width() {
        assert_eq!(pct(5.0), "  5%");
        assert_eq!(pct(100.0), "100%");
        assert_eq!(pct(f32::NAN), "  –%");
        assert_eq!(
            latency(&Latency::Direct { ms: 11.0 }).len(),
            latency(&Latency::Unmeasured).chars().count()
        );
    }
}
