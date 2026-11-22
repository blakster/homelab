//! Render tests: every fixture at the widths it ships at (stress-real-container),
//! asserted on what a person sees, plus insta snapshots of the text grid.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use crate::fixtures::{DataSet, load};
use crate::ui::state::{Action, Overlay, Pane};
use crate::ui::theme::Mode;
use crate::ui::{UiState, render, to_text};

const SIZES: [(u16, u16); 4] = [(50, 30), (80, 24), (120, 40), (200, 60)];

fn draw(set: DataSet, w: u16, h: u16, ui: &UiState) -> Buffer {
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &load(set), ui);
    buf
}

#[test]
fn snapshots_every_set_at_every_width() {
    for set in DataSet::ALL {
        for (w, h) in SIZES {
            let text = to_text(&draw(set, w, h, &UiState::default()));
            insta::assert_snapshot!(format!("{}_{w}x{h}", set.name()), text);
        }
    }
}

#[test]
fn tiny_terminals_never_panic() {
    for w in 0..40 {
        for h in 0..12 {
            for set in [DataSet::Worst, DataSet::Rows1000] {
                draw(set, w, h, &UiState::default());
            }
        }
    }
    let t = to_text(&draw(DataSet::Demo, 20, 5, &UiState::default()));
    assert!(t.contains("homelab needs"), "{t}");
}

#[test]
fn demo_shows_whats_up_and_no_banner() {
    let t = to_text(&draw(DataSet::Demo, 120, 40, &UiState::default()));
    for want in [
        "Machines",
        "Services",
        "History",
        "11 ms",
        "182 ms",
        "Pulse",
        "disabled in config",
        "last seen 8d",
    ] {
        assert!(t.contains(want), "missing {want:?}\n{t}");
    }
    assert!(!t.contains('▲'), "Air Health is off, not a fault:\n{t}");
}

#[test]
fn faults_lead_with_a_banner_naming_the_root_cause() {
    let t = to_text(&draw(DataSet::Error, 80, 24, &UiState::default()));
    let first_lines: String = t.lines().take(2).collect();
    assert!(first_lines.contains("▲ tailscale: can't run"), "{t}");
    assert!(t.contains("blocked by OrbStack Docker"), "{t}");
}

#[test]
fn empty_states_give_one_next_action() {
    let t = to_text(&draw(DataSet::Empty, 120, 40, &UiState::default()));
    assert!(t.contains("No services yet. Add a [[service]]"), "{t}");
    assert!(t.contains("No peers yet."), "{t}");
}

#[test]
fn long_values_are_truncated_with_an_ellipsis() {
    let t = to_text(&draw(DataSet::Worst, 50, 30, &UiState::default()));
    assert!(t.contains('…'));
    for line in t.lines() {
        assert!(unicode_width::UnicodeWidthStr::width(line) <= 50, "overflow: {line}");
    }
}

#[test]
fn light_theme_paints_a_light_ground() {
    let ui = UiState {
        mode: Mode::Light,
        ..UiState::default()
    };
    let buf = draw(DataSet::Demo, 120, 40, &ui);
    assert_eq!(buf[(60, 39)].bg, Color::Rgb(0xfa, 0xfa, 0xfa));
    let dark = draw(DataSet::Demo, 120, 40, &UiState::default());
    assert_eq!(dark[(60, 39)].bg, Color::Rgb(0x0b, 0x0b, 0x0d));
}

#[test]
fn plain_mode_uses_no_truecolor() {
    let ui = UiState {
        plain: true,
        ..UiState::default()
    };
    let buf = draw(DataSet::Demo, 120, 40, &ui);
    let rgb = buf
        .content()
        .iter()
        .filter(|c| matches!(c.fg, Color::Rgb(..)) || matches!(c.bg, Color::Rgb(..)))
        .count();
    assert_eq!(rgb, 0);
    assert!(to_text(&buf).contains('█'), "text bars instead of thin gauges");
}

#[test]
fn detail_overlays_show_the_selected_row() {
    let mut ui = UiState::default();
    ui.apply(Action::Down, 4, 4); // vansh-box-1
    ui.apply(Action::Open, 4, 4);
    assert_eq!(ui.overlay, Overlay::Detail);
    let t = to_text(&draw(DataSet::Demo, 120, 40, &ui));
    assert!(t.contains("relayed via DERP bom"), "{t}");

    let ui = UiState {
        pane: Pane::Services,
        sel_service: 0,
        overlay: Overlay::Detail,
        ..UiState::default()
    };
    let t = to_text(&draw(DataSet::Worst, 120, 40, &ui));
    assert!(t.contains("reason"), "{t}");

    let ui = UiState {
        overlay: Overlay::Help,
        ..UiState::default()
    };
    assert!(to_text(&draw(DataSet::Demo, 80, 24, &ui)).contains("plain mode"));
}

#[test]
fn rendering_is_deterministic() {
    let a = to_text(&draw(DataSet::Demo, 120, 40, &UiState::default()));
    let b = to_text(&draw(DataSet::Demo, 120, 40, &UiState::default()));
    assert_eq!(a, b);
}
