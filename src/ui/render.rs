//! `render(area, buf, snapshot, ui)`: one pure function draws every view —
//! the local TUI, each SSH session, and the snapshot tests.
//!
//! Layout follows `.interface-design/system.md`: one focal hub card, borders
//! only for structure, one accent (`signal`) for data, status colours only for
//! status, 1-cell padding, fixed-width numbers.

use dither_ratatui::dither_core::chart::{format_value, interpolate};
use dither_ratatui::dither_core::{self, Cartesian, PixelGrid};
use dither_ratatui::prelude::*;
use dither_ratatui::raster::paint;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Widget, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::model::*;
use crate::ui::format::{ago, dur, fit, gb, latency, pad, pct};
use crate::ui::state::{Overlay, Pane, UiState};
use crate::ui::theme::{Palette, SIGNAL_TONE};

pub const MIN_W: u16 = 30;
pub const MIN_H: u16 = 8;
const RAIL_W: u16 = 26;
const WIDE: u16 = 120;
const MEDIUM: u16 = 80;
const HUB_H: u16 = 13;
const HISTORY_MIN: u16 = 9;

pub fn render(area: Rect, buf: &mut Buffer, snap: &Snapshot, ui: &UiState) {
    let p = Palette::with_terminal_bg(ui.mode, ui.plain, ui.terminal_bg_rgb);
    if area.is_empty() {
        return;
    }
    Clear.render(area, buf);
    buf.set_style(area, p.text(p.ink));
    if area.width < MIN_W || area.height < MIN_H {
        let msg = format!("homelab needs {MIN_W}×{MIN_H}; this is {}×{}", area.width, area.height);
        buf.set_stringn(area.x, area.y, msg, area.width as usize, p.text(p.ink3));
        return;
    }
    let peers: Vec<&Device> = snap.devices().iter().filter(|d| !d.is_self).collect();
    let status_y = area.bottom() - 1;
    status_bar(buf, Rect::new(area.x, status_y, area.width, 1), snap, &peers, ui, &p);

    let (rail, main) = if area.width >= WIDE {
        (
            Some(Rect::new(area.x, area.y, RAIL_W, area.height - 1)),
            Rect::new(area.x + RAIL_W + 2, area.y, area.width - RAIL_W - 3, area.height - 1),
        )
    } else {
        (None, Rect::new(area.x + 1, area.y, area.width - 2, area.height - 1))
    };
    if let Some(r) = rail {
        draw_rail(buf, r, snap, &peers, &p);
    }

    let mut y = main.y;
    if rail.is_none() {
        header(buf, Rect::new(main.x, y, main.width, 1), snap, &p);
        y += 1;
    }
    if let Some(text) = banner(snap) {
        buf.set_stringn(
            main.x,
            y,
            fit(&text, main.width as usize),
            main.width as usize,
            p.text(p.fault).add_modifier(Modifier::BOLD),
        );
        y += 1;
    }
    let body = Rect::new(main.x, y, main.width, main.bottom().saturating_sub(y));
    if area.width >= WIDE {
        wide_body(buf, body, snap, &peers, ui, &p);
    } else {
        stacked_body(buf, body, snap, &peers, ui, &p, area.width >= MEDIUM);
    }

    match ui.overlay {
        Overlay::None => {}
        Overlay::Help => help(buf, area, &p),
        Overlay::Detail => detail(buf, area, snap, &peers, ui, &p),
    }
}

// ───────────────────────────── layout ─────────────────────────────

fn wide_body(buf: &mut Buffer, body: Rect, snap: &Snapshot, peers: &[&Device], ui: &UiState, p: &Palette) {
    let svc_need = snap.services.len().max(1) as u16 + 1;
    let avail = body.height.saturating_sub(1); // the Machines title
    let topo_want = HUB_H.max(peers.len() as u16 * 2 + 2);
    let topo_h = topo_want.min(avail.saturating_sub(svc_need.min(8) + 1)).max(HUB_H.min(avail));
    section_title(buf, Rect::new(body.x, body.y, body.width, 1), "Machines", &machines_meta(snap), p);
    let topo = Rect::new(body.x, body.y + 1, body.width, topo_h);

    let hub_w = 54.min(topo.width * 45 / 100);
    let hub = Rect::new(topo.right() - hub_w, topo.y, hub_w, HUB_H.min(topo.height));
    let box_w = topo.width.saturating_sub(hub_w + 16);
    let group = Rect::new(topo.x, topo.y, box_w, topo.height);
    let rows = peer_group(buf, group, snap, peers, ui, p);
    hub_card(buf, hub, snap, p, false);
    edges(buf, group.right(), hub, &rows, p);

    let below = Rect::new(
        body.x,
        topo.bottom() + 1,
        body.width,
        body.bottom().saturating_sub(topo.bottom() + 1),
    );
    let svc_h = (svc_need + 1).min(below.height);
    let (svc, hist) = if below.height >= svc_h + HISTORY_MIN {
        split_v(below, svc_h)
    } else {
        (below, Rect::default())
    };
    services(buf, svc, snap, ui, p);
    history(buf, hist, snap, peers, p);
}

fn split_v(r: Rect, top_h: u16) -> (Rect, Rect) {
    (
        Rect::new(r.x, r.y, r.width, top_h),
        Rect::new(r.x, r.y + top_h, r.width, r.height - top_h),
    )
}

