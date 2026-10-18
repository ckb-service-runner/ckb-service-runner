//! Config model (`csr.toml`) and validation.

use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::Path;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct Config {
    pub network: String,
    pub node: NodeCfg,
    pub dashboard: DashboardCfg,
    pub ranking: RankingCfg,
    pub monitor: MonitorCfg,
    pub runtime: RuntimeCfg,
    pub limits: LimitsCfg,
    pub storage: StorageCfg,
    pub logging: LoggingCfg,
    /// Only valid with `network = "devnet"` (debug builds).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub devnet: Option<DevnetCfg>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DevnetCfg {
    /// Path to `deployments/devnet.json` written by `csr-dev deploy-contracts`.
    pub deployments: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct NodeCfg {
    pub rpc_url: String,
    pub node_type: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct DashboardCfg {
    pub bind: String,
    pub password_hash: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct RankingCfg {
    pub join: bool,
    pub ping_mode: String,
    pub ping_bind: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
#[serde(deny_unknown_fields, default)]
pub struct MonitorCfg {
    pub url: String,
    pub pubkey: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct RuntimeCfg {
    pub enabled: bool,
    pub allowed_services: Vec<String>,
    pub min_ckb_per_mcycle: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct LimitsCfg {
    pub max_concurrent_jobs: u32,
    pub max_cycles_per_job: u64,
    pub max_vm_memory_mb: u32,
    pub max_job_wall_seconds: u32,
    pub pause_above_cpu_percent: u32,
    pub min_free_ram_mb: u64,
    pub min_free_disk_gb: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct StorageCfg {
    pub path: String,
    pub total_quota_mb: u64,
    pub per_service_quota_mb: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct LoggingCfg {
    pub level: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            network: "testnet".into(),
            node: NodeCfg::default(),
            dashboard: DashboardCfg::default(),
            ranking: RankingCfg::default(),
            monitor: MonitorCfg::default(),
            runtime: RuntimeCfg::default(),
            limits: LimitsCfg::default(),
            storage: StorageCfg::default(),
            logging: LoggingCfg::default(),
            devnet: None,
        }
    }
}
impl Default for NodeCfg {
    fn default() -> Self {
        NodeCfg {
            rpc_url: "http://127.0.0.1:8114".into(),
            node_type: "auto".into(),
        }
    }
}
impl Default for DashboardCfg {
    fn default() -> Self {
        DashboardCfg {
            bind: "127.0.0.1:8130".into(),
            password_hash: String::new(),
        }
    }
}
impl Default for RankingCfg {
    fn default() -> Self {
        RankingCfg {
            join: true,
            ping_mode: "inbound".into(),
            ping_bind: "0.0.0.0:8131".into(),
        }
    }
}
impl Default for RuntimeCfg {
    fn default() -> Self {
        RuntimeCfg {
            enabled: false,
            allowed_services: Vec::new(),
            min_ckb_per_mcycle: 0.0,
        }
    }
}
impl Default for LimitsCfg {
    fn default() -> Self {
        LimitsCfg {
            max_concurrent_jobs: 2,
            max_cycles_per_job: 70_000_000,
            max_vm_memory_mb: 4,
            max_job_wall_seconds: 30,
            pause_above_cpu_percent: 85,
            min_free_ram_mb: 1024,
            min_free_disk_gb: 10,
        }
    }
}
impl Default for StorageCfg {
    fn default() -> Self {
        StorageCfg {
            path: "service_data".into(),
            total_quota_mb: 256,
            per_service_quota_mb: 64,
        }
    }
}
impl Default for LoggingCfg {
    fn default() -> Self {
        LoggingCfg {
            level: "info".into(),
        }
    }
}

fn one_of(what: &str, v: &str, allowed: &[&str]) -> Result<(), String> {
    if allowed.contains(&v) {
        Ok(())
    } else {
        Err(format!(
            "{what} must be one of {}, got \"{v}\"",
            allowed.join(", ")
        ))
    }
}

fn range(what: &str, v: u64, lo: u64, hi: u64) -> Result<(), String> {
    if (lo..=hi).contains(&v) {
        Ok(())
    } else {
        Err(format!("{what} must be in {lo}..={hi}, got {v}"))
    }
}

fn is_loopback_host(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "localhost" | "[::1]")
}

impl Config {
    /// Parses TOML text and validates it.
    pub fn parse(text: &str) -> Result<Config, String> {
        let cfg: Config = toml::from_str(text).map_err(|e| format!("invalid config: {e}"))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Loads a config file; a missing file yields the defaults.
    pub fn load(path: &Path) -> Result<Config, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => Config::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let cfg = Config::default();
                cfg.validate()?;
                Ok(cfg)
            }
            Err(e) => Err(format!("cannot read {}: {e}", path.display())),
        }
    }

    /// Reads a config file without validating it (missing file = defaults).
    pub fn load_unvalidated(path: &Path) -> Result<Config, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).map_err(|e| format!("invalid config: {e}")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(format!("cannot read {}: {e}", path.display())),
        }
    }

    /// Atomically writes the config (mode 0600, it may hold the password hash).
    /// An existing file is kept as `<path>.bak`.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let text =
            toml::to_string_pretty(self).map_err(|e| format!("cannot serialise config: {e}"))?;
        let err = |what: &str, e: std::io::Error| format!("{what} {}: {e}", path.display());
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| err("cannot create directory for", e))?;
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let tmp = path.with_file_name(format!("{name}.tmp"));
        let mut bak = path.as_os_str().to_owned();
        bak.push(".bak");
        let write = || -> std::io::Result<()> {
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp)?;
            f.write_all(text.as_bytes())?;
            f.sync_all()?;
            if path.exists() {
                std::fs::copy(path, &bak)?;
            }
            std::fs::rename(&tmp, path)
        };
        write().map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            err("cannot write", e)
        })
    }

    pub fn validate(&self) -> Result<(), String> {
        if cfg!(debug_assertions) {
            one_of("network", &self.network, &["testnet", "mainnet", "devnet"])?;
        } else {
            one_of("network", &self.network, &["testnet", "mainnet"])?;
        }
        match (&self.devnet, self.network == "devnet") {
            (Some(d), true) if d.deployments.trim().is_empty() => {
                return Err("devnet.deployments must not be empty".into());
            }
            (Some(_), false) => {
                return Err("[devnet] is only valid with network = \"devnet\"".into());
            }
            _ => {}
        }
        one_of(
            "node.node_type",
            &self.node.node_type,
            &["auto", "ckb", "neuron"],
        )?;
        one_of(
            "ranking.ping_mode",
            &self.ranking.ping_mode,
            &["inbound", "outbound", "both"],
        )?;
        one_of(
            "logging.level",
            &self.logging.level,
            &["error", "warn", "info", "debug", "trace"],
        )?;

        let u = url::Url::parse(&self.node.rpc_url)
            .map_err(|e| format!("node.rpc_url is not a valid URL: {e}"))?;
        let host = u.host_str().unwrap_or("");
        if u.scheme() != "http" || !is_loopback_host(host) {
            return Err(format!(
                "node.rpc_url must be http and point to localhost (127.0.0.1, localhost or [::1]), got \"{}\"",
                self.node.rpc_url
            ));
        }

        let dash: SocketAddr = self
            .dashboard
            .bind
            .parse()
            .map_err(|e| format!("dashboard.bind is not a valid address: {e}"))?;
        if !dash.ip().is_loopback() && self.dashboard.password_hash.is_empty() {
            return Err("dashboard.bind is not loopback: a dashboard password is required (csr set-password)".into());
        }
        self.ranking
            .ping_bind
            .parse::<SocketAddr>()
            .map_err(|e| format!("ranking.ping_bind is not a valid address: {e}"))?;

        if !self.monitor.url.is_empty() {
            let bad = || {
                format!(
                    "monitor.url must use https (http only for loopback hosts), got \"{}\"",
                    self.monitor.url
                )
            };
            let mu = url::Url::parse(&self.monitor.url).map_err(|_| bad())?;
            let ok = match mu.scheme() {
                "https" => mu.host_str().is_some(),
                "http" => is_loopback_host(mu.host_str().unwrap_or("")),
                _ => false,
            };
            if !ok {
                return Err(bad());
            }
        }
        if !self.monitor.pubkey.is_empty() {
            parse_pubkey(&self.monitor.pubkey).map_err(|e| format!("monitor.pubkey: {e}"))?;
        }

        if !self.runtime.min_ckb_per_mcycle.is_finite() || self.runtime.min_ckb_per_mcycle < 0.0 {
            return Err("runtime.min_ckb_per_mcycle must be >= 0".into());
        }

        let l = &self.limits;
        range(
            "limits.max_concurrent_jobs",
            l.max_concurrent_jobs.into(),
            1,
            64,
        )?;
        range(
            "limits.max_cycles_per_job",
            l.max_cycles_per_job,
            1,
            u64::MAX,
        )?;
        range("limits.max_vm_memory_mb", l.max_vm_memory_mb.into(), 1, 64)?;
        range(
            "limits.max_job_wall_seconds",
            l.max_job_wall_seconds.into(),
            1,
            3600,
        )?;
        range(
            "limits.pause_above_cpu_percent",
            l.pause_above_cpu_percent.into(),
            1,
            100,
        )?;

        if self.storage.per_service_quota_mb > self.storage.total_quota_mb {
            return Err("storage.per_service_quota_mb must be <= storage.total_quota_mb".into());
        }
        Ok(())
    }
}

