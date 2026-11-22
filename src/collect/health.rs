//! Service probes and the up/down state table.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde::Deserialize;
use tokio::net::TcpStream;
use tokio::process::Command;

use crate::config::{Check, ServiceConfig};
use crate::model::{Millis, ServiceState, ServiceStatus};

const TIMEOUT: Duration = Duration::from_secs(3);
/// Consecutive failures before an `Up` service is reported down (flap guard).
const FAILS_TO_DOWN: u8 = 2;

/// One probe's raw result: latency in ms, or why it failed.
pub type Probe = Result<u32, String>;

pub struct Prober {
    http: reqwest::Client,
    docker: String,
}

impl Prober {
    pub fn new(docker_bin: Option<String>) -> Self {
        let docker = docker_bin.unwrap_or_else(|| {
            ["/usr/local/bin/docker", "/opt/homebrew/bin/docker", "/usr/bin/docker"]
                .into_iter()
                .find(|p| std::path::Path::new(p).exists())
                .unwrap_or("docker")
                .to_string()
        });
        let http = reqwest::Client::builder().timeout(TIMEOUT).build().expect("http client");
        Self { http, docker }
    }

    pub async fn probe(&self, check: &Check) -> Probe {
        let start = Instant::now();
        let ms = || start.elapsed().as_millis().min(u32::MAX as u128) as u32;
        match check {
            Check::Http { url, expect } => {
                let resp = self.http.get(url).send().await.map_err(|e| http_reason(&e))?;
                let code = resp.status().as_u16();
                let ok = match expect {
                    Some(want) => code == *want,
                    None => resp.status().is_success(),
                };
                if ok { Ok(ms()) } else { Err(format!("HTTP {code}")) }
            }
            Check::Tcp { addr } => match tokio::time::timeout(TIMEOUT, TcpStream::connect(addr)).await {
                Err(_) => Err(format!("timeout connecting to {addr}")),
                Ok(Err(e)) => Err(io_reason(&e, addr)),
                Ok(Ok(_)) => Ok(ms()),
            },
            Check::DockerDaemon => {
                self.docker(&["info", "--format", "{{.ServerVersion}}"])
                    .await
                    .map_err(|_| "docker daemon unreachable".to_string())?;
                Ok(ms())
            }
            Check::Docker { container } => {
                let out = self
                    .docker(&["inspect", "--format", "{{json .State}}", container])
                    .await
                    .map_err(|e| {
                        if e.contains("No such") {
                            format!("no container {container}")
                        } else {
                            "docker daemon unreachable".into()
                        }
                    })?;
                container_state(&out)?;
                Ok(ms())
            }
        }
    }

    async fn docker(&self, args: &[&str]) -> Result<String, String> {
        let out = tokio::time::timeout(TIMEOUT, Command::new(&self.docker).args(args).kill_on_drop(true).output())
            .await
            .map_err(|_| "docker timed out".to_string())?
            .map_err(|e| format!("can't run docker: {e}"))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).into_owned())
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ContainerState {
    running: bool,
    status: String,
    health: Option<Health>,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Health {
    status: String,
}

/// `docker inspect --format '{{json .State}}'` → Ok if running and not unhealthy.
pub fn container_state(json: &str) -> Result<(), String> {
    let s: ContainerState = serde_json::from_str(json.trim()).map_err(|e| format!("unreadable docker state: {e}"))?;
    if !s.running {
        return Err(format!("container {}", s.status));
    }
    match s.health.map(|h| h.status) {
        Some(h) if h == "unhealthy" => Err("container unhealthy".into()),
        _ => Ok(()),
    }
}

fn http_reason(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        return "timeout".into();
    }
    if e.is_connect() {
        let port = e
            .url()
            .and_then(|u| u.port_or_known_default())
            .map(|p| format!(" :{p}"))
            .unwrap_or_default();
        return format!("connection refused{port}");
    }
    e.to_string()
}

