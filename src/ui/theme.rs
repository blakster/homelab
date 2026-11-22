//! Design tokens (see `.interface-design/system.md`). Names come from the
//! domain, not from a gray scale: `signal` is data, `live`/`relay`/`fault`
//! are status, `wire` is structure, `ink*` are the four text tiers.

use dither_ratatui::Theme as DitherTheme;
use dither_ratatui::prelude::{Rgba, Tone};
use ratatui::style::{Color, Modifier, Style};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalBg {
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Inherit the terminal's own background (`Color::Reset`), adapting ink
    /// tiers and chart contrast to the detected light/dark polarity.
    Auto(TerminalBg),
    Dark,
    Light,
}

impl Mode {
    pub fn is_light(self) -> bool {
        matches!(self, Mode::Light | Mode::Auto(TerminalBg::Light))
    }

    pub fn is_auto(self) -> bool {
        matches!(self, Mode::Auto(_))
    }

    /// Cycle through the 3 modes so every press visibly changes the palette.
    pub fn next(self, detected: TerminalBg) -> Mode {
        match (self, detected) {
            (Mode::Auto(TerminalBg::Dark), _) => Mode::Light,
            (Mode::Light, TerminalBg::Dark) => Mode::Dark,
            (Mode::Dark, TerminalBg::Dark) => Mode::Auto(TerminalBg::Dark),

            (Mode::Auto(TerminalBg::Light), _) => Mode::Dark,
            (Mode::Dark, TerminalBg::Light) => Mode::Light,
            (Mode::Light, TerminalBg::Light) => Mode::Auto(TerminalBg::Light),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Mode::Auto(_) => "term",
            Mode::Dark => "dark",
            Mode::Light => "light",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalEnv {
    pub bg: TerminalBg,
    pub bg_rgb: Option<(u8, u8, u8)>,
    pub plain: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub ground: Color,
    pub ink: Color,
    pub ink2: Color,
    pub ink3: Color,
    pub mute: Color,
    pub wire: Color,
    pub raised: Color,
    pub signal: Color,
    pub live: Color,
    pub relay: Color,
    pub fault: Color,
    pub asleep: Color,
    /// Dithered pixels only render in truecolor; plain mode draws text bars.
    pub truecolor: bool,
    pub light: bool,
    pub dither: DitherTheme,
}

/// The one accent: a muted indigo matching `#6672e8` (OKLCH hue ~268°).
pub const SIGNAL_TONE: Tone = Tone::Hue(268.0);
pub const FAULT_TONE: Tone = Tone::Red;

const fn rgb(hex: u32) -> Color {
    Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

fn nudge_surface((r, g, b): (u8, u8, u8), light: bool) -> Color {
    if light {
        Color::Rgb(r.saturating_sub(12), g.saturating_sub(12), b.saturating_sub(12))
    } else {
        Color::Rgb(r.saturating_add(14), g.saturating_add(14), b.saturating_add(18))
    }
}

impl Palette {
    pub fn new(mode: Mode, plain: bool) -> Self {
        Self::with_terminal_bg(mode, plain, None)
    }

    pub fn with_terminal_bg(mode: Mode, plain: bool, bg_rgb: Option<(u8, u8, u8)>) -> Self {
        if plain {
            // 16-colour ANSI: works in any terminal and over weak SSH clients.
            return Palette {
                ground: Color::Reset,
                ink: Color::Reset,
                ink2: Color::Gray,
                ink3: Color::Gray,
                mute: Color::DarkGray,
                wire: Color::DarkGray,
                raised: Color::Reset,
                signal: Color::Blue,
                live: Color::Green,
                relay: Color::Yellow,
                fault: Color::Red,
                asleep: Color::DarkGray,
                truecolor: false,
                light: mode.is_light(),
                dither: if mode.is_light() {
                    DitherTheme::light()
                } else {
                    DitherTheme::dark()
                },
            };
        }
        let light = mode.is_light();
        let transparent = mode.is_auto();
        let dither_bg = match (transparent, bg_rgb, light) {
            (true, Some((r, g, b)), _) => Rgba::rgb(r, g, b),
            (_, _, false) => Rgba::hex(0x0b0b0d),
            (_, _, true) => Rgba::hex(0xfafafa),
        };
        if !light {
            Palette {
                ground: if transparent { Color::Reset } else { rgb(0x0b0b0d) },
                ink: rgb(0xe8e8ec),
                ink2: rgb(0xb6b6bf),
                ink3: rgb(0x85858f),
                mute: rgb(0x5a5a64),
                wire: rgb(0x2a2a31),
                raised: if transparent {
                    bg_rgb.map(|c| nudge_surface(c, false)).unwrap_or_else(|| rgb(0x16161b))
                } else {
                    rgb(0x16161b)
                },
                signal: rgb(0x6672e8),
                live: rgb(0x3fb27f),
                relay: rgb(0xd49a3a),
                fault: rgb(0xe5484d),
                asleep: rgb(0x3c3c44),
                truecolor: true,
                light: false,
                dither: DitherTheme {
                    background: dither_bg,
                    ..DitherTheme::dark()
                },
            }
        } else {
            Palette {
                ground: if transparent { Color::Reset } else { rgb(0xfafafa) },
                ink: rgb(0x18181b),
                ink2: rgb(0x3f3f46),
                ink3: rgb(0x6b6b74),
                mute: rgb(0x9f9fa8),
                wire: rgb(0xe2e2e6),
                raised: if transparent {
                    bg_rgb.map(|c| nudge_surface(c, true)).unwrap_or_else(|| rgb(0xf0f0f2))
                } else {
                    rgb(0xf0f0f2)
                },
                signal: rgb(0x4453c9),
                live: rgb(0x1f9d63),
                relay: rgb(0xb7791f),
                fault: rgb(0xd92d3a),
                asleep: rgb(0xc8c8ce),
                truecolor: true,
                light: true,
                dither: DitherTheme {
                    background: dither_bg,
                    ..DitherTheme::light()
                },
            }
        }
    }

    pub fn text(&self, c: Color) -> Style {
        Style::new().fg(c).bg(self.ground)
    }
    pub fn strong(&self) -> Style {
        self.text(self.ink).add_modifier(Modifier::BOLD)
    }
    pub fn label(&self) -> Style {
        self.text(self.ink3)
    }
}

/// Detect the local terminal's background polarity, RGB color (via OSC 11),
/// and whether to fall back to plain ANSI-16 (`NO_COLOR` / non-truecolor).
pub fn detect_terminal() -> TerminalEnv {
    let plain = detect_plain_env(
        std::env::var("NO_COLOR").ok().as_deref(),
        std::env::var("COLORTERM").ok().as_deref(),
        std::env::var("TERM").ok().as_deref(),
    );
    if let Some(rgb) = query_osc11() {
        let lum = 0.2126 * rgb.0 as f32 + 0.7152 * rgb.1 as f32 + 0.0722 * rgb.2 as f32;
        let bg = if lum >= 128.0 {
            TerminalBg::Light
        } else {
            TerminalBg::Dark
        };
        return TerminalEnv {
            bg,
            bg_rgb: Some(rgb),
            plain,
        };
    }
    let bg = std::env::var("COLORFGBG")
        .ok()
        .and_then(|v| parse_colorfgbg(&v))
        .unwrap_or(TerminalBg::Dark);
    TerminalEnv {
        bg,
        bg_rgb: None,
        plain,
    }
}

pub fn detect_plain_env(no_color: Option<&str>, colorterm: Option<&str>, term: Option<&str>) -> bool {
    if no_color.is_some_and(|v| !v.is_empty()) {
        return true;
    }
    if let Some(ct) = colorterm {
        let lower = ct.to_ascii_lowercase();
        if lower.contains("truecolor") || lower.contains("24bit") {
            return false;
        }
    }
    matches!(term, Some("dumb" | "vt100" | "ansi" | "xterm"))
}

pub fn parse_colorfgbg(val: &str) -> Option<TerminalBg> {
    let last = val.rsplit(';').next()?.trim().parse::<u8>().ok()?;
    match last {
        0..=6 | 8 => Some(TerminalBg::Dark),
        7 | 9..=15 => Some(TerminalBg::Light),
        _ => None,
    }
}

/// Parse an OSC 11 response like `\x1b]11;rgb:1a1a/1b1b/2626\x07` or `...\x1b\\`.
pub fn parse_osc11_rgb(buf: &[u8]) -> Option<(u8, u8, u8)> {
    let s = std::str::from_utf8(buf).ok()?;
    let idx = s.find("rgb:")?;
    let rest = &s[idx + 4..];
    let end = rest
        .find(|c: char| c == '\x07' || c == '\x1b' || (!c.is_ascii_hexdigit() && c != '/'))
        .unwrap_or(rest.len());
    let mut parts = rest[..end].split('/');
    let r = parse_x_channel(parts.next()?)?;
    let g = parse_x_channel(parts.next()?)?;
    let b = parse_x_channel(parts.next()?)?;
    Some((r, g, b))
}

fn parse_x_channel(hex: &str) -> Option<u8> {
    if hex.is_empty() || hex.len() > 4 {
        return None;
    }
    let v = u16::from_str_radix(hex, 16).ok()?;
    let max = (1u32 << (hex.len() * 4)) - 1;
    Some(((v as u32 * 255 + max / 2) / max) as u8)
}

#[cfg(unix)]
fn query_osc11() -> Option<(u8, u8, u8)> {
    use std::io::{IsTerminal, Read, Write};
    use std::time::{Duration, Instant};

    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return None;
    }
    if crossterm::terminal::enable_raw_mode().is_err() {
        return None;
    }
    struct RawGuard;
    impl Drop for RawGuard {
        fn drop(&mut self) {
            let _ = crossterm::terminal::disable_raw_mode();
        }
    }
    let _guard = RawGuard;

    let mut out = std::io::stdout();
    if out.write_all(b"\x1b]11;?\x07").is_err() || out.flush().is_err() {
        return None;
    }

    #[repr(C)]
    struct PollFd {
        fd: i32,
        events: i16,
        revents: i16,
    }
    const POLLIN: i16 = 0x0001;
    unsafe extern "C" {
        fn poll(fds: *mut PollFd, nfds: usize, timeout: i32) -> i32;
    }

    let deadline = Instant::now() + Duration::from_millis(80);
    let mut stdin = std::io::stdin();
    let mut buf = Vec::with_capacity(64);
    let mut tmp = [0u8; 64];

    while buf.len() < 128 {
        let rem = deadline.saturating_duration_since(Instant::now());
        if rem.is_zero() {
            break;
        }
        let mut pfd = PollFd {
            fd: 0,
            events: POLLIN,
            revents: 0,
        };
        let ready = unsafe { poll(&mut pfd, 1, rem.as_millis().min(80) as i32) };
        if ready <= 0 || (pfd.revents & POLLIN) == 0 {
            break;
        }
        let Ok(n) = stdin.read(&mut tmp) else { break };
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.contains(&0x07) || buf.windows(2).any(|w| w == b"\x1b\\") {
            break;
        }
    }
    parse_osc11_rgb(&buf)
}

#[cfg(not(unix))]
fn query_osc11() -> Option<(u8, u8, u8)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_osc11_bel_and_st() {
        assert_eq!(parse_osc11_rgb(b"\x1b]11;rgb:1a1a/1b1b/2626\x07"), Some((0x1a, 0x1b, 0x26)));
        assert_eq!(parse_osc11_rgb(b"\x1b]11;rgb:ffff/ffff/ffff\x1b\\"), Some((0xff, 0xff, 0xff)));
        assert_eq!(parse_osc11_rgb(b"\x1b]11;rgb:0/0/0\x07"), Some((0, 0, 0)));
        assert_eq!(parse_osc11_rgb(b"garbage"), None);
    }

    #[test]
    fn parses_colorfgbg() {
        assert_eq!(parse_colorfgbg("15;0"), Some(TerminalBg::Dark));
        assert_eq!(parse_colorfgbg("0;15"), Some(TerminalBg::Light));
        assert_eq!(parse_colorfgbg("7;default;0"), Some(TerminalBg::Dark));
        assert_eq!(parse_colorfgbg("bad"), None);
    }

    #[test]
    fn detects_plain_env() {
        assert!(detect_plain_env(Some("1"), Some("truecolor"), Some("xterm-256color")));
        assert!(!detect_plain_env(Some(""), Some("truecolor"), Some("xterm-256color")));
        assert!(!detect_plain_env(None, Some("24bit"), Some("xterm")));
        assert!(detect_plain_env(None, None, Some("dumb")));
        assert!(!detect_plain_env(None, None, Some("xterm-256color")));
    }

    #[test]
    fn auto_mode_uses_transparent_ground() {
        let p = Palette::with_terminal_bg(Mode::Auto(TerminalBg::Dark), false, Some((0x1e, 0x1e, 0x2e)));
        assert_eq!(p.ground, Color::Reset);
        assert!(!p.light);
        assert_eq!(p.dither.background, Rgba::rgb(0x1e, 0x1e, 0x2e));

        let pl = Palette::with_terminal_bg(Mode::Auto(TerminalBg::Light), false, Some((0xf8, 0xf8, 0xf8)));
        assert_eq!(pl.ground, Color::Reset);
        assert!(pl.light);
    }
}
