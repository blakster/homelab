//! btop-style braille plots: each cell is a 2×4 dot grid, so a 60-column
//! chart has 120 samples of horizontal resolution and 4× the vertical.
//!
//! One raster per chart. Each cell keeps the colour of its highest-ranked dot
//! (a trace's line beats any fill, later traces beat earlier ones), so lines
//! stay crisp where they cross a filled area.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

pub struct Trace<'a> {
    pub vals: &'a [f64],
    pub color: Color,
    /// Fill under the line with a gradient that fades toward the ground.
    pub fill: bool,
}

/// Plot `traces` into `r`, scaled to `0..=y_max`, newest sample at the right.
pub fn plot(buf: &mut Buffer, r: Rect, traces: &[Trace], y_max: f64, ground: Color) {
    let (cols, rows) = (r.width as usize * 2, r.height as usize * 4);
    if cols < 2 || rows < 1 || y_max.is_nan() || y_max <= 0.0 {
        return;
    }
    // Per cell: dot bits, colour, and the rank of the dot that set the colour.
    let mut cells = vec![(0u8, Color::Reset, 0usize); r.width as usize * r.height as usize];
    let mut dot = |x: usize, y_up: usize, color: Color, rank: usize| {
        let y = rows - 1 - y_up.min(rows - 1);
        let i = (y / 4) * r.width as usize + x / 2;
        cells[i].0 |= BITS[y % 4][x % 2];
        if rank >= cells[i].2 {
            cells[i].1 = color;
            cells[i].2 = rank;
        }
    };
    for (t, trace) in traces.iter().enumerate() {
        if trace.vals.len() < 2 {
            continue;
        }
        let height = |x: usize| {
            let at = x as f64 * (trace.vals.len() - 1) as f64 / (cols - 1) as f64;
            let (i, f) = (at.floor() as usize, at.fract());
            let v = trace.vals[i] * (1.0 - f) + trace.vals[(i + 1).min(trace.vals.len() - 1)] * f;
            ((v / y_max).clamp(0.0, 1.0) * (rows - 1) as f64).round() as usize
        };
        let mut prev = height(0);
        for x in 0..cols {
            let h = height(x);
            if trace.fill {
                for y in 0..h {
                    let shade = 0.18 + 0.5 * (y as f64 / rows as f64);
                    dot(x, y, blend(trace.color, ground, shade), 1 + t * 2);
                }
            }
            // Join to the previous column so steep moves stay continuous.
            for y in prev.min(h)..=prev.max(h) {
                dot(x, y, trace.color, 2 + t * 2);
            }
            prev = h;
        }
    }
    for (i, (bits, color, _)) in cells.into_iter().enumerate() {
        if bits == 0 {
            continue;
        }
        let (x, y) = (r.x + (i % r.width as usize) as u16, r.y + (i / r.width as usize) as u16);
        let ch = char::from_u32(0x2800 + bits as u32).unwrap_or(' ');
        buf[(x, y)].set_char(ch).set_style(Style::new().fg(color).bg(ground));
    }
}

const BITS: [[u8; 2]; 4] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]];

/// Mix `c` toward `ground` (`amount` 0 = ground, 1 = c). Non-RGB colours
/// (plain mode) pass through unchanged.
pub fn blend(c: Color, ground: Color, amount: f64) -> Color {
    match (c, ground) {
        (Color::Rgb(r1, g1, b1), Color::Rgb(r0, g0, b0)) => {
            let m = |a: u8, b: u8| (b as f64 + (a as f64 - b as f64) * amount).round() as u8;
            Color::Rgb(m(r1, r0), m(g1, g0), m(b1, b0))
        }
        _ => c,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rising_line_climbs_from_bottom_left_to_top_right() {
        let r = Rect::new(0, 0, 4, 2);
        let mut buf = Buffer::empty(r);
        let vals = [0.0, 100.0];
        plot(
            &mut buf,
            r,
            &[Trace {
                vals: &vals,
                color: Color::Red,
                fill: false,
            }],
            100.0,
            Color::Black,
        );
        assert_ne!(buf[(0, 1)].symbol(), " ", "starts bottom-left");
        assert_ne!(buf[(3, 0)].symbol(), " ", "ends top-right");
        assert_eq!(buf[(0, 0)].symbol(), " ");
    }
}