/// CPU/memory history and per-peer latency as Dither Kit charts. Only drawn
/// when there's room left after the things that answer "what's down".
fn history(buf: &mut Buffer, r: Rect, snap: &Snapshot, peers: &[&Device], p: &Palette) {
    if r.height < HISTORY_MIN || r.width < 40 {
        return;
    }
    let mins = snap.history.cpu.len() as i64 * 2 / 60;
    section_title(
        buf,
        Rect::new(r.x, r.y, r.width, 1),
        "History",
        &format!("last {} min", mins.max(1)),
        p,
    );
    let body = Rect::new(r.x, r.y + 1, r.width, r.height - 1);
    let half = body.width / 2;
    let left = Rect::new(body.x, body.y, half.saturating_sub(1), body.height);
    let right = Rect::new(body.x + half + 1, body.y, body.width - half - 1, body.height);
    let cpu: Vec<f64> = snap.history.cpu.iter().map(|v| *v as f64).collect();
    let mem: Vec<f64> = snap.history.mem.iter().map(|v| *v as f64).collect();
    if cpu.len() >= 2 && p.truecolor {
        let series = [Series::new("cpu %", SIGNAL_TONE, cpu), Series::new("mem %", Tone::Grey, mem)];
        let labels = sparse_labels(series[0].values.len(), 2);
        let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
        render_braille_chart(buf, left, &series, &labels, p, true, true);
    } else {
        spark(buf, left, &cpu, Some(100.0), SIGNAL_TONE, p);
    }
    let lat: Vec<(&Device, Vec<f64>)> = peers
        .iter()
        .filter(|d| matches!(d.presence, Presence::Online { .. }))
        .filter_map(|d| {
            snap.history
                .latency
                .get(&d.id)
                .filter(|h| h.len() >= 2)
                .map(|h| (*d, h.iter().map(|v| *v as f64).collect()))
        })
        .take(3)
        .collect();
    if lat.is_empty() {
        buf.set_stringn(
            right.x,
            right.y,
            "latency: waiting for pings…",
            right.width as usize,
            p.text(p.mute),
        );
        return;
    }
    if p.truecolor {
        let n = lat.iter().map(|(_, v)| v.len()).min().unwrap_or(0);
        let tones = [SIGNAL_TONE, Tone::Grey, Tone::Orange];
        let series: Vec<Series> = lat
            .iter()
            .enumerate()
            .map(|(i, (d, v))| Series::new(fit(&d.name, 18), tones[i], v[v.len() - n..].to_vec()))
            .collect();
        let labels = sparse_labels(n, 30);
        let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
        render_braille_chart(buf, right, &series, &labels, p, true, false);
    } else {
        spark(buf, right, &lat[0].1, None, SIGNAL_TONE, p);
    }
}

/// Braille (`2×4` dots/cell) chart over `dither-core`: crisp strokes for every
/// series, optional low-density ordered-dither fill under only the primary
/// series, no background dot clouds or gridlines.
fn render_braille_chart(
    buf: &mut Buffer,
    area: Rect,
    series: &[Series],
    labels: &[&str],
    p: &Palette,
    show_legend: bool,
    soft_fill_first: bool,
) {
    if area.width < 6 || area.height < 4 {
        return;
    }
    let mut plot = area;
    if show_legend && plot.height > 3 {
        let mut x = plot.x;
        let total: u16 = series
            .iter()
            .filter(|s| !s.hidden)
            .map(|s| 3 + s.label.width() as u16 + 2)
            .sum::<u16>()
            .saturating_sub(2);
        if total < plot.width {
            x += (plot.width - total) / 2;
        }
        for s in series.iter().filter(|s| !s.hidden) {
            let swatch = s.tone.shade(if p.light { 0.45 } else { 0.68 });
            let c = Color::Rgb(swatch.r, swatch.g, swatch.b);
            if x + 2 <= plot.right() {
                buf.set_string(x, plot.y, "── ", p.text(c));
                x += 3;
            }
            let rem = plot.right().saturating_sub(x) as usize;
            if rem > 0 {
                buf.set_stringn(x, plot.y, &s.label, rem, p.text(p.ink3));
                x = x.saturating_add(s.label.width() as u16 + 2);
            }
        }
        plot.y += 1;
        plot.height -= 1;
    }

    let mut core = Cartesian::new(CartesianKind::Line, series);
    core.grid_lines = false;
    let scale = core.scale();
    let ticks = &scale.ticks;
    let axis_w = (ticks.iter().map(|v| format_value(*v).len()).max().unwrap_or(0) as u16 + 1).min(plot.width / 2);
    let label_h = u16::from(!labels.is_empty() && plot.height > 2);
    plot.x += axis_w;
    plot.width = plot.width.saturating_sub(axis_w);
    plot.height = plot.height.saturating_sub(label_h);
    if plot.width == 0 || plot.height == 0 {
        return;
    }

    let (pw, ph) = (plot.width as usize * 2, plot.height as usize * 4);
    let mut g = PixelGrid::new(pw, ph);
    let n = series.iter().map(|s| s.values.len()).max().unwrap_or(0);

    let baseline = if p.light { Rgba::hex(0xd4d4dc) } else { Rgba::hex(0x2a2a31) };
    for x in (0..pw as i32).step_by(2) {
        g.set(x, ph as i32 - 1, baseline);
    }

    if soft_fill_first && !series.is_empty() {
        let s0 = &series[0];
        for x in 0..pw {
            let f = if n > 1 { x as f32 / (pw - 1) as f32 * (n - 1) as f32 } else { 0.0 };
            let v = interpolate(&s0.values, f);
            let top = scale.y(v, ph);
            let span = (ph - 1) as f32 - top;
            for y in (top.ceil() as i32 + 1)..(ph as i32 - 1) {
                let t = 1.0 - (y as f32 - top) / span.max(1.0);
                let density = if p.light { 0.08 + 0.30 * t } else { 0.08 + 0.36 * t };
                if Fill::Gradient.covers(x as i32, y, density) {
                    let shade = if p.light { 0.76 - 0.14 * t } else { 0.22 + 0.16 * t };
                    g.set(x as i32, y, s0.tone.shade(shade));
                }
            }
        }
    }

    for (si, s) in series.iter().filter(|s| !s.hidden).enumerate() {
        let stroke = match (p.light, si) {
            (false, 0) => s.tone.shade(0.72),
            (false, _) => s.tone.shade(0.62),
            (true, 0) => s.tone.shade(0.42),
            (true, _) => s.tone.shade(0.52),
        };
        let mut prev: Option<i32> = None;
        for x in 0..pw {
            let f = if n > 1 { x as f32 / (pw - 1) as f32 * (n - 1) as f32 } else { 0.0 };
            let v = interpolate(&s.values, f);
            let ty = scale.y(v, ph).round() as i32;
            let from = prev.unwrap_or(ty);
            for y in from.min(ty)..=from.max(ty) {
                g.set(x as i32, y, stroke);
            }
            prev = Some(ty);
        }
    }

    paint_braille(&g, plot, buf, p);

    for &v in ticks {
        let text = format_value(v);
        let py = scale.y(v, ph).round().clamp(0.0, (ph - 1) as f32) as u16;
        let row = py / 4;
        if row < plot.height {
            let tx = area.x + axis_w.saturating_sub(text.len() as u16 + 1);
            buf.set_string(tx, plot.y + row, text, p.text(p.mute));
        }
    }
    if label_h > 0 {
        let ly = plot.bottom();
        let count = labels.len();
        for (i, label) in labels.iter().enumerate() {
            if label.is_empty() {
                continue;
            }
            let frac = if count > 1 { i as f32 / (count - 1) as f32 } else { 0.5 };
            let col = (frac * plot.width.saturating_sub(1) as f32).round() as u16;
            let start = (plot.x + col)
                .saturating_sub(label.len() as u16 / 2)
                .clamp(plot.x, plot.right().saturating_sub(label.len() as u16));
            buf.set_stringn(start, ly, label, plot.right().saturating_sub(start) as usize, p.text(p.mute));
        }
    }
}

