//! The dashboard over SSH, and nothing else.
//!
//! Public-key auth against an allowlist file; one dashboard per session; no
//! shell, no exec, no subsystems, no port forwarding. Exposed on the tailnet
//! with `tailscale serve --tcp 2222 tcp://127.0.0.1:2222`.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::cursor::{Hide, Show};
use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::{Terminal, TerminalOptions, Viewport};
use russh::keys::{Algorithm, PrivateKey, PublicKey};
use russh::server::{Auth, ChannelOpenHandle, Handle, Handler, Msg, PublicKeyOffer, Session, VerifiedPublicKey};
use russh::{Channel, ChannelId, MethodKind, MethodSet, Pty};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, watch};

use crate::config::{SshConfig, expand};
use crate::model::Snapshot;
use crate::ui::{self, Action, UiState, state::parse_keys};

/// Serve until the listener fails.
pub async fn serve(cfg: &SshConfig, snapshots: watch::Receiver<Arc<Snapshot>>) -> Result<()> {
    let listener = TcpListener::bind(&cfg.listen)
        .await
        .with_context(|| format!("binding {}", cfg.listen))?;
    eprintln!("homelab: ssh dashboard on {}", listener.local_addr()?);
    serve_on(listener, &expand(&cfg.host_key), &expand(&cfg.authorized_keys), snapshots).await
}

pub async fn serve_on(
    listener: TcpListener,
    host_key: &Path,
    authorized_keys: &Path,
    snapshots: watch::Receiver<Arc<Snapshot>>,
) -> Result<()> {
    use russh::server::Server as _;
    let config = russh::server::Config {
        keys: vec![load_or_create_host_key(host_key)?],
        methods: MethodSet::from(&[MethodKind::PublicKey][..]),
        inactivity_timeout: Some(Duration::from_secs(3600)),
        auth_rejection_time: Duration::from_secs(1),
        auth_rejection_time_initial: Some(Duration::from_secs(0)),
        nodelay: true,
        ..Default::default()
    };
    let mut server = Server {
        snapshots,
        authorized_keys: authorized_keys.to_path_buf(),
    };
    server.run_on_socket(Arc::new(config), &listener).await?;
    Ok(())
}

/// Ed25519 host key, generated once and kept at 0600 so clients see a stable fingerprint.
pub fn load_or_create_host_key(path: &Path) -> Result<PrivateKey> {
    if path.exists() {
        return russh::keys::load_secret_key(path, None).with_context(|| format!("loading host key {}", path.display()));
    }
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).context("generating host key")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let pem = key.to_openssh(russh::keys::ssh_key::LineEnding::LF)?;
    write_private(path, pem.as_bytes())?;
    Ok(key)
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?;
    f.write_all(bytes)?;
    Ok(())
}
#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    Ok(std::fs::write(path, bytes)?)
}

/// Parse an OpenSSH `authorized_keys`-style file. Re-read on every auth, so
/// adding a phone key needs no restart. Lines with options are skipped.
pub fn read_authorized_keys(path: &Path) -> Vec<PublicKey> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| PublicKey::from_openssh(l).ok())
        .collect()
}

#[derive(Clone)]
struct Server {
    snapshots: watch::Receiver<Arc<Snapshot>>,
    authorized_keys: std::path::PathBuf,
}

impl russh::server::Server for Server {
    type Handler = Conn;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Conn {
        Conn {
            server: self.clone(),
            channel: None,
            size: None,
            input: None,
        }
    }
}

enum Input {
    Keys(Vec<Action>),
    Resize(u16, u16),
}

/// One SSH connection. Holds at most one dashboard session.
struct Conn {
    server: Server,
    channel: Option<ChannelId>,
    size: Option<(u16, u16)>,
    input: Option<mpsc::UnboundedSender<Input>>,
}

impl Conn {
    fn allowed(&self, key: &PublicKey) -> bool {
        read_authorized_keys(&self.server.authorized_keys)
            .iter()
            .any(|k| k.key_data() == key.key_data())
    }
}

const NO_EXEC: &str = "homelab: this server only shows the dashboard; shell, exec and forwarding are disabled.\r\n";
const NO_PTY: &str = "homelab: needs a terminal; connect with `ssh -t`.\r\n";

/// Accept a request, explain on stderr why nothing will run, exit 1, close.
fn refuse(session: &mut Session, channel: ChannelId, why: &'static str) -> Result<()> {
    session.channel_success(channel)?;
    session.extended_data(channel, 1, bytes::Bytes::from_static(why.as_bytes()))?;
    session.exit_status_request(channel, 1)?;
    session.eof(channel)?;
    session.close(channel)?;
    Ok(())
}

impl Handler for Conn {
    type Error = anyhow::Error;

