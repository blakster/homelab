//! `~/.config/homelab/config.toml`, parsed once at startup. No secrets by design.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub tailscale: TailscaleConfig,
    #[serde(default)]
    pub docker_bin: Option<String>,
    #[serde(default)]
    pub ssh: SshConfig,
    #[serde(default, rename = "service")]
    pub services: Vec<ServiceConfig>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TailscaleConfig {
    #[serde(default = "default_tailscale_bin")]
    pub bin: String,
}

impl Default for TailscaleConfig {
    fn default() -> Self {
        Self {
            bin: default_tailscale_bin(),
        }
    }
}

fn default_tailscale_bin() -> String {
    const MAC_APP: &str = "/Applications/Tailscale.app/Contents/MacOS/Tailscale";
    if Path::new(MAC_APP).exists() {
        MAC_APP.into()
    } else {
        "tailscale".into()
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SshConfig {
    #[serde(default = "default_listen")]
    pub listen: String,
    #[serde(default = "default_authorized_keys")]
    pub authorized_keys: String,
    #[serde(default = "default_host_key")]
    pub host_key: String,
}

impl Default for SshConfig {
    fn default() -> Self {
        Self {
            listen: default_listen(),
            authorized_keys: default_authorized_keys(),
            host_key: default_host_key(),
        }
    }
}

fn default_listen() -> String {
    "127.0.0.1:2222".into()
}
fn default_authorized_keys() -> String {
    "~/.config/homelab/authorized_keys".into()
}
fn default_host_key() -> String {
    "~/.config/homelab/host_ed25519".into()
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ServiceConfig {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub url: Option<String>,
    pub check: Check,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Check {
    Http {
        url: String,
        /// Expected status; any 2xx when absent.
        #[serde(default)]
        expect: Option<u16>,
    },
    Tcp {
        addr: String,
    },
    Docker {
        container: String,
    },
    DockerDaemon,
}

impl Config {
    pub fn default_path() -> PathBuf {
        expand("~/.config/homelab/config.toml")
    }

    pub fn load(path: &Path) -> Result<Config> {
        if !path.exists() {
            return Config::parse("");
        }
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Config::parse(&text).with_context(|| format!("in {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Config> {
        let cfg: Config = toml::from_str(text)?;
        cfg.validate()?;
        Ok(cfg)
    }

    fn validate(&self) -> Result<()> {
        let mut seen = std::collections::HashSet::new();
        for s in &self.services {
            if !seen.insert(s.id.as_str()) {
                bail!("duplicate service id `{}`", s.id);
            }
        }
        for s in &self.services {
            for d in &s.depends_on {
                if !seen.contains(d.as_str()) {
                    bail!("service `{}`: depends_on names unknown service `{d}`", s.id);
                }
                if d == &s.id {
                    bail!("service `{}` depends on itself", s.id);
                }
            }
        }
        Ok(())
    }
}

/// Expand a leading `~/` to `$HOME`.
pub fn expand(path: &str) -> PathBuf {
    match (path.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = include_str!("../config.example.toml");

    #[test]
    fn example_config_parses() {
        let cfg = Config::parse(EXAMPLE).unwrap();
        assert_eq!(cfg.services.len(), 4);
        let pulse = cfg.services.iter().find(|s| s.id == "pulse").unwrap();
        assert_eq!(pulse.depends_on, vec!["pulse-db"]);
        assert!(matches!(&pulse.check, Check::Http { url, .. } if url.ends_with(":3001/healthz")));
        let air = cfg.services.iter().find(|s| s.id == "air-health").unwrap();
        assert!(!air.enabled);
    }

    #[test]
    fn empty_config_is_valid() {
        let cfg = Config::parse("").unwrap();
        assert!(cfg.services.is_empty());
        assert_eq!(cfg.ssh.listen, "127.0.0.1:2222");
    }

    #[test]
    fn bad_field_names_the_field() {
        let err = Config::parse("[[service]]\nid='a'\nname='A'\ncheck={kind='http', ur='x'}\n").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("ur"), "{msg}");
    }

    #[test]
    fn unknown_dependency_is_rejected() {
        let err = Config::parse("[[service]]\nid='a'\nname='A'\ncheck={kind='docker-daemon'}\ndepends_on=['nope']\n").unwrap_err();
        assert!(err.to_string().contains("nope"));
    }
}