/// Paint a `2×4` subpixel `PixelGrid` into Braille cells (`U+2800..U+28FF`),
/// preserving `p.ground` so transparent terminal themes show through cleanly.
fn paint_braille(grid: &PixelGrid, area: Rect, buf: &mut Buffer, p: &Palette) {
    const BITS: [[u8; 2]; 4] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]];
    let bg = p.dither.background;
    for row in 0..area.height as usize {
        for col in 0..area.width as usize {
            let mut mask = 0u8;
            let (mut r_sum, mut g_sum, mut b_sum, mut count) = (0u32, 0u32, 0u32, 0u32);
            for (dy, bit_row) in BITS.iter().enumerate() {
                for (dx, &bit) in bit_row.iter().enumerate() {
                    if let Some(px) = grid.get(col * 2 + dx, row * 4 + dy)
                        && px.a > 24
                    {
                        mask |= bit;
                        let blended = if px.a == 255 { px } else { px.over(bg) };
                        r_sum += blended.r as u32;
                        g_sum += blended.g as u32;
                        b_sum += blended.b as u32;
                        count += 1;
                    }
                }
            }
            let cell = &mut buf[(area.x + col as u16, area.y + row as u16)];
            cell.set_bg(p.ground);
            if let Some(r) = r_sum.checked_div(count) {
                let ch = char::from_u32(0x2800 + mask as u32).unwrap_or(' ');
                let fg = Color::Rgb(r as u8, (g_sum / count) as u8, (b_sum / count) as u8);
                cell.set_char(ch).set_fg(fg);
            } else {
                cell.set_char(' ').set_fg(p.ink);
            }
        }
    }
}

/// x-axis labels for `n` samples taken every `secs`: only the ends are named.
fn sparse_labels(n: usize, secs: usize) -> Vec<String> {
    (0..n)
        .map(|i| match i {
            0 => format!("-{}m", (n - 1) * secs / 60),
            _ if i == n - 1 => "now".into(),
            _ => String::new(),
        })
        .collect()
}

fn stacked_body(buf: &mut Buffer, body: Rect, snap: &Snapshot, peers: &[&Device], ui: &UiState, p: &Palette, medium: bool) {
    let hub_h = if medium { 11 } else { 8 }.min(body.height);
    hub_card(buf, Rect::new(body.x, body.y, body.width, hub_h), snap, p, !medium);
    let rest = Rect::new(body.x, body.y + hub_h, body.width, body.height - hub_h);
    if rest.height < 2 {
        return;
    }
    let svc_need = snap.services.len().max(1) as u16 + 1;
    let peer_need = peers.len().max(1) as u16 * 2 + 1;
    let peer_h = peer_need.min(rest.height.saturating_sub(svc_need.min(rest.height / 2)));
    let peer_rect = Rect::new(rest.x, rest.y, rest.width, peer_h);
    section_title(
        buf,
        Rect::new(peer_rect.x, peer_rect.y, peer_rect.width, 1),
        "Tailnet",
        &machines_meta(snap),
        p,
    );
    if peer_rect.height > 1 {
        peer_list(
            buf,
            Rect::new(peer_rect.x, peer_rect.y + 1, peer_rect.width, peer_rect.height - 1),
            snap,
            peers,
            ui,
            p,
        );
    }
    let below = Rect::new(
        rest.x,
        peer_rect.bottom(),
        rest.width,
        rest.bottom().saturating_sub(peer_rect.bottom()),
    );
    let svc_h = (svc_need + 1).min(below.height);
    let (svc, hist) = if medium && below.height >= svc_h + HISTORY_MIN {
        split_v(below, svc_h)
    } else {
        (below, Rect::default())
    };
    services(buf, svc, snap, ui, p);
    history(buf, hist, snap, peers, p);
}

// ───────────────────────────── pieces ─────────────────────────────

fn header(buf: &mut Buffer, r: Rect, snap: &Snapshot, p: &Palette) {
    let host = snap.host.as_ref().map(|h| h.name.as_str()).unwrap_or("…");
    let room = (r.width as usize).saturating_sub(12);
    let line = Line::from(vec![
        Span::styled("homelab", p.strong()),
        Span::styled("  ·  ", p.text(p.mute)),
        Span::styled(fit(host, room), p.text(p.ink3)),
    ]);
    buf.set_line(r.x, r.y, &line, r.width);
}

fn section_title(buf: &mut Buffer, r: Rect, title: &str, meta: &str, p: &Palette) {
    buf.set_stringn(r.x, r.y, title, r.width as usize, p.strong());
    let tw = title.width() as u16 + 1;
    let meta = fit(meta, r.width.saturating_sub(tw + 4) as usize);
    let mw = meta.width() as u16;
    let rule_end = r.right().saturating_sub(mw + 1);
    for x in (r.x + tw)..rule_end {
        buf.set_string(x, r.y, "─", p.text(p.wire));
    }
    buf.set_string(r.right() - mw, r.y, meta, p.label());
}

fn machines_meta(snap: &Snapshot) -> String {
    match &snap.tailnet {
        Tailnet::Loading => "waiting for tailscale…".into(),
        Tailnet::Error { .. } => "tailscale unavailable".into(),
        Tailnet::Ready { devices, .. } => {
            let on = devices.iter().filter(|d| matches!(d.presence, Presence::Online { .. })).count();
            format!("{on} of {} online", devices.len())
        }
    }
}

/// The one-line fault banner, so "what's down and why" is never below the fold.
pub fn banner(snap: &Snapshot) -> Option<String> {
    let name_of = |id: &str| {
        snap.services
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.name.clone())
            .unwrap_or_else(|| id.to_string())
    };
    let mut faults: Vec<String> = snap
        .faults()
        .map(|s| {
            let since = dur(snap.at - s.since);
            match &s.state {
                ServiceState::Down { reason } => format!("{} down · {reason} · {since}", s.name),
                ServiceState::Blocked { by } => format!("{} blocked by {} · {since}", s.name, name_of(by)),
                _ => unreachable!("faults() only yields down/blocked"),
            }
        })
        .collect();
    if let Tailnet::Error { reason } = &snap.tailnet {
        faults.insert(0, format!("tailscale: {reason}"));
    }
    let first = faults.first()?;
    let more = if faults.len() > 1 {
        format!("  +{} more", faults.len() - 1)
    } else {
        String::new()
    };
    Some(format!("▲ {first}{more}"))
}