    async fn auth_publickey_offered(&mut self, _: &str, offer: PublicKeyOffer<'_>) -> Result<Auth> {
        Ok(if self.allowed(offer.public_key) {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn auth_publickey(&mut self, _: &str, key: VerifiedPublicKey<'_>) -> Result<Auth> {
        Ok(if self.allowed(key.public_key) {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn channel_open_session(&mut self, channel: Channel<Msg>, reply: ChannelOpenHandle, _: &mut Session) -> Result<()> {
        if self.channel.is_some() {
            return Ok(()); // dropping `reply` rejects: one dashboard per connection
        }
        self.channel = Some(channel.id());
        reply.accept().await;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _: &str,
        cols: u32,
        rows: u32,
        _: u32,
        _: u32,
        _: &[(Pty, u32)],
        session: &mut Session,
    ) -> Result<()> {
        // OpenSSH sends 0×0 when its own stdin isn't a terminal (`ssh -tt` in a pipe).
        self.size = Some(if cols == 0 || rows == 0 {
            (80, 24)
        } else {
            (cols.min(u16::MAX as u32) as u16, rows.min(u16::MAX as u32) as u16)
        });
        session.channel_success(channel)?;
        Ok(())
    }

    async fn shell_request(&mut self, channel: ChannelId, session: &mut Session) -> Result<()> {
        let Some((w, h)) = self.size else {
            refuse(session, channel, NO_PTY)?;
            return Ok(());
        };
        session.channel_success(channel)?;
        let (tx, rx) = mpsc::unbounded_channel();
        self.input = Some(tx);
        tokio::spawn(run_session(session.handle(), channel, self.server.snapshots.clone(), rx, (w, h)));
        Ok(())
    }

    async fn exec_request(&mut self, channel: ChannelId, _: &[u8], session: &mut Session) -> Result<()> {
        // Accept the request so the client prints our reason, then exit 1 without running anything.
        refuse(session, channel, NO_EXEC)
    }

    async fn subsystem_request(&mut self, channel: ChannelId, _: &str, session: &mut Session) -> Result<()> {
        session.channel_failure(channel)?;
        session.close(channel)?;
        Ok(())
    }

    async fn data(&mut self, _: ChannelId, data: &[u8], _: &mut Session) -> Result<()> {
        if let Some(tx) = &self.input {
            let _ = tx.send(Input::Keys(parse_keys(data)));
        }
        Ok(())
    }

    async fn window_change_request(&mut self, _: ChannelId, cols: u32, rows: u32, _: u32, _: u32, _: &mut Session) -> Result<()> {
        let (w, h) = (cols.min(u16::MAX as u32) as u16, rows.min(u16::MAX as u32) as u16);
        self.size = Some((w, h));
        if let Some(tx) = &self.input {
            let _ = tx.send(Input::Resize(w, h));
        }
        Ok(())
    }
}

/// Bytes ratatui writes are buffered, then shipped to the channel on flush.
struct ChannelWriter {
    tx: mpsc::UnboundedSender<Vec<u8>>,
    sink: Vec<u8>,
}

impl std::io::Write for ChannelWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.sink.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        if self.sink.is_empty() {
            return Ok(());
        }
        self.tx
            .send(std::mem::take(&mut self.sink))
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::BrokenPipe, e.to_string()))
    }
}

/// Owns this session's terminal and UI state; nothing here is shared.
async fn run_session(
    handle: Handle,
    channel: ChannelId,
    mut snapshots: watch::Receiver<Arc<Snapshot>>,
    mut input: mpsc::UnboundedReceiver<Input>,
    (w, h): (u16, u16),
) {
    let (tx, mut out) = mpsc::unbounded_channel::<Vec<u8>>();
    let sender = handle.clone();
    let pump = tokio::spawn(async move {
        while let Some(bytes) = out.recv().await {
            if sender.data(channel, bytes::Bytes::from(bytes)).await.is_err() {
                break;
            }
        }
    });
    let writer = ChannelWriter { tx, sink: Vec::new() };
    let Ok(mut term) = Terminal::with_options(
        CrosstermBackend::new(writer),
        TerminalOptions {
            viewport: Viewport::Fixed(Rect::new(0, 0, w, h)),
        },
    ) else {
        return;
    };
    let _ = execute!(term.backend_mut(), EnterAlternateScreen, Hide);
    let mut ui = UiState::default();
    loop {
        let snap = snapshots.borrow_and_update().clone();
        if term.draw(|f| ui::render(f.area(), f.buffer_mut(), &snap, &ui)).is_err() {
            break;
        }
        tokio::select! {
            changed = snapshots.changed() => if changed.is_err() { break },
            msg = input.recv() => match msg {
                None => break,
                Some(Input::Resize(w, h)) => { let _ = term.resize(Rect::new(0, 0, w, h)); }
                Some(Input::Keys(keys)) => {
                    let peers = snap.devices().iter().filter(|d| !d.is_self).count();
                    for k in keys { ui.apply(k, peers, snap.services.len()); }
                }
            },
        }
        if ui.quit {
            break;
        }
    }
    let _ = execute!(term.backend_mut(), Show, LeaveAlternateScreen);
    drop(term); // closes the byte channel so the pump drains and exits
    let _ = pump.await;
    let _ = handle.exit_status_request(channel, 0).await;
    let _ = handle.eof(channel).await;
    let _ = handle.close(channel).await;
}
