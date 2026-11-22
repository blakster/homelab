//! Fixed data sets for development and tests (`--data <set>`), loaded through
//! the same boundary as live data: a `Snapshot`. "now" is pinned so relative
//! times never drift. Compiled only with the `dev-fixtures` feature or in tests.

use std::collections::VecDeque;

use clap::ValueEnum;

use crate::model::*;

/// 2026-11-22T07:30:00Z, the instant every fixture is relative to.
pub const NOW: Millis = 1_795_332_600_000;
const MIN: Millis = 60_000;
const HOUR: Millis = 60 * MIN;
const DAY: Millis = 24 * HOUR;
const GB: u64 = 1 << 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum DataSet {
    Demo,
    Worst,
    Empty,
    One,
    #[value(name = "1000")]
    Rows1000,
    Error,
    Loading,
}

impl DataSet {
    pub const ALL: [DataSet; 7] = [
        DataSet::Demo,
        DataSet::Worst,
        DataSet::Empty,
        DataSet::One,
        DataSet::Rows1000,
        DataSet::Error,
        DataSet::Loading,
    ];
    pub fn name(self) -> &'static str {
        match self {
            DataSet::Demo => "demo",
            DataSet::Worst => "worst",
            DataSet::Empty => "empty",
            DataSet::One => "one",
            DataSet::Rows1000 => "1000",
            DataSet::Error => "error",
            DataSet::Loading => "loading",
        }
    }
}

pub fn load(set: DataSet) -> Snapshot {
    match set {
        DataSet::Demo => demo(),
        DataSet::Worst => worst(),
        DataSet::Empty => Snapshot {
            tailnet: Tailnet::Ready {
                tailnet: "blakster.github".into(),
                devices: vec![mac_self()],
            },
            services: vec![],
            ..demo()
        },
        DataSet::One => {
            let d = demo();
            Snapshot {
                tailnet: Tailnet::Ready {
                    tailnet: "blakster.github".into(),
                    devices: d.devices()[..2].to_vec(),
                },
                services: d.services[2..3].to_vec(),
                ..d
            }
        }
        DataSet::Rows1000 => rows1000(),
        DataSet::Error => {
            let d = demo();
            Snapshot {
                tailnet: Tailnet::Error {
                    reason: "can't run /Applications/Tailscale.app/Contents/MacOS/Tailscale: No such file or directory".into(),
                },
                services: d
                    .services
                    .iter()
                    .map(|s| ServiceStatus {
                        state: if matches!(s.state, ServiceState::Off) {
                            ServiceState::Off
                        } else if s.id == "orbstack" {
                            ServiceState::Down {
                                reason: "docker daemon unreachable".into(),
                            }
                        } else {
                            ServiceState::Blocked { by: "orbstack".into() }
                        },
                        since: NOW - 33 * MIN,
                        ..s.clone()
                    })
                    .collect(),
                ..d
            }
        }
        DataSet::Loading => Snapshot {
            at: NOW,
            services: demo()
                .services
                .into_iter()
                .map(|s| ServiceStatus {
                    state: if matches!(s.state, ServiceState::Off) {
                        ServiceState::Off
                    } else {
                        ServiceState::Unknown
                    },
                    checked_at: None,
                    ..s
                })
                .collect(),
            ..Default::default()
        },
    }
}

fn wave(n: usize, base: f32, amp: f32, period: f32) -> VecDeque<f32> {
    (0..n)
        .map(|i| (base + amp * (i as f32 / period).sin() + ((i * 37 % 11) as f32 - 5.0)).clamp(0.0, 100.0))
        .collect()
}

fn mac_host() -> HostMetrics {
    HostMetrics {
        name: "Vansh’s MacBook Air (5)".into(),
        model: "Apple M1".into(),
        cores: 8,
        cpu_pct: 31.0,
        mem: Usage {
            used: 11 * GB + GB / 4,
            total: 16 * GB,
        },
        pressure: Pressure::Normal,
        disk: Usage {
            used: 124 * GB,
            total: 228 * GB,
        },
        uptime_secs: 2 * 86_400 + 4 * 3_600 + 10 * 60,
        load: [3.61, 3.44, 3.41],
    }
}

fn mac_self() -> Device {
    Device {
        id: "n-self".into(),
        name: "Vansh’s MacBook Air (5)".into(),
        dns_name: "vanshs-macbook-air-5.tail1234.ts.net".into(),
        os: "macOS".into(),
        is_self: true,
        presence: Presence::Online {
            latency: Latency::Unmeasured,
        },
    }
}

fn dev(id: &str, name: &str, os: &str, presence: Presence) -> Device {
    Device {
        id: id.into(),
        name: name.into(),
        dns_name: format!("{id}.tail1234.ts.net"),
        os: os.into(),
        is_self: false,
        presence,
    }
}

fn svc(id: &str, name: &str, state: ServiceState, since: Millis) -> ServiceStatus {
    ServiceStatus {
        id: id.into(),
        name: name.into(),
        url: None,
        state,
        checked_at: Some(NOW - 2_000),
        since,
    }
}