fn status_bar(buf: &mut Buffer, r: Rect, snap: &Snapshot, peers: &[&Device], ui: &UiState, p: &Palette) {
    let on = peers.iter().filter(|d| matches!(d.presence, Presence::Online { .. })).count();
    let mut left = format!(" {on} online · {} offline", peers.len() - on);
    if let Some(h) = &snap.host {
        left.push_str(&format!(" · up {}", dur(h.uptime_secs as i64 * 1000)));
    }
    let next_theme = ui.mode.next(ui.detected_bg).label();
    let keys = [
        format!("t {next_theme}"),
        if ui.plain { "p dither".into() } else { "p plain".into() },
        "tab pane".into(),
        "⏎ detail".into(),
        "? help".into(),
        "q quit".into(),
    ];
    let room = (r.width as usize).saturating_sub(left.width() + 2);
    let mut right = String::new();
    for k in keys.iter().rev() {
        let cand = if right.is_empty() { k.clone() } else { format!("{k} · {right}") };
        if cand.width() > room {
            break;
        }
        right = cand;
    }
    buf.set_stringn(r.x, r.y, &left, r.width as usize, p.label());
    let rw = right.width() as u16;
    buf.set_string(r.right().saturating_sub(rw + 1), r.y, right, p.text(p.mute));
}

fn draw_rail(buf: &mut Buffer, r: Rect, snap: &Snapshot, peers: &[&Device], p: &Palette) {
    for y in r.y..r.bottom() {
        buf.set_string(r.right(), y, "│", p.text(p.wire));
    }
    let x = r.x + 1;
    let w = (r.width - 2) as usize;
    let mut y = r.y;
    let end = r.bottom();
    let put = |buf: &mut Buffer, y: &mut u16, left: Span, right: Span| {
        if *y >= end {
            return;
        }
        let rw = right.width();
        let lw = w.saturating_sub(rw + 1);
        buf.set_span(x, *y, &Span::styled(pad(&left.content, lw), left.style), lw as u16);
        buf.set_span(x + (w - rw) as u16, *y, &right, rw as u16);
        *y += 1;
    };
    put(buf, &mut y, Span::styled("homelab", p.strong()), Span::styled("", p.label()));
    y += 1;
    put(buf, &mut y, Span::styled("THIS MAC", p.text(p.mute)), Span::raw(""));
    match &snap.host {
        Some(h) => put(
            buf,
            &mut y,
            Span::styled(h.name.clone(), p.text(p.ink2)),
            Span::styled(pct(h.cpu_pct).trim().to_string(), p.label()),
        ),
        None => put(buf, &mut y, Span::styled("sampling…", p.text(p.mute)), Span::raw("")),
    }
    y += 1;
    let on = peers.iter().filter(|d| matches!(d.presence, Presence::Online { .. })).count();
    put(
        buf,
        &mut y,
        Span::styled("TAILNET", p.text(p.mute)),
        Span::styled(format!("{on}/{}", peers.len()), p.text(p.mute)),
    );
    let svc_rows = snap.services.len().min(6) as u16 + 2;
    let peer_end = end.saturating_sub(svc_rows + 1);
    for (i, d) in peers.iter().enumerate() {
        if y + 1 >= peer_end && i + 1 < peers.len() {
            put(
                buf,
                &mut y,
                Span::styled(format!("+{} more", peers.len() - i), p.text(p.mute)),
                Span::raw(""),
            );
            break;
        }
        let (dot, right) = match &d.presence {
            Presence::Online { latency: l } => (
                Span::styled("● ", p.text(p.live)),
                Span::styled(latency(l).trim().to_string(), p.label()),
            ),
            Presence::Offline { last_seen } => (
                Span::styled("○ ", p.text(p.asleep)),
                Span::styled(
                    last_seen.map(|t| dur(snap.at - t)).unwrap_or_else(|| "never".into()),
                    p.text(p.mute),
                ),
            ),
        };
        let name_style = if matches!(d.presence, Presence::Online { .. }) {
            p.text(p.ink2)
        } else {
            p.text(p.ink3)
        };
        if y < end {
            buf.set_span(x, y, &dot, 2);
        }
        let rw = right.width();
        let lw = w.saturating_sub(rw + 3);
        if y < end {
            buf.set_stringn(x + 2, y, pad(&d.name, lw), lw, name_style);
            buf.set_span(x + (w - rw) as u16, y, &right, rw as u16);
            y += 1;
        }
    }
    y += 1;
    let up = snap.services.iter().filter(|s| matches!(s.state, ServiceState::Up { .. })).count();
    let active = snap.services.iter().filter(|s| !matches!(s.state, ServiceState::Off)).count();
    put(
        buf,
        &mut y,
        Span::styled("SERVICES", p.text(p.mute)),
        Span::styled(format!("{up}/{active}"), p.text(p.mute)),
    );
    for (i, s) in snap.services.iter().enumerate() {
        if y + 1 >= end && i + 1 < snap.services.len() {
            put(
                buf,
                &mut y,
                Span::styled(format!("+{} more", snap.services.len() - i), p.text(p.mute)),
                Span::raw(""),
            );
            break;
        }
        let (dot_c, word) = state_look(&s.state, p);
        if y < end {
            buf.set_string(x, y, "● ", p.text(dot_c));
            let rw = word.width();
            let lw = w.saturating_sub(rw + 3);
            buf.set_stringn(x + 2, y, pad(&s.name, lw), lw, p.text(p.ink2));
            buf.set_string(x + (w - rw) as u16, y, word, p.text(dot_c));
            y += 1;
        }
    }
}

fn state_look(s: &ServiceState, p: &Palette) -> (Color, &'static str) {
    match s {
        ServiceState::Up { .. } => (p.live, "up"),
        ServiceState::Down { .. } => (p.fault, "down"),
        ServiceState::Blocked { .. } => (p.relay, "blocked"),
        ServiceState::Off => (p.asleep, "off"),
        ServiceState::Unknown => (p.mute, "checking"),
    }
}

// ───────────────────────────── hub card ─────────────────────────────

