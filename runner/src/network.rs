//! Per-network defaults, embedded from `config/networks/`.

// Consumed by the monitor/ping code in later phases.
#![allow(dead_code)]

use serde::Deserialize;

const TESTNET: &str = include_str!("../../config/networks/testnet.toml");
const MAINNET: &str = include_str!("../../config/networks/mainnet.toml");

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkDefaults {
    pub monitor_url: String,
    pub monitor_pubkey: String,
}

impl NetworkDefaults {
    pub fn for_network(name: &str) -> NetworkDefaults {
        // devnet has no shipped defaults (no monitor); it reuses the testnet file.
        let text = if name == "mainnet" { MAINNET } else { TESTNET };
        toml::from_str(text).expect("embedded network file is valid")
    }
}

/// The monitor key in effect: the configured one, else the network default.
pub fn effective_monitor_key(cfg: &crate::config::Config) -> String {
    if cfg.monitor.pubkey.is_empty() {
        NetworkDefaults::for_network(&cfg.network).monitor_pubkey
    } else {
        cfg.monitor.pubkey.clone()
    }
}

/// False for an empty key, the secp256k1 generator point (the old placeholder) and a key that does not parse.
pub fn monitor_key_usable(pubkey: &str) -> bool {
    const GENERATOR: &str = "0x0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
    !pubkey.is_empty()
        && !pubkey.eq_ignore_ascii_case(GENERATOR)
        && crate::config::parse_pubkey(pubkey).is_ok()
}

/// Why no monitor key is usable, or `None`. Logs one warning per process.
pub fn monitor_key_problem(cfg: &crate::config::Config) -> Option<String> {
    static WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if monitor_key_usable(&effective_monitor_key(cfg)) {
        return None;
    }
    let msg = format!(
        "no usable monitor key for {} (set monitor.pubkey); challenges are not answered and nothing is sent to the monitor",
        cfg.network
    );
    if !WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        tracing::warn!("{msg}");
    }
    Some(msg)
}

/// The HTTP client for every outgoing request: no proxy variables, no redirects.
pub fn http_client(timeout: std::time::Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(timeout)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("http client: {e}"))
}

/// Error for a redirect answer.
pub fn check_no_redirect(resp: &reqwest::Response) -> Result<(), String> {
    if resp.status().is_redirection() {
        return Err("unexpected redirect".into());
    }
    Ok(())
}

/// Switched on by the release that deploys mainnet (decision D5 B).
pub fn mainnet_runtime_unlocked() -> bool {
    false
}

/// Shown and returned while registration is not available on mainnet (0.5.1).
pub const MAINNET_LOCKED_MSG: &str = "Mainnet: node dashboard and ranking only. Registration and service jobs start with CSR 1.x, once the network has 20 ranked runners and 4 months of monitor history. Nothing needs to be funded now.";

/// `Some(message)` while registration, renewal and consent are not available on this network: mainnet before
/// the runtime is unlocked (the CSR scripts are not deployed there yet).
pub fn registration_locked(network: &str) -> Option<&'static str> {
    (network == "mainnet" && !mainnet_runtime_unlocked()).then_some(MAINNET_LOCKED_MSG)
}

#[cfg(test)]
mod lock_tests {
    #[test]
    fn registration_locked_only_on_mainnet() {
        assert!(super::registration_locked("mainnet").is_some());
        assert!(super::registration_locked("testnet").is_none());
        assert!(super::registration_locked("devnet").is_none());
    }
}
