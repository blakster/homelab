//! Design tokens (see `.interface-design/system.md`). Names come from the
//! domain, not from a gray scale: `signal` is data, `live`/`relay`/`fault`
//! are status, `wire` is structure, `ink*` are the four text tiers.

use dither_ratatui::Theme as DitherTheme;
use dither_ratatui::prelude::{Rgba, Tone};
use ratatui::style::{Color, Modifier, Style};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Dark,
    Light,
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
    pub dither: DitherTheme,
}

/// The one accent: an indigo close to strawhat's bars.
pub const SIGNAL_TONE: Tone = Tone::Hue(238.0);
pub const FAULT_TONE: Tone = Tone::Red;

const fn rgb(hex: u32) -> Color {
    Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

impl Palette {
    pub fn new(mode: Mode, plain: bool) -> Self {
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
                dither: DitherTheme::dark(),
            };
        }
        match mode {
            Mode::Dark => Palette {
                ground: rgb(0x0b0b0d),
                ink: rgb(0xe8e8ec),
                ink2: rgb(0xb6b6bf),
                ink3: rgb(0x85858f),
                mute: rgb(0x5a5a64),
                wire: rgb(0x2a2a31),
                raised: rgb(0x16161b),
                signal: rgb(0x6672e8),
                live: rgb(0x3fb27f),
                relay: rgb(0xd49a3a),
                fault: rgb(0xe5484d),
                asleep: rgb(0x3c3c44),
                truecolor: true,
                dither: DitherTheme {
                    background: Rgba::hex(0x0b0b0d),
                    ..DitherTheme::dark()
                },
            },
            Mode::Light => Palette {
                ground: rgb(0xfafafa),
                ink: rgb(0x18181b),
                ink2: rgb(0x3f3f46),
                ink3: rgb(0x6b6b74),
                mute: rgb(0x9f9fa8),
                wire: rgb(0xe2e2e6),
                raised: rgb(0xf0f0f2),
                signal: rgb(0x4453c9),
                live: rgb(0x1f9d63),
                relay: rgb(0xb7791f),
                fault: rgb(0xd92d3a),
                asleep: rgb(0xc8c8ce),
                truecolor: true,
                dither: DitherTheme {
                    background: Rgba::hex(0xfafafa),
                    ..DitherTheme::light()
                },
            },
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