fn hub_card(buf: &mut Buffer, r: Rect, snap: &Snapshot, p: &Palette, compact: bool) {
    if r.height < 3 || r.width < 20 {
        return;
    }
    let block = Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(p.text(p.wire))
        .title(Line::from(vec![Span::styled(" hub ", p.text(p.signal))]))
        .style(p.text(p.ink));
    let inner = block.inner(r);
    block.render(r, buf);
    let inner = Rect::new(inner.x + 1, inner.y, inner.width.saturating_sub(2), inner.height);
    let Some(h) = &snap.host else {
        buf.set_string(inner.x, inner.y, "sampling…", p.text(p.mute));
        return;
    };
    let mut y = inner.y;

    let text_x = if !compact && inner.height >= 8 && inner.width >= 34 {
        avatar(buf, Rect::new(inner.x, inner.y, 8, 4), &h.name, p);
        inner.x + 10
    } else {
        inner.x
    };
    let tw = inner.right().saturating_sub(text_x) as usize;
    let lines: Vec<Line> = vec![
        Line::from(Span::styled(fit(&h.name, tw), p.strong())),
        Line::from(Span::styled(
            fit(&format!("{} · {} cores · {} GB", h.model, h.cores, gb(h.mem.total)), tw),
            p.text(p.ink3),
        )),
        Line::from(Span::styled(
            fit(
                &format!(
                    "up {} · load {:.2} {:.2} {:.2}",
                    dur(h.uptime_secs as i64 * 1000),
                    h.load[0],
                    h.load[1],
                    h.load[2]
                ),
                tw,
            ),
            p.text(p.ink3),
        )),
        Line::from(match h.pressure {
            Pressure::Normal => Span::styled("memory pressure normal", p.text(p.mute)),
            Pressure::Warn => Span::styled("memory pressure warn", p.text(p.relay)),
            Pressure::Critical => Span::styled("memory pressure critical", p.text(p.fault).add_modifier(Modifier::BOLD)),
        }),
    ];
    let text_rows = if text_x == inner.x { 2 } else { 4 };
    for l in lines.iter().take(text_rows) {
        if y < inner.bottom() {
            buf.set_line(text_x, y, l, tw as u16);
            y += 1;
        }
    }
    let meter_rows = 2u16;
    let spark_h = inner.bottom().saturating_sub(y + meter_rows);
    if spark_h > 0 {
        let vals: Vec<f64> = snap.history.cpu.iter().map(|v| *v as f64).collect();
        spark(buf, Rect::new(inner.x, y, inner.width, spark_h), &vals, Some(100.0), SIGNAL_TONE, p);
        y += spark_h;
    }
    if y < inner.bottom() {
        let half = inner.width / 2;
        meter_row(buf, Rect::new(inner.x, y, half.saturating_sub(1), 1), "CPU", h.cpu_pct / 100.0, p);
        meter_row(
            buf,
            Rect::new(inner.x + half + 1, y, inner.width - half - 1, 1),
            "MEM",
            h.mem.ratio(),
            p,
        );
        y += 1;
    }
    if y < inner.bottom() {
        let cap = if h.disk.total == 0 {
            " no disk data".to_string()
        } else {
            format!(" {} of {} GB", gb(h.disk.used), gb(h.disk.total))
        };
        let cw = (cap.width() as u16).min(inner.width / 2);
        meter_row(buf, Rect::new(inner.x, y, inner.width - cw, 1), "DISK", h.disk.ratio(), p);
        buf.set_stringn(inner.right() - cw, y, cap, cw as usize, p.text(p.ink3));
    }
}

fn meter_row(buf: &mut Buffer, r: Rect, label: &str, ratio: f32, p: &Palette) {
    if r.width < 12 {
        return;
    }
    let ratio = if ratio.is_finite() { ratio.clamp(0.0, 1.0) } else { 0.0 };
    buf.set_string(r.x, r.y, format!("{label:<4}"), p.text(p.mute));
    let v = pct(ratio * 100.0);
    let bar = Rect::new(r.x + 5, r.y, r.width - 5 - 5, 1);
    let fill_color = if ratio >= 0.9 { p.fault } else { p.signal };
    if p.truecolor {
        // Solid 1/8th-block bar + faint diagonal-hatched empty track.
        const PARTIALS: [&str; 8] = ["", "▏", "▎", "▍", "▌", "▋", "▊", "▉"];
        let total_eighths = (ratio * bar.width as f32 * 8.0).round() as usize;
        let full = (total_eighths / 8).min(bar.width as usize);
        let rem = if full < bar.width as usize { total_eighths % 8 } else { 0 };
        let empty = (bar.width as usize).saturating_sub(full + usize::from(rem > 0));
        let mut x = bar.x;
        if full > 0 {
            buf.set_string(x, bar.y, "█".repeat(full), p.text(fill_color));
            x += full as u16;
        }
        if rem > 0 {
            buf.set_string(x, bar.y, PARTIALS[rem], p.text(fill_color));
            x += 1;
        }
        if empty > 0 {
            buf.set_string(x, bar.y, "╱".repeat(empty), p.text(p.wire));
        }
    } else {
        let filled = (ratio * bar.width as f32).round() as usize;
        let s = format!("{}{}", "█".repeat(filled), "░".repeat(bar.width as usize - filled));
        buf.set_string(bar.x, bar.y, s, p.text(fill_color));
    }
    let c = if ratio >= 0.9 { p.fault } else { p.ink2 };
    buf.set_string(r.right() - 4, r.y, v, p.text(c));
}

/// A chrome-free Braille sparkline. With `max`, the scale is fixed (CPU 0–100),
/// so a quiet machine looks quiet instead of auto-scaling noise into mountains.
fn spark(buf: &mut Buffer, r: Rect, vals: &[f64], max: Option<f64>, tone: Tone, p: &Palette) {
    if vals.len() < 2 || r.width == 0 || r.height == 0 {
        buf.set_stringn(r.x, r.y, "collecting…", r.width as usize, p.text(p.mute));
        return;
    }
    let tail = &vals[vals.len().saturating_sub(r.width as usize * 2)..];
    if p.truecolor {
        let (pw, ph) = (r.width as usize * 2, r.height as usize * 4);
        let mut g = PixelGrid::new(pw, ph);
        let hi = max.unwrap_or_else(|| tail.iter().copied().fold(1e-9, f64::max) * 1.15);
        let n = tail.len();
        let stroke = tone.shade(if p.light { 0.42 } else { 0.72 });
        let mut prev: Option<i32> = None;
        for x in 0..pw {
            let f = if n > 1 { x as f32 / (pw - 1) as f32 * (n - 1) as f32 } else { 0.0 };
            let v = interpolate(tail, f);
            let frac = (v / hi.max(1e-9)).clamp(0.0, 1.0) as f32;
            let top = (1.0 - frac) * (ph - 1) as f32;
            let span = (ph - 1) as f32 - top;
            for y in (top.ceil() as i32 + 1)..(ph as i32) {
                let t = 1.0 - (y as f32 - top) / span.max(1.0);
                let density = if p.light { 0.08 + 0.30 * t } else { 0.08 + 0.36 * t };
                if Fill::Gradient.covers(x as i32, y, density) {
                    let shade = if p.light { 0.76 - 0.14 * t } else { 0.22 + 0.16 * t };
                    g.set(x as i32, y, tone.shade(shade));
                }
            }
            let ty = top.round() as i32;
            let from = prev.unwrap_or(ty);
            for y in from.min(ty)..=from.max(ty) {
                g.set(x as i32, y, stroke);
            }
            prev = Some(ty);
        }
        paint_braille(&g, r, buf, p);
    } else {
        const BARS: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
        let plain_tail = &vals[vals.len().saturating_sub(r.width as usize)..];
        let hi = max.unwrap_or_else(|| plain_tail.iter().copied().fold(1e-9, f64::max));
        let s: String = plain_tail
            .iter()
            .map(|v| BARS[((v / hi).clamp(0.0, 1.0) * 7.0).round() as usize])
            .collect();
        buf.set_string(r.x, r.bottom() - 1, s, p.text(p.signal));
    }
}

