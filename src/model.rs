//! The one type every view renders, and the JSON seam (`homelab snapshot --json`).
//!
//! Illegal states are unrepresentable: an offline device has no latency, a
//! blocked service has no reason of its own, a disabled service is never probed.

use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};

/// Milliseconds since the Unix epoch.
pub type Millis = i64;

pub const CPU_HISTORY: usize = 300; // 10 min at 2 s
pub const LATENCY_HISTORY: usize = 60; // 30 min at 30 s

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub at: Millis,
    /// `None` until the first sample lands.
    pub host: Option<HostMetrics>,
    pub tailnet: Tailnet,
    pub services: Vec<ServiceStatus>,
    pub history: History,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HostMetrics {
    pub name: String,
    pub model: String,
    pub cores: u16,
    pub cpu_pct: f32,
    pub mem: Usage,
    pub pressure: Pressure,
    pub disk: Usage,
    pub uptime_secs: u64,
    pub load: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub used: u64,
    pub total: u64,
}

impl Usage {
    /// 0.0..=1.0, guarded against a zero total.
    pub fn ratio(self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            (self.used as f64 / self.total as f64).clamp(0.0, 1.0) as f32
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Pressure {
    Normal,
    Warn,
    Critical,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Tailnet {
    #[default]
    Loading,
    Error {
        reason: String,
    },
    Ready {
        tailnet: String,
        devices: Vec<Device>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub dns_name: String,
    pub os: String,
    pub is_self: bool,
    pub presence: Presence,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Presence {
    Online { latency: Latency },
    Offline { last_seen: Option<Millis> },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Latency {
    Unmeasured,
    Direct { ms: f32 },
    Relay { ms: f32, region: String },
}

impl Latency {
    pub fn ms(&self) -> Option<f32> {
        match self {
            Latency::Unmeasured => None,
            Latency::Direct { ms } | Latency::Relay { ms, .. } => Some(*ms),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ServiceStatus {
    pub id: String,
    pub name: String,
    pub url: Option<String>,
    pub state: ServiceState,
    pub checked_at: Option<Millis>,
    /// When `state` last changed kind (up → down etc.).
    pub since: Millis,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ServiceState {
    /// Not probed yet.
    Unknown,
    Up {
        ms: u32,
    },
    Down {
        reason: String,
    },
    /// Failing, and a `depends_on` service is not up. `by` is that service's id.
    Blocked {
        by: String,
    },
    /// `enabled = false` in config: shown, never probed, never a fault.
    Off,
}

impl ServiceState {
    pub fn is_fault(&self) -> bool {
        matches!(self, ServiceState::Down { .. } | ServiceState::Blocked { .. })
    }
    pub fn same_kind(&self, other: &ServiceState) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct History {
    pub cpu: VecDeque<f32>,
    pub mem: VecDeque<f32>,
    /// Keyed by device id.
    pub latency: BTreeMap<String, VecDeque<f32>>,
}

/// Push onto a bounded ring, dropping the oldest value past `cap`.
pub fn push_ring(ring: &mut VecDeque<f32>, value: f32, cap: usize) {
    ring.push_back(value);
    while ring.len() > cap {
        ring.pop_front();
    }
}

impl Snapshot {
    pub fn devices(&self) -> &[Device] {
        match &self.tailnet {
            Tailnet::Ready { devices, .. } => devices,
            _ => &[],
        }
    }
    pub fn faults(&self) -> impl Iterator<Item = &ServiceStatus> {
        self.services.iter().filter(|s| s.state.is_fault())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_caps_and_keeps_order() {
        let mut r = VecDeque::new();
        for i in 0..305 {
            push_ring(&mut r, i as f32, CPU_HISTORY);
        }
        assert_eq!(r.len(), CPU_HISTORY);
        assert_eq!(r.front(), Some(&5.0));
        assert_eq!(r.back(), Some(&304.0));
    }

    #[test]
    fn usage_ratio_guards_zero_total() {
        assert_eq!(Usage { used: 5, total: 0 }.ratio(), 0.0);
        assert_eq!(Usage { used: 50, total: 100 }.ratio(), 0.5);
    }

    #[test]
    fn snapshot_round_trips_through_json() {
        let s = crate::fixtures::load(crate::fixtures::DataSet::Worst);
        let json = serde_json::to_string(&s).unwrap();
        let back: Snapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }
}
