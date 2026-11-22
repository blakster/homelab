//! End to end: a real russh client against `serve_on`, asserting what a
//! visitor gets. Unknown keys are refused, a pty session gets the dashboard,
//! exec gets a refusal, and two sessions don't share state.
#![cfg(feature = "dev-fixtures")]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use homelab::fixtures::{DataSet, load};
use russh::ChannelMsg;
use russh::client::{self, Handle};
use russh::keys::{Algorithm, PrivateKey, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::time::timeout;

struct Trusting;
impl client::Handler for Trusting {
    type Error = russh::Error;
    async fn check_server_key(&mut self, _: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

fn key() -> PrivateKey {
    PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap()
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("homelab-ssh-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Start a server that trusts `allowed`; returns its port.
async fn start(name: &str, allowed: &PrivateKey) -> u16 {
    let dir = scratch(name);
    let ak = dir.join("authorized_keys");
    std::fs::write(&ak, allowed.public_key().to_openssh().unwrap()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (_tx, rx) = watch::channel(Arc::new(load(DataSet::Demo)));
    tokio::spawn(async move {
        let _keep = _tx;
        homelab::ssh::serve_on(listener, &dir.join("host_ed25519"), &ak, rx).await.unwrap();
    });
    port
}

async fn connect(port: u16, key: &PrivateKey) -> (Handle<Trusting>, bool) {
    let mut h = client::connect(Arc::new(client::Config::default()), ("127.0.0.1", port), Trusting)
        .await
        .unwrap();
    let ok = h
        .authenticate_publickey("vansh", PrivateKeyWithHashAlg::new(Arc::new(key.clone()), None))
        .await
        .unwrap()
        .success();
    (h, ok)
}

/// Collect stdout until `want` shows up (or 5 s pass).
async fn read_until(ch: &mut russh::Channel<client::Msg>, want: &str) -> String {
    let mut out = Vec::new();
    let _ = timeout(Duration::from_secs(5), async {
        while let Some(msg) = ch.wait().await {
            if let ChannelMsg::Data { data } = msg {
                out.extend_from_slice(&data);
                if String::from_utf8_lossy(&out).contains(want) {
                    break;
                }
            }
        }
    })
    .await;
    String::from_utf8_lossy(&out).into_owned()
}

#[tokio::test]
async fn unknown_keys_are_refused() {
    let port = start("refuse", &key()).await;
    let (_h, ok) = connect(port, &key()).await;
    assert!(!ok);
}

#[tokio::test]
async fn pty_shell_gets_the_dashboard() {
    let me = key();
    let port = start("shell", &me).await;
    let (h, ok) = connect(port, &me).await;
    assert!(ok);
    let mut ch = h.channel_open_session().await.unwrap();
    ch.request_pty(false, "xterm-256color", 120, 40, 0, 0, &[]).await.unwrap();
    ch.request_shell(false).await.unwrap();
    let screen = read_until(&mut ch, "Services").await;
    assert!(screen.contains("\x1b[?1049h"), "alternate screen first");
    for want in ["Machines", "Services", "Pulse"] {
        assert!(screen.contains(want), "missing {want}");
    }
    ch.data(&b"q"[..]).await.unwrap();
    let rest = read_until(&mut ch, "\x1b[?1049l").await;
    assert!(rest.contains("\x1b[?1049l"), "q restores the terminal");
}

#[tokio::test]
async fn exec_is_refused_with_a_reason() {
    let me = key();
    let port = start("exec", &me).await;
    let (h, _) = connect(port, &me).await;
    let mut ch = h.channel_open_session().await.unwrap();
    ch.exec(true, "cat /etc/passwd").await.unwrap();
    let (mut stderr, mut status) = (String::new(), None);
    let _ = timeout(Duration::from_secs(5), async {
        while let Some(msg) = ch.wait().await {
            match msg {
                ChannelMsg::ExtendedData { data, ext: 1 } => stderr.push_str(&String::from_utf8_lossy(&data)),
                ChannelMsg::Data { data } => panic!("exec produced stdout: {:?}", String::from_utf8_lossy(&data)),
                ChannelMsg::ExitStatus { exit_status } => status = Some(exit_status),
                ChannelMsg::Close => break,
                _ => {}
            }
        }
    })
    .await;
    assert_eq!(status, Some(1));
    assert!(stderr.contains("only shows the dashboard"), "{stderr}");
}

#[tokio::test]
async fn shell_without_a_pty_is_told_why() {
    let me = key();
    let port = start("nopty", &me).await;
    let (h, _) = connect(port, &me).await;
    let mut ch = h.channel_open_session().await.unwrap();
    ch.request_shell(false).await.unwrap();
    let mut all = String::new();
    let _ = timeout(Duration::from_secs(5), async {
        while let Some(msg) = ch.wait().await {
            match msg {
                ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } => all.push_str(&String::from_utf8_lossy(&data)),
                ChannelMsg::Close => break,
                _ => {}
            }
        }
    })
    .await;
    assert!(all.contains("needs a terminal"), "{all}");
}

#[tokio::test]
async fn sessions_are_independent() {
    let me = key();
    let port = start("two", &me).await;
    let (a, _) = connect(port, &me).await;
    let (b, _) = connect(port, &me).await;
    let mut ca = a.channel_open_session().await.unwrap();
    let mut cb = b.channel_open_session().await.unwrap();
    ca.request_pty(false, "xterm-256color", 120, 40, 0, 0, &[]).await.unwrap();
    cb.request_pty(false, "xterm-256color", 80, 24, 0, 0, &[]).await.unwrap();
    ca.request_shell(false).await.unwrap();
    cb.request_shell(false).await.unwrap();
    read_until(&mut ca, "Services").await;
    read_until(&mut cb, "Services").await;
    ca.data(&b"?"[..]).await.unwrap();
    let a_screen = read_until(&mut ca, "plain mode").await;
    assert!(a_screen.contains("plain mode"), "a opened help");
    cb.data(&b"j"[..]).await.unwrap();
    let b_screen = read_until(&mut cb, "plain mode").await;
    assert!(!b_screen.contains("plain mode"), "b never saw a's help overlay");
}

#[tokio::test]
async fn session_follows_the_client_terminal_background() {
    let me = key();
    let port = start("theme", &me).await;
    let (h, ok) = connect(port, &me).await;
    assert!(ok);
    let mut ch = h.channel_open_session().await.unwrap();
    ch.request_pty(false, "xterm-ghostty", 120, 40, 0, 0, &[]).await.unwrap();
    ch.request_shell(false).await.unwrap();
    // Answer the server's OSC 11 query with a white background, as a terminal would.
    let mut answered = false;
    let mut screen = String::new();
    let _ = timeout(Duration::from_secs(5), async {
        while let Some(msg) = ch.wait().await {
            if let ChannelMsg::Data { data } = msg {
                screen.push_str(&String::from_utf8_lossy(&data));
                if !answered && data.windows(5).any(|w| w == b"]11;?") {
                    ch.data(&b"\x1b]11;rgb:ffff/ffff/ffff\x07"[..]).await.unwrap();
                    answered = true;
                }
                if screen.contains("38;2;24;24;27") {
                    break; // light palette ink made it into a frame
                }
            }
        }
    })
    .await;
    assert!(answered, "server queried the terminal background");
    assert!(screen.contains("38;2;24;24;27"), "light palette after OSC 11 reply");
}

#[tokio::test]
async fn zero_sized_pty_falls_back_to_80x24() {
    // OpenSSH sends 0×0 for `ssh -tt` when its stdin is a pipe.
    let me = key();
    let port = start("zero", &me).await;
    let (h, _) = connect(port, &me).await;
    let mut ch = h.channel_open_session().await.unwrap();
    ch.request_pty(false, "xterm", 0, 0, 0, 0, &[]).await.unwrap();
    ch.request_shell(false).await.unwrap();
    let screen = read_until(&mut ch, "Services").await;
    assert!(screen.contains("Services"), "{} bytes", screen.len());
}