/// Deterministic identicon from the hostname: Dither Kit's 8×8 avatar on the
/// hub card (8×4 half-block cells).
fn avatar(buf: &mut Buffer, r: Rect, name: &str, p: &Palette) {
    if !p.truecolor {
        buf.set_string(r.x, r.y, "◆◆", p.text(p.signal));
        return;
    }
    let src = dither_core::avatar(name, Some(268.0));
    paint(&src, r, buf, p.dither.background);
}

// ───────────────────────────── peers ─────────────────────────────

/// Peers inside a bordered group box (wide layout). Returns the row and device
/// of every visible online peer so `edges` can wire them to the hub.
fn peer_group<'a>(buf: &mut Buffer, r: Rect, snap: &Snapshot, peers: &[&'a Device], ui: &UiState, p: &Palette) -> Vec<(u16, &'a Device)> {
    if r.width < 16 || r.height < 3 {
        return vec![];
    }
    let tailnet = match &snap.tailnet {
        Tailnet::Ready { tailnet, .. } => tailnet.clone(),
        _ => "tailnet".into(),
    };
    let block = Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(p.text(p.wire))
        .title(Span::styled(
            format!(" {} ", fit(&tailnet.to_uppercase(), r.width as usize - 4)),
            p.text(p.mute),
        ))
        .style(p.text(p.ink));
    let inner = block.inner(r);
    block.render(r, buf);
    peer_list(
        buf,
        Rect::new(inner.x + 1, inner.y, inner.width.saturating_sub(2), inner.height),
        snap,
        peers,
        ui,
        p,
    )
}

fn peer_list<'a>(buf: &mut Buffer, r: Rect, snap: &Snapshot, peers: &[&'a Device], ui: &UiState, p: &Palette) -> Vec<(u16, &'a Device)> {
    match &snap.tailnet {
        Tailnet::Loading => {
            buf.set_stringn(r.x, r.y, "waiting for tailscale status…", r.width as usize, p.text(p.mute));
            return vec![];
        }
        Tailnet::Error { reason } => {
            let para = Paragraph::new(reason.as_str()).style(p.text(p.fault)).wrap(Wrap { trim: true });
            para.render(r, buf);
            return vec![];
        }
        Tailnet::Ready { .. } if peers.is_empty() => {
            let para = Paragraph::new("No peers yet. Run `tailscale up` on another device.")
                .style(p.text(p.ink3))
                .wrap(Wrap { trim: true });
            para.render(r, buf);
            return vec![];
        }
        Tailnet::Ready { .. } => {}
    }
    let focused = ui.pane == Pane::Machines;
    let fits = peers.len() * 2 <= r.height as usize;
    let visible = (if fits { r.height / 2 } else { r.height.saturating_sub(1) / 2 }).max(1) as usize;
    let offset = ui.sel_peer.saturating_sub(visible - 1);
    let mut rows = vec![];
    for (i, d) in peers.iter().enumerate().skip(offset).take(visible) {
        let y = r.y + ((i - offset) * 2) as u16;
        if y + 1 > r.bottom() {
            break;
        }
        let selected = focused && i == ui.sel_peer;
        if selected {
            buf.set_style(Rect::new(r.x, y, r.width, 2.min(r.bottom() - y)), Style::new().bg(p.raised));
        }
        let bg = |s: Style| if selected { s.bg(p.raised) } else { s };
        let (dot, dot_c) = match &d.presence {
            Presence::Online {
                latency: Latency::Relay { .. },
            } => ("●", p.relay),
            Presence::Online { .. } => ("●", p.live),
            Presence::Offline { .. } => ("○", p.asleep),
        };
        buf.set_string(r.x, y, dot, bg(p.text(dot_c)));
        let x = r.x + 2;
        let w = r.width.saturating_sub(2) as usize;
        let online = matches!(d.presence, Presence::Online { .. });
        let (right, right_c, sub) = match &d.presence {
            Presence::Online { latency: l } => {
                let path = match l {
                    Latency::Direct { .. } => "direct".to_string(),
                    Latency::Relay { region, .. } => format!("via DERP {region}"),
                    Latency::Unmeasured => "online".to_string(),
                };
                let c = if matches!(l, Latency::Relay { .. }) { p.relay } else { p.ink2 };
                (latency(l), c, format!("{} · {path}", d.os))
            }
            Presence::Offline { last_seen } => (
                "offline".to_string(),
                p.mute,
                format!(
                    "{} · {}",
                    d.os,
                    last_seen
                        .map(|t| format!("last seen {}", ago(t, snap.at)))
                        .unwrap_or_else(|| "never seen".into())
                ),
            ),
        };
        let rw = right.width();
        let nw = w.saturating_sub(rw + 1);
        let mut name_style = if online { p.text(p.ink) } else { p.text(p.ink3) };
        if selected {
            name_style = name_style.add_modifier(Modifier::BOLD).bg(p.raised);
        }
        buf.set_stringn(x, y, pad(&d.name, nw), nw, name_style);
        if w > rw {
            buf.set_string(x + (w - rw) as u16, y, &right, bg(p.text(right_c)));
        }
        if y + 1 < r.bottom() {
            buf.set_stringn(x, y + 1, fit(&sub, w), w, bg(p.text(p.ink3)));
        }
        if online {
            rows.push((y, *d));
        }
    }
    if peers.len() > visible {
        let more = format!(" {}–{} of {} ", offset + 1, (offset + visible).min(peers.len()), peers.len());
        let mw = more.width() as u16;
        if r.width > mw {
            buf.set_string(r.right() - mw, r.bottom().saturating_sub(1), more, p.text(p.mute));
        }
    }
    rows
}

