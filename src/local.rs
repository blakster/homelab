//! The dashboard in the current terminal.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{Event, EventStream};
use ratatui::DefaultTerminal;
use tokio::sync::watch;
use tokio_stream::StreamExt;

use crate::model::Snapshot;
use crate::ui::{self, UiState, state};

pub async fn run(mut snapshots: watch::Receiver<Arc<Snapshot>>) -> Result<()> {
    let mut terminal = ratatui::init();
    let res = event_loop(&mut terminal, &mut snapshots).await;
    ratatui::restore();
    res
}

async fn event_loop(terminal: &mut DefaultTerminal, snapshots: &mut watch::Receiver<Arc<Snapshot>>) -> Result<()> {
    let mut ui = UiState::default();
    let mut events = EventStream::new();
    loop {
        let snap = snapshots.borrow_and_update().clone();
        terminal.draw(|f| ui::render(f.area(), f.buffer_mut(), &snap, &ui))?;
        // Redraw only on new data or input: no idle frame loop.
        tokio::select! {
            changed = snapshots.changed() => { if changed.is_err() { return Ok(()); } }
            ev = events.next() => match ev {
                Some(Ok(Event::Key(k))) if k.kind == crossterm::event::KeyEventKind::Press => {
                    if let Some(a) = state::from_key(k) {
                        let peers = snap.devices().iter().filter(|d| !d.is_self).count();
                        ui.apply(a, peers, snap.services.len());
                    }
                }
                Some(Ok(_)) => {}
                Some(Err(e)) => return Err(e.into()),
                None => return Ok(()),
            },
            _ = tokio::time::sleep(Duration::from_secs(30)) => {}
        }
        if ui.quit {
            return Ok(());
        }
    }
}
