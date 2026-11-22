use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use clap::{Parser, Subcommand};
use homelab::config::Config;
use homelab::{hub, local, ssh, ui};

/// A terminal dashboard for your Mac and its tailnet.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Config file (default ~/.config/homelab/config.toml).
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    /// Render a fixed data set instead of live data (development builds).
    #[cfg(feature = "dev-fixtures")]
    #[arg(long, global = true, value_enum)]
    data: Option<homelab::fixtures::DataSet>,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run collectors and the SSH dashboard server (what launchd runs).
    Serve,
    /// Print one snapshot as JSON after collecting for a few seconds.
    Snapshot {
        #[arg(long, default_value_t = 4)]
        wait: u64,
    },
    /// Draw the dashboard once to stdout as plain text (for docs and tests).
    Render {
        #[arg(long, default_value_t = 120)]
        width: u16,
        #[arg(long, default_value_t = 40)]
        height: u16,
        #[arg(long)]
        light: bool,
        #[arg(long)]
        wait: Option<u64>,
        /// Emit cells (symbol, fg, bg) as JSON instead of text, for screenshots.
        #[arg(long)]
        cells: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let cfg = Config::load(&cli.config.clone().unwrap_or_else(Config::default_path))?;
    let snapshots = {
        #[cfg(feature = "dev-fixtures")]
        if let Some(set) = cli.data {
            let (tx, rx) = tokio::sync::watch::channel(std::sync::Arc::new(homelab::fixtures::load(set)));
            std::mem::forget(tx); // fixed data: keep the channel open forever
            rx
        } else {
            hub::spawn(cfg.clone())
        }
        #[cfg(not(feature = "dev-fixtures"))]
        hub::spawn(cfg.clone())
    };
    match cli.cmd {
        None => local::run(snapshots).await,
        Some(Cmd::Serve) => ssh::serve(&cfg.ssh, snapshots).await,
        Some(Cmd::Snapshot { wait }) => {
            tokio::time::sleep(Duration::from_secs(wait)).await;
            println!("{}", serde_json::to_string_pretty(&**snapshots.borrow())?);
            Ok(())
        }
        Some(Cmd::Render {
            width,
            height,
            light,
            wait,
            cells,
        }) => {
            if let Some(w) = wait {
                tokio::time::sleep(Duration::from_secs(w)).await;
            }
            let mut st = ui::UiState::default();
            if light {
                st.mode = ui::theme::Mode::Light;
            }
            let area = ratatui::layout::Rect::new(0, 0, width, height);
            let mut buf = ratatui::buffer::Buffer::empty(area);
            ui::render(area, &mut buf, &snapshots.borrow(), &st);
            if cells {
                println!("{}", ui::to_cells_json(&buf));
            } else {
                print!("{}", ui::to_text(&buf));
            }
            Ok(())
        }
    }
}