fn io_reason(e: &std::io::Error, addr: &str) -> String {
    match e.kind() {
        std::io::ErrorKind::ConnectionRefused => format!("connection refused {addr}"),
        _ => format!("{e} ({addr})"),
    }
}

/// Per-service memory between probes. One owner: the health loop.
#[derive(Clone, Debug)]
pub struct Tracker {
    state: ServiceState,
    fails: u8,
    since: Millis,
    checked_at: Option<Millis>,
}

impl Tracker {
    pub fn new(enabled: bool, now: Millis) -> Self {
        let state = if enabled { ServiceState::Unknown } else { ServiceState::Off };
        Self {
            state,
            fails: 0,
            since: now,
            checked_at: None,
        }
    }

    /// The transition table. Up→Down needs `FAILS_TO_DOWN` failures in a row;
    /// anything→Up needs one success; Unknown→Down is immediate.
    pub fn observe(&mut self, probe: Probe, now: Millis) {
        self.checked_at = Some(now);
        let next = match (&self.state, probe) {
            (ServiceState::Off, _) => return,
            (_, Ok(ms)) => {
                self.fails = 0;
                ServiceState::Up { ms }
            }
            (ServiceState::Up { ms }, Err(_)) if self.fails + 1 < FAILS_TO_DOWN => {
                self.fails += 1;
                ServiceState::Up { ms: *ms }
            }
            (_, Err(reason)) => {
                self.fails = self.fails.saturating_add(1);
                ServiceState::Down { reason }
            }
        };
        if !next.same_kind(&self.state) {
            self.since = now;
        }
        self.state = next;
    }
}

