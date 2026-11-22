//! Collectors feed one hub; the hub is the only writer of the snapshot and
//! publishes it on a `watch` channel that every view (local TUI, each SSH
//! session, `snapshot --json`) reads.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, watch};
use tokio::task::JoinSet;

use crate::collect::health::{Prober, Tracker, resolve};
use crate::collect::host::HostSampler;
use crate::collect::tailscale;
use crate::config::Config;
use crate::model::*;

pub fn now() -> Millis {
    chrono::Utc::now().timestamp_millis()
}

#[derive(Debug)]
pub enum Update {
    Host(HostMetrics),
    Tailnet(Tailnet),
    Latency { id: String, latency: Latency },
    Services(Vec<ServiceStatus>),
}

/// Apply one collector update. Pure, so the merge rules are testable.
pub fn apply(snap: &mut Snapshot, update: Update, at: Millis) {
    snap.at = at;
    match update {
        Update::Host(h) => {
            push_ring(&mut snap.history.cpu, h.cpu_pct, CPU_HISTORY);
            push_ring(&mut snap.history.mem, h.mem.ratio() * 100.0, CPU_HISTORY);
            snap.host = Some(h);
        }
        Update::Tailnet(mut next) => {
            // `status` knows nothing about latency: carry the last ping over.
            if let Tailnet::Ready { devices, .. } = &mut next {
                let old: HashMap<&str, &Latency> = snap
                    .devices()
                    .iter()
                    .filter_map(|d| match &d.presence {
                        Presence::Online { latency } => Some((d.id.as_str(), latency)),
                        Presence::Offline { .. } => None,
                    })
                    .collect();
                for d in devices.iter_mut() {
                    if let (Presence::Online { latency }, Some(prev)) = (&mut d.presence, old.get(d.id.as_str())) {
                        *latency = (*prev).clone();
                    }
                }
            }
            snap.tailnet = next;
        }
        Update::Latency { id, latency } => {
            if let Tailnet::Ready { devices, .. } = &mut snap.tailnet
                && let Some(d) = devices.iter_mut().find(|d| d.id == id)
                && let Presence::Online { latency: l } = &mut d.presence
            {
                *l = latency.clone();
            }
            if let Some(ms) = latency.ms() {
                push_ring(snap.history.latency.entry(id).or_default(), ms, LATENCY_HISTORY);
            }
        }
        Update::Services(s) => snap.services = s,
    }
}