fn demo() -> Snapshot {
    let mut history = History {
        cpu: wave(CPU_HISTORY, 30.0, 14.0, 9.0),
        mem: wave(CPU_HISTORY, 70.0, 3.0, 40.0),
        ..Default::default()
    };
    history.latency.insert("n-rex".into(), wave(LATENCY_HISTORY, 12.0, 2.0, 5.0));
    history.latency.insert(
        "n-box1".into(),
        (0..LATENCY_HISTORY).map(|i| 180.0 + ((i * 13 % 17) as f32)).collect(),
    );
    Snapshot {
        at: NOW,
        host: Some(mac_host()),
        tailnet: Tailnet::Ready {
            tailnet: "blakster.github".into(),
            devices: vec![
                mac_self(),
                dev(
                    "n-rex",
                    "vanshs-macbook-air-5-rex",
                    "macOS",
                    Presence::Online {
                        latency: Latency::Direct { ms: 11.0 },
                    },
                ),
                dev(
                    "n-box1",
                    "vansh-box-1",
                    "linux",
                    Presence::Online {
                        latency: Latency::Relay {
                            ms: 182.0,
                            region: "bom".into(),
                        },
                    },
                ),
                dev(
                    "n-phone",
                    "Nothing Phone (1)",
                    "android",
                    Presence::Offline {
                        last_seen: Some(NOW - 8 * DAY - 3 * HOUR),
                    },
                ),
                dev(
                    "n-box",
                    "vansh-box",
                    "linux",
                    Presence::Offline {
                        last_seen: Some(NOW - 39 * DAY),
                    },
                ),
            ],
        },
        services: vec![
            svc("orbstack", "OrbStack Docker", ServiceState::Up { ms: 4 }, NOW - 2 * DAY),
            svc("pulse-db", "Pulse Postgres", ServiceState::Up { ms: 38 }, NOW - 2 * DAY),
            ServiceStatus {
                url: Some("https://vanshs-macbook-air-5.tail1234.ts.net".into()),
                ..svc("pulse", "Pulse", ServiceState::Up { ms: 41 }, NOW - 12 * MIN)
            },
            ServiceStatus {
                checked_at: None,
                ..svc("air-health", "Air Health", ServiceState::Off, NOW - 2 * DAY)
            },
        ],
        history,
    }
}

const LONG_HOST: &str = "a-really-long-hostname-that-hits-the-dns-label-limit-of-63-char";
const LONG_REASON: &str = "error sending request for url (http://127.0.0.1:3001/healthz): client error (Connect): tcp connect error: Connection refused (os error 61) — and then some more text so the reason runs well past any column width we could give it";

fn worst() -> Snapshot {
    let mut history = History {
        cpu: VecDeque::from(vec![100.0; CPU_HISTORY]),
        mem: VecDeque::from(vec![100.0; CPU_HISTORY]),
        ..Default::default()
    };
    history
        .latency
        .insert("w-relay".into(), VecDeque::from(vec![1999.0; LATENCY_HISTORY]));
    Snapshot {
        at: NOW,
        host: Some(HostMetrics {
            name: "株式会社サーバー’s Mac Studio — build farm (2)".into(),
            model: "Apple M4 Ultra with an unusually long brand string".into(),
            cores: 32,
            cpu_pct: 100.0,
            mem: Usage {
                used: 192 * GB,
                total: 192 * GB,
            },
            pressure: Pressure::Critical,
            disk: Usage { used: 0, total: 0 },
            uptime_secs: 400 * 86_400 + 23 * 3_600 + 59 * 60,
            load: [99.99, 88.88, 77.77],
        }),
        tailnet: Tailnet::Ready {
            tailnet: "a-very-long-tailnet-name.github".into(),
            devices: vec![
                Device {
                    name: LONG_HOST.into(),
                    ..mac_self()
                },
                dev(
                    "w-relay",
                    LONG_HOST,
                    "linux",
                    Presence::Online {
                        latency: Latency::Relay {
                            ms: 1999.0,
                            region: "fra-extra".into(),
                        },
                    },
                ),
                dev(
                    "w-zero",
                    "0",
                    "ios",
                    Presence::Online {
                        latency: Latency::Direct { ms: 0.0 },
                    },
                ),
                dev(
                    "w-unmeasured",
                    "Ünïcødé 🚀 phone",
                    "android",
                    Presence::Online {
                        latency: Latency::Unmeasured,
                    },
                ),
                dev("w-never", "never-seen", "windows", Presence::Offline { last_seen: None }),
                dev(
                    "w-old",
                    "ancient",
                    "linux",
                    Presence::Offline {
                        last_seen: Some(NOW - 900 * DAY),
                    },
                ),
                dev(
                    "w-future",
                    "clock-skew",
                    "linux",
                    Presence::Offline {
                        last_seen: Some(NOW + 5 * MIN),
                    },
                ),
            ],
        },
        services: vec![
            svc(
                "root",
                "OrbStack Docker",
                ServiceState::Down {
                    reason: LONG_REASON.into(),
                },
                NOW - 400 * DAY,
            ),
            svc(
                "child",
                "A service name that is sixty-four characters long, give or take",
                ServiceState::Blocked { by: "root".into() },
                NOW - 59 * 1000,
            ),
            svc("zero", "z", ServiceState::Up { ms: 0 }, NOW),
            svc("slow", "Slow", ServiceState::Up { ms: 2999 }, NOW - 1),
            ServiceStatus {
                checked_at: None,
                ..svc("never", "Never probed", ServiceState::Unknown, NOW)
            },
            svc("off", "Disabled", ServiceState::Off, NOW),
        ],
        history,
    }
}

fn rows1000() -> Snapshot {
    let w = worst();
    let d = demo();
    let pool: Vec<Device> = w.devices()[1..].iter().chain(d.devices()[1..].iter()).cloned().collect();
    let mut devices = vec![mac_self()];
    devices.extend((0..1000).map(|i| Device {
        id: format!("r{i}"),
        ..pool[i % pool.len()].clone()
    }));
    let spool: Vec<ServiceStatus> = w.services.iter().chain(d.services.iter()).cloned().collect();
    let services = (0..1000)
        .map(|i| ServiceStatus {
            id: format!("s{i}"),
            ..spool[i % spool.len()].clone()
        })
        .collect();
    Snapshot {
        tailnet: Tailnet::Ready {
            tailnet: "blakster.github".into(),
            devices,
        },
        services,
        ..d
    }
}