/// Wires each online peer row to the hub card, latency on the edge.
fn edges(buf: &mut Buffer, from_x: u16, hub: Rect, rows: &[(u16, &Device)], p: &Palette) {
    if rows.is_empty() || hub.x <= from_x + 6 {
        return;
    }
    let bus_x = hub.x - 3;
    let hub_mid = hub.y + hub.height / 2;
    let top = rows.iter().map(|r| r.0).min().unwrap().min(hub_mid);
    let bottom = rows.iter().map(|r| r.0).max().unwrap().max(hub_mid);
    for (y, d) in rows {
        let Presence::Online { latency: l } = &d.presence else { continue };
        let relay = matches!(l, Latency::Relay { .. });
        let (ch, c) = if relay { ("╌", p.relay) } else { ("─", p.wire) };
        buf.set_string(from_x - 1, *y, "├", p.text(p.wire));
        for x in from_x..bus_x {
            buf.set_string(x, *y, ch, p.text(c));
        }
        let label = format!(" {} ", latency(l).trim());
        let lw = label.width() as u16;
        if bus_x > from_x + lw + 1 {
            buf.set_string(from_x + 1, *y, label, p.text(if relay { p.relay } else { p.ink3 }));
        }
    }
    let left: std::collections::HashSet<u16> = rows.iter().map(|r| r.0).collect();
    for y in top..=bottom {
        let (l, r, u, d) = (left.contains(&y), y == hub_mid, y > top, y < bottom);
        let ch = match (l, r, u, d) {
            (true, true, false, false) => "─",
            (true, false, false, true) => "┐",
            (true, false, true, false) => "┘",
            (true, false, true, true) => "┤",
            (false, true, false, true) => "┌",
            (false, true, true, false) => "└",
            (false, true, true, true) => "├",
            (true, true, false, true) => "┬",
            (true, true, true, false) => "┴",
            (true, true, true, true) => "┼",
            _ => "│",
        };
        buf.set_string(bus_x, y, ch, p.text(p.wire));
    }
    for x in bus_x + 1..hub.x {
        buf.set_string(x, hub_mid, "─", p.text(p.wire));
    }
    buf.set_string(hub.x, hub_mid, "┤", p.text(p.wire));
}

// ───────────────────────────── services ─────────────────────────────

fn services(buf: &mut Buffer, r: Rect, snap: &Snapshot, ui: &UiState, p: &Palette) {
    if r.height == 0 {
        return;
    }
    let up = snap.services.iter().filter(|s| matches!(s.state, ServiceState::Up { .. })).count();
    let off = snap.services.iter().filter(|s| matches!(s.state, ServiceState::Off)).count();
    let list = Rect::new(r.x, r.y + 1, r.width, r.height - 1);
    let visible = (list.height as usize).max(1);
    let offset = ui.sel_service.saturating_sub(visible - 1);
    let mut meta = if off > 0 {
        format!("{up} up · {off} off")
    } else {
        format!("{up} up")
    };
    if snap.services.len() > visible {
        meta = format!(
            "{}–{} of {} · {meta}",
            offset + 1,
            (offset + visible).min(snap.services.len()),
            snap.services.len()
        );
    }
    section_title(buf, Rect::new(r.x, r.y, r.width, 1), "Services", &meta, p);
    if list.height == 0 {
        return;
    }
    if snap.services.is_empty() {
        let para = Paragraph::new("No services yet. Add a [[service]] to ~/.config/homelab/config.toml.")
            .style(p.text(p.ink3))
            .wrap(Wrap { trim: true });
        para.render(list, buf);
        return;
    }
    let name_of = |id: &str| {
        snap.services
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.name.clone())
            .unwrap_or_else(|| id.to_string())
    };
    let focused = ui.pane == Pane::Services;
    let w = list.width as usize;
    let show_since = w >= 60;
    let rest = w.saturating_sub(19 + if show_since { 11 } else { 0 });
    let nw = if show_since {
        (w / 4).clamp(10, 28)
    } else {
        (rest / 2).clamp(8, 28)
    };
    for (i, s) in snap.services.iter().enumerate().skip(offset).take(visible) {
        let y = list.y + (i - offset) as u16;
        let selected = focused && i == ui.sel_service;
        let bg = |st: Style| if selected { st.bg(p.raised) } else { st };
        if selected {
            buf.set_style(Rect::new(list.x, y, list.width, 1), Style::new().bg(p.raised));
        }
        let (c, word) = state_look(&s.state, p);
        let mut x = list.x;
        buf.set_string(x, y, "● ", bg(p.text(c)));
        x += 2;
        let name_c = if matches!(s.state, ServiceState::Off) { p.ink3 } else { p.ink };
        let mut ns = bg(p.text(name_c));
        if selected {
            ns = ns.add_modifier(Modifier::BOLD);
        }
        buf.set_stringn(x, y, pad(&s.name, nw), nw, ns);
        x += nw as u16 + 1;
        buf.set_string(x, y, pad(word, 8), bg(p.text(c)));
        x += 9;
        let ms = match &s.state {
            ServiceState::Up { ms } => format!("{ms:>4} ms"),
            _ => "       ".into(),
        };
        buf.set_string(x, y, &ms, bg(p.text(p.ink2)));
        x += 8;
        if show_since {
            let since = match &s.state {
                ServiceState::Off | ServiceState::Unknown => String::new(),
                _ => format!("for {}", dur(snap.at - s.since)),
            };
            buf.set_string(x, y, pad(&since, 10), bg(p.text(p.ink3)));
            x += 11;
        }
        let (detail, dc) = match &s.state {
            ServiceState::Down { reason } => (reason.clone(), p.fault),
            ServiceState::Blocked { by } => (format!("blocked by {}", name_of(by)), p.relay),
            ServiceState::Off => ("disabled in config".into(), p.mute),
            ServiceState::Unknown => ("waiting for first probe".into(), p.mute),
            ServiceState::Up { .. } => (s.url.clone().unwrap_or_default(), p.ink3),
        };
        let dw = list.right().saturating_sub(x) as usize;
        if dw >= 6 {
            buf.set_stringn(x, y, pad(&detail, dw), dw, bg(p.text(dc)));
        }
    }
}

// ───────────────────────────── overlays ─────────────────────────────