/// Turn trackers into statuses, marking a down service `Blocked` when one of
/// its dependencies is not up. Walks dependencies so the root cause is named.
pub fn resolve(services: &[ServiceConfig], trackers: &HashMap<String, Tracker>) -> Vec<ServiceStatus> {
    let up = |id: &str| matches!(trackers.get(id).map(|t| &t.state), Some(ServiceState::Up { .. }));
    let root_cause = |start: &ServiceConfig| -> Option<String> {
        let by_id: HashMap<&str, &ServiceConfig> = services.iter().map(|s| (s.id.as_str(), s)).collect();
        let mut culprit = None;
        let mut frontier: Vec<&str> = start.depends_on.iter().map(String::as_str).collect();
        let mut seen = std::collections::HashSet::new();
        while let Some(id) = frontier.pop() {
            if !seen.insert(id) || up(id) {
                continue;
            }
            culprit = Some(id.to_string()); // deepest not-up dependency wins
            if let Some(dep) = by_id.get(id) {
                frontier.extend(dep.depends_on.iter().map(String::as_str));
            }
        }
        culprit
    };
    services
        .iter()
        .map(|cfg| {
            let t = &trackers[&cfg.id];
            let state = match &t.state {
                ServiceState::Down { .. } => match root_cause(cfg) {
                    Some(by) => ServiceState::Blocked { by },
                    None => t.state.clone(),
                },
                s => s.clone(),
            };
            ServiceStatus {
                id: cfg.id.clone(),
                name: cfg.name.clone(),
                url: cfg.url.clone(),
                state,
                checked_at: t.checked_at,
                since: t.since,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn up_needs_two_failures_to_go_down() {
        let mut t = Tracker::new(true, 0);
        t.observe(Ok(5), 1);
        assert_eq!(t.state, ServiceState::Up { ms: 5 });
        t.observe(Err("HTTP 500".into()), 2);
        assert!(matches!(t.state, ServiceState::Up { .. }), "one failure is a flap");
        assert_eq!(t.since, 1);
        t.observe(Err("HTTP 500".into()), 3);
        assert_eq!(t.state, ServiceState::Down { reason: "HTTP 500".into() });
        assert_eq!(t.since, 3);
        t.observe(Ok(4), 4);
        assert_eq!(t.state, ServiceState::Up { ms: 4 });
        assert_eq!(t.since, 4);
    }

    #[test]
    fn unknown_goes_down_immediately() {
        let mut t = Tracker::new(true, 0);
        t.observe(Err("connection refused :3001".into()), 1);
        assert!(matches!(t.state, ServiceState::Down { .. }));
    }

    #[test]
    fn disabled_is_off_and_never_changes() {
        let mut t = Tracker::new(false, 0);
        t.observe(Err("x".into()), 1);
        t.observe(Ok(1), 2);
        assert_eq!(t.state, ServiceState::Off);
    }

    #[test]
    fn down_with_down_dependency_is_blocked_by_root_cause() {
        let cfg = Config::parse(include_str!("../../config.example.toml")).unwrap();
        let mut tr: HashMap<String, Tracker> = cfg.services.iter().map(|s| (s.id.clone(), Tracker::new(s.enabled, 0))).collect();
        for id in ["orbstack", "pulse-db", "pulse"] {
            tr.get_mut(id).unwrap().observe(Err("refused".into()), 1);
        }
        let out = resolve(&cfg.services, &tr);
        let get = |id: &str| out.iter().find(|s| s.id == id).unwrap().state.clone();
        assert_eq!(get("orbstack"), ServiceState::Down { reason: "refused".into() });
        assert_eq!(get("pulse-db"), ServiceState::Blocked { by: "orbstack".into() });
        assert_eq!(get("pulse"), ServiceState::Blocked { by: "orbstack".into() });
        assert_eq!(get("air-health"), ServiceState::Off);
        assert!(!get("air-health").is_fault());
    }

    #[test]
    fn down_with_healthy_dependencies_stays_down() {
        let cfg = Config::parse(include_str!("../../config.example.toml")).unwrap();
        let mut tr: HashMap<String, Tracker> = cfg.services.iter().map(|s| (s.id.clone(), Tracker::new(s.enabled, 0))).collect();
        tr.get_mut("orbstack").unwrap().observe(Ok(3), 1);
        tr.get_mut("pulse-db").unwrap().observe(Ok(3), 1);
        tr.get_mut("pulse").unwrap().observe(Err("connection refused :3001".into()), 1);
        let out = resolve(&cfg.services, &tr);
        let pulse = out.iter().find(|s| s.id == "pulse").unwrap();
        assert_eq!(
            pulse.state,
            ServiceState::Down {
                reason: "connection refused :3001".into()
            }
        );
    }

    #[test]
    fn container_states() {
        assert!(container_state(r#"{"Running":true,"Status":"running","Health":{"Status":"healthy"}}"#).is_ok());
        assert_eq!(
            container_state(r#"{"Running":false,"Status":"exited"}"#),
            Err("container exited".into())
        );
        assert_eq!(
            container_state(r#"{"Running":true,"Status":"running","Health":{"Status":"unhealthy"}}"#),
            Err("container unhealthy".into())
        );
    }

    #[tokio::test]
    async fn http_probe_against_a_real_listener() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut n = 0;
            loop {
                let (mut s, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 1024];
                let _ = s.read(&mut buf).await;
                let code = if n == 0 { "200 OK" } else { "500 Internal Server Error" };
                n += 1;
                let _ = s
                    .write_all(format!("HTTP/1.1 {code}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").as_bytes())
                    .await;
            }
        });
        let p = Prober::new(None);
        let check = Check::Http {
            url: format!("http://{addr}/healthz"),
            expect: None,
        };
        assert!(p.probe(&check).await.is_ok());
        assert_eq!(p.probe(&check).await, Err("HTTP 500".into()));
        let refused = Check::Http {
            url: "http://127.0.0.1:9/healthz".into(),
            expect: None,
        };
        assert!(p.probe(&refused).await.unwrap_err().starts_with("connection refused"));
        let tcp = Check::Tcp { addr: addr.to_string() };
        assert!(p.probe(&tcp).await.is_ok());
    }
}
