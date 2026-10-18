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
        let text = if name == "mainnet" { MAINNET } else { TESTNET };
        toml::from_str(text).expect("embedded network file is valid")
    }
}

/// TODO: read the monitor stat cell in a later milestone.
pub fn mainnet_runtime_unlocked() -> bool {
    false
}