fn popover(buf: &mut Buffer, area: Rect, w: u16, h: u16, title: &str, p: &Palette) -> Rect {
    let w = w.min(area.width.saturating_sub(2));
    let h = h.min(area.height.saturating_sub(2));
    let r = Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h);
    Clear.render(r, buf);
    let block = Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(p.text(p.ink3))
        .title(Span::styled(format!(" {title} "), p.strong()))
        .style(p.text(p.ink));
    let inner = block.inner(r);
    block.render(r, buf);
    Rect::new(inner.x + 1, inner.y, inner.width.saturating_sub(2), inner.height)
}

fn help(buf: &mut Buffer, area: Rect, p: &Palette) {
    let keys = [
        ("tab", "switch Machines / Services"),
        ("↑ ↓  j k", "move"),
        ("⏎  l", "open detail"),
        ("esc  h", "close"),
        ("t", "terminal / dark / light"),
        ("p", "plain mode (no truecolor)"),
        ("g  G", "top / bottom"),
        ("q", "quit"),
    ];
    let inner = popover(buf, area, 48, keys.len() as u16 + 2, "keys", p);
    for (i, (k, v)) in keys.iter().enumerate() {
        let y = inner.y + i as u16;
        if y >= inner.bottom() {
            break;
        }
        buf.set_string(inner.x, y, pad(k, 10), p.text(p.signal));
        buf.set_stringn(inner.x + 11, y, *v, inner.width.saturating_sub(11) as usize, p.text(p.ink2));
    }
}

fn detail(buf: &mut Buffer, area: Rect, snap: &Snapshot, peers: &[&Device], ui: &UiState, p: &Palette) {
    match ui.pane {
        Pane::Machines => {
            let Some(d) = peers.get(ui.sel_peer) else { return };
            let inner = popover(buf, area, 72, 20, &fit(&d.name, 60), p);
            let mut lines = vec![
                Line::from(vec![
                    Span::styled("dns   ", p.text(p.mute)),
                    Span::styled(d.dns_name.clone(), p.text(p.ink2)),
                ]),
                Line::from(vec![
                    Span::styled("os    ", p.text(p.mute)),
                    Span::styled(d.os.clone(), p.text(p.ink2)),
                ]),
            ];
            let status = match &d.presence {
                Presence::Online { latency: l } => match l {
                    Latency::Direct { ms } => format!("online · direct · {ms:.0} ms"),
                    Latency::Relay { ms, region } => format!("online · relayed via DERP {region} · {ms:.0} ms"),
                    Latency::Unmeasured => "online · latency not measured yet".into(),
                },
                Presence::Offline { last_seen } => match last_seen {
                    Some(t) => format!("offline · last seen {}", ago(*t, snap.at)),
                    None => "offline · never seen".into(),
                },
            };
            lines.push(Line::from(vec![
                Span::styled("state ", p.text(p.mute)),
                Span::styled(status, p.text(p.ink)),
            ]));
            let head = lines.len() as u16 + 1;
            Paragraph::new(lines)
                .style(p.text(p.ink))
                .render(Rect::new(inner.x, inner.y, inner.width, head.min(inner.height)), buf);
            let chart_r = Rect::new(inner.x, inner.y + head, inner.width, inner.height.saturating_sub(head));
            match snap.history.latency.get(&d.id).filter(|h| h.len() >= 2) {
                Some(h) if chart_r.height >= 5 => {
                    let vals: Vec<f64> = h.iter().map(|v| *v as f64).collect();
                    if p.truecolor {
                        let n = vals.len();
                        let labels_owned: Vec<String> = (0..n)
                            .map(|i| {
                                if i == 0 {
                                    format!("-{}m", (n - 1) / 2)
                                } else if i == n - 1 {
                                    "now".into()
                                } else {
                                    String::new()
                                }
                            })
                            .collect();
                        let labels: Vec<&str> = labels_owned.iter().map(String::as_str).collect();
                        let series = [Series::new("latency ms", SIGNAL_TONE, vals)];
                        render_braille_chart(buf, chart_r, &series, &labels, p, false, true);
                    } else {
                        spark(buf, chart_r, &vals, None, SIGNAL_TONE, p);
                    }
                }
                _ => {
                    buf.set_stringn(
                        chart_r.x,
                        chart_r.y,
                        "no latency samples yet",
                        chart_r.width as usize,
                        p.text(p.mute),
                    );
                }
            }
        }
        Pane::Services => {
            let Some(s) = snap.services.get(ui.sel_service) else { return };
            let inner = popover(buf, area, 72, 14, &fit(&s.name, 60), p);
            let (c, word) = state_look(&s.state, p);
            let mut lines = vec![
                Line::from(vec![
                    Span::styled("id      ", p.text(p.mute)),
                    Span::styled(s.id.clone(), p.text(p.ink2)),
                ]),
                Line::from(vec![
                    Span::styled("state   ", p.text(p.mute)),
                    Span::styled(word, p.text(c).add_modifier(Modifier::BOLD)),
                ]),
            ];
            if !matches!(s.state, ServiceState::Off | ServiceState::Unknown) {
                lines.push(Line::from(vec![
                    Span::styled("since   ", p.text(p.mute)),
                    Span::styled(format!("{} ago", dur(snap.at - s.since)), p.text(p.ink2)),
                ]));
            }
            if let Some(t) = s.checked_at {
                lines.push(Line::from(vec![
                    Span::styled("checked ", p.text(p.mute)),
                    Span::styled(ago(t, snap.at), p.text(p.ink2)),
                ]));
            }
            if let Some(u) = &s.url {
                lines.push(Line::from(vec![
                    Span::styled("url     ", p.text(p.mute)),
                    Span::styled(u.clone(), p.text(p.signal)),
                ]));
            }
            match &s.state {
                ServiceState::Down { reason } => lines.push(Line::from(vec![
                    Span::styled("reason  ", p.text(p.mute)),
                    Span::styled(reason.clone(), p.text(p.fault)),
                ])),
                ServiceState::Blocked { by } => lines.push(Line::from(vec![
                    Span::styled("blocked ", p.text(p.mute)),
                    Span::styled(format!("by {by} (fix that first)"), p.text(p.relay)),
                ])),
                ServiceState::Off => lines.push(Line::from(Span::styled(
                    "Disabled in config. Set enabled = true to probe it.",
                    p.text(p.ink3),
                ))),
                _ => {}
            }
            Paragraph::new(lines)
                .style(p.text(p.ink))
                .wrap(Wrap { trim: false })
                .render(inner, buf);
        }
    }
}
