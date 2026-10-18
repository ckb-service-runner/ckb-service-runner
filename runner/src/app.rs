//! Shared state of a running service.

use crate::config::Config;
use crate::db::Db;
use crate::logs::LevelSetter;
use crate::network::mainnet_runtime_unlocked;
use crate::node::r2::R2State;
use crate::node::Polled;
use crate::resources::Resources;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, RwLock};
use std::time::Instant;

pub struct AppState {
    pub config: RwLock<Config>,
    pub config_path: PathBuf,
    pub net_dir: PathBuf,
    pub started: Instant,
    pub service_pubkey: String,
    pub service_key: secp256k1::SecretKey,
    pub storage_key: [u8; 32],
    /// Ping nonces answered within the replay window, with the time.
    pub ping_nonces: Mutex<std::collections::HashMap<String, i64>>,
    pub db: Db,
    pub set_log_level: LevelSetter,
    pub node: RwLock<Polled>,
    pub peer_id: RwLock<Option<String>>,
    pub resources: RwLock<Option<Resources>>,
    /// Unix seconds of the last successful ping answer, 0 = never.
    pub last_check_ok_at: AtomicI64,
    /// Serialises writes of the config file.
    pub config_write: Mutex<()>,
    /// Dashboard sessions (in memory only).
    /// Cached monitor public-API data.
    pub monitor_cache: tokio::sync::Mutex<crate::monitor::Cache>,
    pub sessions: Mutex<std::collections::HashMap<String, crate::web::Session>>,
}

impl AppState {
    /// Reserves a ping nonce; false if it was answered within the replay window.
    pub fn reserve_ping_nonce(&self, nonce: &str, now: i64) -> bool {
        let mut m = self.ping_nonces.lock().unwrap();
        m.retain(|_, t| now - *t < crate::ping::protocol::REPLAY_WINDOW_SECS);
        m.insert(nonce.to_string(), now).is_none()
    }

    pub fn release_ping_nonce(&self, nonce: &str) {
        self.ping_nonces.lock().unwrap().remove(nonce);
    }

    pub fn log_path(&self) -> PathBuf {
        self.net_dir.join("logs").join("csr.log")
    }

    pub fn status_json(&self) -> Value {
        let cfg = self.config.read().unwrap();
        let node = self.node.read().unwrap();
        let (state, reason) = runtime_state(cfg.runtime.enabled, &cfg.network, node.report.state);
        let ok_at = self.last_check_ok_at.load(Ordering::Relaxed);
        json!({
            "network": cfg.network,
            "version": env!("CARGO_PKG_VERSION"),
            "uptime_s": self.started.elapsed().as_secs(),
            "peer_id": *self.peer_id.read().unwrap(),
            "service_pubkey": self.service_pubkey,
            "r2": {"state": node.report.state.as_str(), "reasons": node.report.reasons},
            "runtime": {"state": state, "reason": reason},
            "mainnet_runtime_unlocked": mainnet_runtime_unlocked(),
            "ranking": {
                "join": cfg.ranking.join,
                "ping_mode": cfg.ranking.ping_mode,
                "last_check_ok_at": if ok_at > 0 { json!(ok_at) } else { Value::Null },
            },
        })
    }

    pub fn node_json(&self, cfg_node_type: &str) -> Value {
        let p = self.node.read().unwrap();
        let s = &p.snap;
        let r = &p.report;
        let reachable = !s.transport_error && s.tip.is_some();
        let (peers_in, peers_out) = match (&s.peers, reachable) {
            (Some(v), true) => {
                let out = v.iter().filter(|o| **o).count();
                (Some(v.len() - out), Some(out))
            }
            _ => (None, None),
        };
        let node_type = if !reachable {
            None
        } else if s.chain_info.is_none() || s.sync.is_none() {
            Some("unsupported")
        } else if cfg_node_type == "neuron" {
            Some("neuron")
        } else {
            Some("ckb")
        };
        json!({
            "chain": s.chain_info.as_ref().map(|c| c.0.clone()),
            "node_version": s.node_version,
            "node_type": node_type,
            "tip_number": r.tip,
            "tip_hash": s.tip.as_ref().map(|t| t.hash.clone()),
            "best_known": r.best_known,
            "tip_lag": r.tip_lag,
            "epoch": s.tip.as_ref().and_then(|t| crate::node::rpc::hex_u64(&json!(t.epoch))).map(|e| e & 0xff_ffff),
            "peers_in": peers_in,
            "peers_out": peers_out,
            "indexer_tip": r.indexer_tip,
            "indexer_lag": r.indexer_lag,
            "tx_pool": {
                "pending": p.tx_pool.map(|t| t.0),
                "proposed": p.tx_pool.map(|t| t.1),
            },
        })
    }
}

/// Runtime state values: `off`, `inactive` (mainnet gate), `paused`, `on`.
pub fn runtime_state(enabled: bool, network: &str, r2: R2State) -> (&'static str, Option<String>) {
    if !enabled {
        ("off", Some("runtime disabled in the config".into()))
    } else if network == "mainnet" && !mainnet_runtime_unlocked() {
        (
            "inactive",
            Some("mainnet runtime is not unlocked yet".into()),
        )
    } else if r2 != R2State::Synced {
        (
            "paused",
            Some(format!("node not synced (R2: {})", r2.as_str())),
        )
    } else {
        ("on", None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_states() {
        assert_eq!(runtime_state(false, "testnet", R2State::Synced).0, "off");
        assert_eq!(
            runtime_state(true, "mainnet", R2State::Synced).0,
            "inactive"
        );
        assert_eq!(runtime_state(true, "testnet", R2State::Syncing).0, "paused");
        assert_eq!(runtime_state(true, "testnet", R2State::Synced).0, "on");
    }
}
