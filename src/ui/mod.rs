pub mod format;
pub mod render;
pub mod state;
pub mod theme;

pub use render::render;
pub use state::{Action, UiState};

/// Render a snapshot into a plain-text grid (symbols only), for tests and
/// `homelab render`.
pub fn to_text(buf: &ratatui::buffer::Buffer) -> String {
    let area = buf.area;
    let mut out = String::new();
    for y in 0..area.height {
        let mut skip = 0;
        for x in 0..area.width {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            let sym = buf[(x, y)].symbol();
            skip = unicode_width::UnicodeWidthStr::width(sym).saturating_sub(1);
            out.push_str(sym);
        }
        out.push('\n');
    }
    out
}

/// Every cell as `{s, fg, bg}` with RGB colours, for rendering screenshots.
pub fn to_cells_json(buf: &ratatui::buffer::Buffer) -> String {
    use ratatui::style::Color;
    let rgb = |c: Color| match c {
        Color::Rgb(r, g, b) => serde_json::json!([r, g, b]),
        _ => serde_json::Value::Null,
    };
    let a = buf.area;
    let cells: Vec<_> = (0..a.height)
        .flat_map(|y| (0..a.width).map(move |x| (x, y)))
        .map(|(x, y)| {
            let c = &buf[(x, y)];
            serde_json::json!({"x": x, "y": y, "s": c.symbol(), "fg": rgb(c.fg), "bg": rgb(c.bg)})
        })
        .collect();
    serde_json::json!({"w": a.width, "h": a.height, "cells": cells}).to_string()
}

#[cfg(test)]
mod tests;