/// Parses `0x` + 66 hex chars as a compressed secp256k1 public key.
pub fn parse_pubkey(s: &str) -> Result<secp256k1::PublicKey, String> {
    let h = s.strip_prefix("0x").ok_or("must start with 0x")?;
    if h.len() != 66 {
        return Err("must be 0x + 66 hex chars (compressed key)".into());
    }
    let bytes = hex::decode(h).map_err(|e| format!("invalid hex: {e}"))?;
    secp256k1::PublicKey::from_slice(&bytes).map_err(|e| format!("not a valid secp256k1 key: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(text: &str) -> String {
        Config::parse(text).unwrap_err()
    }

    #[test]
    fn defaults_are_valid() {
        Config::default().validate().unwrap();
        assert_eq!(Config::parse("").unwrap(), Config::default());
    }

    #[test]
    fn example_is_valid() {
        Config::parse(include_str!("../../config/csr.example.toml")).unwrap();
    }

    #[test]
    fn network_defaults_parse() {
        for n in ["testnet", "mainnet"] {
            let d = crate::network::NetworkDefaults::for_network(n);
            parse_pubkey(&d.monitor_pubkey).unwrap();
        }
    }

    #[test]
    fn unknown_keys_rejected() {
        assert!(Config::parse("foo = 1").is_err());
        assert!(Config::parse("[node]\nr2_guard = false").is_err());
        assert!(Config::parse("[guard]\nenabled = false").is_err());
    }

    #[test]
    fn rpc_url_rules() {
        assert!(err("[node]\nrpc_url = \"http://10.0.0.1:8114\"").contains("localhost"));
        assert!(err("[node]\nrpc_url = \"https://127.0.0.1:8114\"").contains("localhost"));
        Config::parse("[node]\nrpc_url = \"http://[::1]:8114\"").unwrap();
        Config::parse("[node]\nrpc_url = \"http://localhost:8114\"").unwrap();
    }

    #[test]
    fn enums_and_ranges() {
        assert!(Config::parse("network = \"regtest\"").is_err());
        assert!(Config::parse("[ranking]\nping_mode = \"x\"").is_err());
        assert!(Config::parse("[logging]\nlevel = \"loud\"").is_err());
        assert!(Config::parse("[node]\nnode_type = \"light\"").is_err());
        assert!(Config::parse("[limits]\nmax_concurrent_jobs = 0").is_err());
        assert!(Config::parse("[limits]\nmax_vm_memory_mb = 65").is_err());
        assert!(Config::parse("[limits]\nmax_job_wall_seconds = 3601").is_err());
        assert!(Config::parse("[limits]\npause_above_cpu_percent = 101").is_err());
        assert!(Config::parse("[limits]\nmax_cycles_per_job = 0").is_err());
        assert!(Config::parse("[runtime]\nmin_ckb_per_mcycle = -0.5").is_err());
        assert!(Config::parse("[storage]\nper_service_quota_mb = 999").is_err());
    }

    #[test]
    fn devnet_rules() {
        let dev = "network = \"devnet\"\n[devnet]\ndeployments = \"d.json\"";
        assert_eq!(Config::parse(dev).is_ok(), cfg!(debug_assertions));
        assert!(Config::parse("[devnet]\ndeployments = \"d.json\"").is_err());
        assert!(Config::parse("network = \"mainnet\"\n[devnet]\ndeployments = \"x\"").is_err());
    }

    #[test]
    fn dashboard_password_rule() {
        assert!(err("[dashboard]\nbind = \"0.0.0.0:8130\"").contains("password"));
        Config::parse("[dashboard]\nbind = \"0.0.0.0:8130\"\npassword_hash = \"x\"").unwrap();
    }

    #[test]
    fn monitor_rules() {
        assert!(err("[monitor]\nurl = \"http://10.1.2.3:9\"").contains("https"));
        Config::parse("[monitor]\nurl = \"http://127.0.0.1:9\"").unwrap();
        Config::parse("[monitor]\nurl = \"https://m.example.org\"").unwrap();
        assert!(Config::parse("[monitor]\npubkey = \"0x1234\"").is_err());
        let g = "0x0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
        Config::parse(&format!("[monitor]\npubkey = \"{g}\"")).unwrap();
    }
}
