//! This machine's CPU, memory, disk, uptime and load.

use sysinfo::{CpuRefreshKind, Disks, MemoryRefreshKind, RefreshKind, System};

use crate::model::{HostMetrics, Pressure, Usage};

pub struct HostSampler {
    sys: System,
    disks: Disks,
    name: String,
    model: String,
}

impl HostSampler {
    pub fn new() -> anyhow::Result<Self> {
        let sys = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::everything())
                .with_memory(MemoryRefreshKind::everything()),
        )?;
        let model = sys.cpus().first().map(|c| c.brand().trim().to_string()).unwrap_or_default();
        let name = macos_computer_name()
            .or_else(|| System::host_name().ok())
            .unwrap_or_else(|| "this machine".into());
        Ok(Self {
            sys,
            disks: Disks::new_with_refreshed_list()?,
            name,
            model,
        })
    }

    /// CPU % is the delta since the previous call, so call this on a steady tick.
    pub fn sample(&mut self) -> HostMetrics {
        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();
        self.disks.refresh(true);
        let load = System::load_average()
            .map(|l| [l.one as f32, l.five as f32, l.fifteen as f32])
            .unwrap_or([0.0; 3]);
        HostMetrics {
            name: self.name.clone(),
            model: self.model.clone(),
            cores: self.sys.cpus().len() as u16,
            cpu_pct: self.sys.global_cpu_usage().clamp(0.0, 100.0),
            mem: memory(&self.sys),
            pressure: pressure(),
            disk: disk(&self.disks),
            uptime_secs: System::uptime().unwrap_or(0),
            load,
        }
    }
}

/// macOS counts "used" as active + wired + compressed (what Activity Monitor
/// calls App + Wired + Compressed). `sysinfo` elsewhere.
fn memory(sys: &System) -> Usage {
    #[cfg(target_os = "macos")]
    if let Some(out) = run("vm_stat", &[]) {
        if let Some(used) = parse_vm_stat(&out) {
            return Usage {
                used,
                total: sys.total_memory(),
            };
        }
    }
    Usage {
        used: sys.used_memory(),
        total: sys.total_memory(),
    }
}

/// Parse `vm_stat` into used bytes (active + wired + compressed).
pub fn parse_vm_stat(out: &str) -> Option<u64> {
    let page: u64 = out.split("page size of ").nth(1)?.split_whitespace().next()?.parse().ok()?;
    let pages = |label: &str| -> Option<u64> {
        let line = out.lines().find(|l| l.starts_with(label))?;
        line.rsplit(':').next()?.trim().trim_end_matches('.').parse().ok()
    };
    let active = pages("Pages active")?;
    let wired = pages("Pages wired down")?;
    let compressed = pages("Pages occupied by compressor").unwrap_or(0);
    Some((active + wired + compressed) * page)
}

fn pressure() -> Pressure {
    #[cfg(target_os = "macos")]
    if let Some(level) = run("sysctl", &["-n", "kern.memorystatus_vm_pressure_level"]) {
        return parse_pressure(&level);
    }
    Pressure::Normal
}

/// `kern.memorystatus_vm_pressure_level`: 1 normal, 2 warn, 4 critical.
pub fn parse_pressure(level: &str) -> Pressure {
    match level.trim() {
        "2" => Pressure::Warn,
        "4" => Pressure::Critical,
        _ => Pressure::Normal,
    }
}

/// The volume that actually fills: the APFS Data volume on macOS, `/` elsewhere.
fn disk(disks: &Disks) -> Usage {
    let want = if cfg!(target_os = "macos") { "/System/Volumes/Data" } else { "/" };
    let d = disks.iter().find(|d| d.mount_point().as_os_str() == want).or_else(|| disks.first());
    match d {
        Some(d) => Usage {
            used: d.total_space().saturating_sub(d.available_space()),
            total: d.total_space(),
        },
        None => Usage { used: 0, total: 0 },
    }
}

#[cfg(target_os = "macos")]
fn macos_computer_name() -> Option<String> {
    run("scutil", &["--get", "ComputerName"])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}
#[cfg(not(target_os = "macos"))]
fn macos_computer_name() -> Option<String> {
    None
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(cmd).args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vm_stat_fixture_gives_exact_bytes() {
        let out = include_str!("../../fixtures/vm_stat.txt");
        // (227340 active + 120000 wired + 90000 compressed) * 16384
        assert_eq!(parse_vm_stat(out), Some((227_340 + 120_000 + 90_000) * 16_384));
    }

    #[test]
    fn vm_stat_garbage_is_none() {
        assert_eq!(parse_vm_stat("nope"), None);
    }

    #[test]
    fn pressure_levels() {
        assert_eq!(parse_pressure("1\n"), Pressure::Normal);
        assert_eq!(parse_pressure("2"), Pressure::Warn);
        assert_eq!(parse_pressure("4"), Pressure::Critical);
    }

    #[test]
    fn sampler_reports_sane_values_here() {
        let mut s = HostSampler::new().unwrap();
        let m = s.sample();
        assert!(m.cores > 0 && m.mem.total > 0 && m.mem.used <= m.mem.total);
        assert!((0.0..=100.0).contains(&m.cpu_pct));
    }
}
