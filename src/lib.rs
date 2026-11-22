//! homelab: a terminal dashboard for a Mac and its tailnet.
//!
//! collectors → [`hub`] (single writer) → `watch<Arc<Snapshot>>` → views
//! ([`local`] TUI, [`ssh`] sessions, `snapshot --json`). Every view calls the
//! same pure [`ui::render`].

pub mod collect;
pub mod config;
#[cfg(any(test, feature = "dev-fixtures"))]
pub mod fixtures;
pub mod hub;
pub mod local;
pub mod model;
pub mod ssh;
pub mod ui;
