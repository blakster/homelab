//! Tailscale devices (`status --json`) and latency (`ping -c 1`).
//!
//! We shell out to the CLI: the standalone macOS app's local API needs a token
//! from its group container, and the CLI output is a stable contract.

use std::collections::HashMap;
use std::time::Duration;

use chrono::DateTime;
use serde::Deserialize;
use tokio::process::Command;

use crate::model::{Device, Latency, Presence, Tailnet};

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct StatusJson {
    backend_state: String,
    current_tailnet: Option<TailnetJson>,
    #[serde(rename = "Self")]
    self_: Option<PeerJson>,
    #[serde(default)]
    peer: HashMap<String, PeerJson>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct TailnetJson {
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PeerJson {
    #[serde(rename = "ID")]
    id: String,
    host_name: String,
    #[serde(rename = "DNSName", default)]
    dns_name: String,
    #[serde(rename = "OS", default)]
    os: String,
    #[serde(default)]
    online: bool,
    #[serde(default)]
    last_seen: Option<String>,
}

/// Parse `tailscale status --json`. Online peers start `Unmeasured`; the ping
/// loop fills latency in.
pub fn parse_status(json: &str) -> Tailnet {
    let s: StatusJson = match serde_json::from_str(json) {
        Ok(s) => s,
        Err(e) => {
            // Not JSON: usually the CLI telling us something ("The Tailscale GUI failed to start…").
            let said = json.lines().map(str::trim).find(|l| !l.is_empty());
            return Tailnet::Error {
                reason: match said {
                    Some(line) if !line.starts_with('{') => format!("tailscale said: {line}"),
                    _ => format!("unreadable tailscale status: {e}"),
                },
            };
        }
    };
    if s.backend_state != "Running" {
        return Tailnet::Error {
            reason: format!("tailscale is {}", s.backend_state),
        };
    }
    let mut devices: Vec<Device> = s.self_.into_iter().map(|p| device(p, true)).collect();
    let mut peers: Vec<Device> = s.peer.into_values().map(|p| device(p, false)).collect();
    // Online first, then by name: stable order so rows don't jump between ticks.
    peers.sort_by(|a, b| {
        let on = |d: &Device| matches!(d.presence, Presence::Online { .. });
        on(b).cmp(&on(a)).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    devices.extend(peers);
    Tailnet::Ready {
        tailnet: s.current_tailnet.map(|t| t.name).unwrap_or_default(),
        devices,
    }
}

fn device(p: PeerJson, is_self: bool) -> Device {
    let presence = if p.online || is_self {
        Presence::Online {
            latency: Latency::Unmeasured,
        }
    } else {
        Presence::Offline {
            last_seen: p.last_seen.as_deref().and_then(parse_last_seen),
        }
    };
    let dns_name = p.dns_name.trim_end_matches('.').to_string();
    // HostName isn't unique (two machines both called `vansh-box`); the MagicDNS
    // label is, and it's what `tailscale status` prints. Self keeps its pretty name.
    let label = dns_name.split('.').next().filter(|l| !l.is_empty());
    let name = match label {
        Some(l) if !is_self => l.to_string(),
        _ => p.host_name,
    };
    Device {
        id: p.id,
        name,
        dns_name,
        os: p.os,
        is_self,
        presence,
    }
}

/// Tailscale writes `0001-01-01T00:00:00Z` for "never".
fn parse_last_seen(s: &str) -> Option<i64> {
    let t = DateTime::parse_from_rfc3339(s).ok()?;
    (t.timestamp() > 0).then(|| t.timestamp_millis())
}

/// Parse one line of `tailscale ping` output.
///
/// `pong from host (100.x) via 192.168.1.20:65212 in 11ms` → Direct
/// `pong from host (100.x) via DERP(bom) in 182ms` → Relay
pub fn parse_ping(out: &str) -> Latency {
    let Some(line) = out.lines().find(|l| l.starts_with("pong from")) else {
        return Latency::Unmeasured;
    };
    let Some((_, rest)) = line.split_once(" via ") else {
        return Latency::Unmeasured;
    };
    let Some((path, ms)) = rest.rsplit_once(" in ") else {
        return Latency::Unmeasured;
    };
    let Ok(ms) = ms.trim().trim_end_matches("ms").parse::<f32>() else {
        return Latency::Unmeasured;
    };
    match path.strip_prefix("DERP(").and_then(|r| r.strip_suffix(')')) {
        Some(region) => Latency::Relay {
            ms,
            region: region.to_string(),
        },
        None => Latency::Direct { ms },
    }
}

pub async fn status(bin: &str) -> Tailnet {
    match run(bin, &["status", "--json"], Duration::from_secs(5)).await {
        Ok(out) => parse_status(&out),
        Err(e) => Tailnet::Error { reason: e },
    }
}

pub async fn ping(bin: &str, host: &str) -> Latency {
    // The CLI exits non-zero on timeout; whatever it printed is still parseable.
    match run(bin, &["ping", "-c", "1", "--timeout", "2s", host], Duration::from_secs(4)).await {
        Ok(out) | Err(out) => parse_ping(&out),
    }
}

/// Ok(stdout) on success; Err(reason or partial stdout) otherwise.
async fn run(bin: &str, args: &[&str], limit: Duration) -> Result<String, String> {
    let mut cmd = Command::new(bin);
    // The macOS app-bundle binary only acts as a CLI when TERM is set; under
    // launchd it isn't, and the binary tries to launch the GUI instead.
    if std::env::var_os("TERM").is_none() {
        cmd.env("TERM", "dumb");
    }
    let child = cmd.args(args).kill_on_drop(true).output();
    match tokio::time::timeout(limit, child).await {
        Err(_) => Err(format!("`{bin} {}` timed out", args[0])),
        Ok(Err(e)) => Err(format!("can't run {bin}: {e}")),
        Ok(Ok(o)) if o.status.success() => Ok(String::from_utf8_lossy(&o.stdout).into_owned()),
        Ok(Ok(o)) => Err(String::from_utf8_lossy(&o.stdout).into_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_fixture_yields_devices_with_presence() {
        let Tailnet::Ready { tailnet, devices } = parse_status(include_str!("../../fixtures/tailscale-status.json")) else {
            panic!("expected ready");
        };
        assert_eq!(tailnet, "blakster.github");
        assert_eq!(devices.len(), 5);
        assert!(devices[0].is_self);
        assert_eq!(devices[0].name, "Vansh’s MacBook Air (5)");
        // online peer sorts first among peers
        assert_eq!(devices[1].name, "vanshs-macbook-air-5-rex");
        assert!(matches!(devices[1].presence, Presence::Online { .. }));
        assert_eq!(devices[1].dns_name, "vanshs-macbook-air-5-rex.tail1234.ts.net");
        let phone = devices.iter().find(|d| d.os == "android").unwrap();
        assert!(matches!(phone.presence, Presence::Offline { last_seen: Some(_) }));
        let never = devices.iter().find(|d| d.name == "never-seen").unwrap();
        assert_eq!(never.presence, Presence::Offline { last_seen: None });
    }

    #[test]
    fn stopped_backend_is_an_error() {
        let t = parse_status(r#"{"BackendState":"Stopped","Peer":{}}"#);
        assert!(matches!(t, Tailnet::Error { reason } if reason.contains("Stopped")));
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(matches!(parse_status("not json"), Tailnet::Error { .. }));
    }

    #[test]
    fn peers_are_named_by_their_unique_magicdns_label() {
        let json = r#"{"BackendState":"Running","Peer":{
            "a":{"ID":"a","HostName":"vansh-box","DNSName":"vansh-box.t.ts.net.","OS":"linux","Online":true},
            "b":{"ID":"b","HostName":"vansh-box","DNSName":"vansh-box-1.t.ts.net.","OS":"linux","Online":true}}}"#;
        let Tailnet::Ready { devices, .. } = parse_status(json) else {
            panic!()
        };
        let names: Vec<&str> = devices.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["vansh-box", "vansh-box-1"]);
    }

    #[test]
    fn non_json_output_is_quoted_in_the_reason() {
        let t = parse_status("The Tailscale GUI failed to start: CLIError 3.\n");
        assert_eq!(
            t,
            Tailnet::Error {
                reason: "tailscale said: The Tailscale GUI failed to start: CLIError 3.".into()
            }
        );
    }

    #[test]
    fn ping_direct_relay_timeout() {
        assert_eq!(
            parse_ping(include_str!("../../fixtures/ping-direct.txt")),
            Latency::Direct { ms: 11.0 }
        );
        assert_eq!(
            parse_ping(include_str!("../../fixtures/ping-relay.txt")),
            Latency::Relay {
                ms: 182.0,
                region: "bom".into()
            }
        );
        assert_eq!(parse_ping(include_str!("../../fixtures/ping-timeout.txt")), Latency::Unmeasured);
    }
}
