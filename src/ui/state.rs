//! Per-viewer UI state. Each local TUI and each SSH session owns one; it is
//! never shared, so two people can browse different panes and themes.

use crate::ui::theme::{Mode, TerminalBg, TerminalEnv};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    Machines,
    Services,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overlay {
    None,
    Detail,
    Help,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Theme,
    Plain,
    NextPane,
    Up,
    Down,
    Top,
    Bottom,
    Open,
    Close,
    Help,
    Quit,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UiState {
    pub mode: Mode,
    /// Detected terminal polarity used when cycling back to `Mode::Auto`.
    pub detected_bg: TerminalBg,
    /// Terminal background RGB reported by OSC 11 (if available).
    pub terminal_bg_rgb: Option<(u8, u8, u8)>,
    pub plain: bool,
    pub pane: Pane,
    /// Selected row per pane: peers (excluding self), services.
    pub sel_peer: usize,
    pub sel_service: usize,
    pub overlay: Overlay,
    pub quit: bool,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            mode: Mode::Dark,
            detected_bg: TerminalBg::Dark,
            terminal_bg_rgb: None,
            plain: false,
            pane: Pane::Machines,
            sel_peer: 0,
            sel_service: 0,
            overlay: Overlay::None,
            quit: false,
        }
    }
}

impl UiState {
    pub fn from_terminal() -> Self {
        Self::from_terminal_env(crate::ui::theme::detect_terminal())
    }

    pub fn from_terminal_env(env: TerminalEnv) -> Self {
        Self {
            mode: Mode::Auto(env.bg),
            detected_bg: env.bg,
            terminal_bg_rgb: env.bg_rgb,
            plain: env.plain,
            ..Self::default()
        }
    }

    /// `peers` and `services` are the current row counts, used to clamp.
    pub fn apply(&mut self, a: Action, peers: usize, services: usize) {
        let (sel, len) = match self.pane {
            Pane::Machines => (&mut self.sel_peer, peers),
            Pane::Services => (&mut self.sel_service, services),
        };
        match a {
            Action::Quit => self.quit = true,
            Action::Theme => self.mode = self.mode.next(self.detected_bg),
            Action::Plain => self.plain = !self.plain,
            Action::Help => {
                self.overlay = if self.overlay == Overlay::Help {
                    Overlay::None
                } else {
                    Overlay::Help
                }
            }
            Action::Close => self.overlay = Overlay::None,
            Action::Open if len > 0 => self.overlay = Overlay::Detail,
            Action::Open => {}
            Action::NextPane => {
                self.pane = if self.pane == Pane::Machines {
                    Pane::Services
                } else {
                    Pane::Machines
                };
                self.overlay = Overlay::None;
            }
            Action::Up => *sel = sel.saturating_sub(1),
            Action::Down => *sel = (*sel + 1).min(len.saturating_sub(1)),
            Action::Top => *sel = 0,
            Action::Bottom => *sel = len.saturating_sub(1),
        }
        // Rows can disappear between ticks; never point past the end.
        self.sel_peer = self.sel_peer.min(peers.saturating_sub(1));
        self.sel_service = self.sel_service.min(services.saturating_sub(1));
    }
}

/// Raw bytes from an SSH channel → actions. Arrow keys arrive as `ESC [ A`.
pub fn parse_keys(data: &[u8]) -> Vec<Action> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let a = match data[i] {
            0x1b if data.get(i + 1) == Some(&b'[') || data.get(i + 1) == Some(&b'O') => {
                let a = match data.get(i + 2) {
                    Some(b'A') => Some(Action::Up),
                    Some(b'B') => Some(Action::Down),
                    Some(b'H') => Some(Action::Top),
                    Some(b'F') => Some(Action::Bottom),
                    _ => None,
                };
                i += 2;
                a
            }
            0x1b => Some(Action::Close),
            b'q' | 0x03 | 0x04 => Some(Action::Quit),
            b't' => Some(Action::Theme),
            b'p' => Some(Action::Plain),
            b'\t' => Some(Action::NextPane),
            b'k' => Some(Action::Up),
            b'j' => Some(Action::Down),
            b'g' => Some(Action::Top),
            b'G' => Some(Action::Bottom),
            b'\r' | b'\n' | b'l' => Some(Action::Open),
            b'h' => Some(Action::Close),
            b'?' => Some(Action::Help),
            _ => None,
        };
        out.extend(a);
        i += 1;
    }
    out
}

/// Local terminal keys (crossterm) → actions.
pub fn from_key(k: crossterm::event::KeyEvent) -> Option<Action> {
    use crossterm::event::{KeyCode, KeyModifiers};
    if k.modifiers.contains(KeyModifiers::CONTROL) && matches!(k.code, KeyCode::Char('c') | KeyCode::Char('d')) {
        return Some(Action::Quit);
    }
    Some(match k.code {
        KeyCode::Up => Action::Up,
        KeyCode::Down => Action::Down,
        KeyCode::Home => Action::Top,
        KeyCode::End => Action::Bottom,
        KeyCode::Enter => Action::Open,
        KeyCode::Esc => Action::Close,
        KeyCode::Tab | KeyCode::BackTab => Action::NextPane,
        KeyCode::Char(c) if c.is_ascii() => return parse_keys(&[c as u8]).into_iter().next(),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_to_actions() {
        assert_eq!(parse_keys(b"\x1b[A\x1b[Bq"), vec![Action::Up, Action::Down, Action::Quit]);
        assert_eq!(parse_keys(b"\x1b"), vec![Action::Close]);
        assert_eq!(
            parse_keys(b"t\t?\r"),
            vec![Action::Theme, Action::NextPane, Action::Help, Action::Open]
        );
        assert_eq!(parse_keys(b"\x03"), vec![Action::Quit]);
    }

    #[test]
    fn selection_clamps_to_rows() {
        let mut ui = UiState::default();
        for _ in 0..10 {
            ui.apply(Action::Down, 3, 5);
        }
        assert_eq!(ui.sel_peer, 2);
        ui.apply(Action::NextPane, 3, 5);
        ui.apply(Action::Bottom, 3, 5);
        assert_eq!(ui.sel_service, 4);
        ui.apply(Action::Up, 3, 2); // services shrank
        assert_eq!(ui.sel_service, 1);
        ui.apply(Action::Open, 3, 0);
        assert_eq!(ui.overlay, Overlay::None, "nothing to open");
    }

    #[test]
    fn theme_toggles() {
        let mut ui = UiState::from_terminal_env(TerminalEnv {
            bg: TerminalBg::Dark,
            bg_rgb: None,
            plain: false,
        });
        assert_eq!(ui.mode, Mode::Auto(TerminalBg::Dark));
        ui.apply(Action::Theme, 0, 0);
        assert_eq!(ui.mode, Mode::Light);
        ui.apply(Action::Theme, 0, 0);
        assert_eq!(ui.mode, Mode::Dark);
        ui.apply(Action::Theme, 0, 0);
        assert_eq!(ui.mode, Mode::Auto(TerminalBg::Dark));

        let mut light_term = UiState::from_terminal_env(TerminalEnv {
            bg: TerminalBg::Light,
            bg_rgb: None,
            plain: false,
        });
        light_term.apply(Action::Theme, 0, 0);
        assert_eq!(light_term.mode, Mode::Dark);
        light_term.apply(Action::Theme, 0, 0);
        assert_eq!(light_term.mode, Mode::Light);
        light_term.apply(Action::Theme, 0, 0);
        assert_eq!(light_term.mode, Mode::Auto(TerminalBg::Light));
    }
}