/// Start every collector and the hub. Returns the snapshot stream.
pub fn spawn(cfg: Config) -> watch::Receiver<Arc<Snapshot>> {
    let (tx, mut rx) = mpsc::channel::<Update>(64);
    let initial = Snapshot {
        at: now(),
        services: resolve(&cfg.services, &trackers(&cfg)),
        ..Default::default()
    };
    let (out, snapshots) = watch::channel(Arc::new(initial));

    // The hub: sole writer.
    tokio::spawn(async move {
        let mut snap = (**out.borrow()).clone();
        while let Some(u) = rx.recv().await {
            apply(&mut snap, u, now());
            if out.send(Arc::new(snap.clone())).is_err() {
                break;
            }
        }
    });

    // Host metrics every 2 s (blocking syscalls → spawn_blocking).
    let host_tx = tx.clone();
    tokio::spawn(async move {
        let Ok(mut sampler) = HostSampler::new() else { return };
        let mut tick = tokio::time::interval(Duration::from_secs(2));
        loop {
            tick.tick().await;
            let (s, m) = tokio::task::spawn_blocking(move || {
                let m = sampler.sample();
                (sampler, m)
            })
            .await
            .expect("host sampler");
            sampler = s;
            if host_tx.send(Update::Host(m)).await.is_err() {
                break;
            }
        }
    });

    // Tailscale status every 10 s.
    let bin = cfg.tailscale.bin.clone();
    let ts_tx = tx.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(10));
        loop {
            tick.tick().await;
            if ts_tx.send(Update::Tailnet(tailscale::status(&bin).await)).await.is_err() {
                break;
            }
        }
    });

    // Pings every 30 s for online peers, at most 3 at once.
    let bin = cfg.tailscale.bin.clone();
    let ping_tx = tx.clone();
    let mut watch_devices = snapshots.clone();
    tokio::spawn(async move {
        // Wait for the first device list instead of pinging nothing.
        let _ = watch_devices.wait_for(|s| matches!(s.tailnet, Tailnet::Ready { .. })).await;
        let mut tick = tokio::time::interval(Duration::from_secs(30));
        loop {
            tick.tick().await;
            let targets: Vec<(String, String)> = watch_devices
                .borrow()
                .devices()
                .iter()
                .filter(|d| !d.is_self && matches!(d.presence, Presence::Online { .. }))
                .map(|d| {
                    (
                        d.id.clone(),
                        if d.dns_name.is_empty() {
                            d.name.clone()
                        } else {
                            d.dns_name.clone()
                        },
                    )
                })
                .collect();
            for chunk in targets.chunks(3) {
                let mut set = JoinSet::new();
                for (id, host) in chunk.iter().cloned() {
                    let bin = bin.clone();
                    set.spawn(async move { (id, tailscale::ping(&bin, &host).await) });
                }
                while let Some(Ok((id, latency))) = set.join_next().await {
                    if ping_tx.send(Update::Latency { id, latency }).await.is_err() {
                        return;
                    }
                }
            }
        }
    });

    // Health probes every 5 s.
    tokio::spawn(async move {
        let prober = Arc::new(Prober::new(cfg.docker_bin.clone()));
        let mut trackers = trackers(&cfg);
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        loop {
            tick.tick().await;
            let mut set = JoinSet::new();
            for s in cfg.services.iter().filter(|s| s.enabled) {
                let (p, id, check) = (prober.clone(), s.id.clone(), s.check.clone());
                set.spawn(async move { (id, p.probe(&check).await) });
            }
            while let Some(Ok((id, probe))) = set.join_next().await {
                trackers.get_mut(&id).expect("tracker").observe(probe, now());
            }
            if tx.send(Update::Services(resolve(&cfg.services, &trackers))).await.is_err() {
                break;
            }
        }
    });

    snapshots
}

fn trackers(cfg: &Config) -> HashMap<String, Tracker> {
    let t = now();
    cfg.services.iter().map(|s| (s.id.clone(), Tracker::new(s.enabled, t))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::tailscale::parse_status;

    #[test]
    fn status_refresh_keeps_last_latency() {
        let mut s = Snapshot::default();
        let status = parse_status(include_str!("../fixtures/tailscale-status.json"));
        apply(&mut s, Update::Tailnet(status.clone()), 1);
        apply(
            &mut s,
            Update::Latency {
                id: "n-rex".into(),
                latency: Latency::Direct { ms: 11.0 },
            },
            2,
        );
        apply(&mut s, Update::Tailnet(status), 3);
        let rex = s.devices().iter().find(|d| d.id == "n-rex").unwrap();
        assert_eq!(
            rex.presence,
            Presence::Online {
                latency: Latency::Direct { ms: 11.0 }
            }
        );
        assert_eq!(s.history.latency["n-rex"].len(), 1);
    }

    #[test]
    fn host_updates_fill_history() {
        let mut s = Snapshot::default();
        let h = crate::fixtures::load(crate::fixtures::DataSet::Demo).host.unwrap();
        for _ in 0..3 {
            apply(&mut s, Update::Host(h.clone()), 1);
        }
        assert_eq!(s.history.cpu.len(), 3);
        assert!(s.host.is_some());
    }

    #[tokio::test]
    async fn hub_publishes_service_states() {
        let cfg = Config::parse(
            "[tailscale]\nbin='/nonexistent/tailscale'\n[[service]]\nid='dead'\nname='Dead'\ncheck={kind='tcp', addr='127.0.0.1:9'}\n",
        )
        .unwrap();
        let mut rx = spawn(cfg);
        let snap = tokio::time::timeout(
            Duration::from_secs(10),
            rx.wait_for(|s| s.services.iter().any(|x| x.state.is_fault()) && matches!(s.tailnet, Tailnet::Error { .. })),
        )
        .await
        .expect("published within 10 s")
        .unwrap()
        .clone();
        assert!(matches!(&snap.tailnet, Tailnet::Error { reason } if reason.contains("can't run")));
    }
}
