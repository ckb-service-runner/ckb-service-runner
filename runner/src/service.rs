//! `csr service ...` commands.

use crate::config::Config;
use crate::identity::Identity;
use crate::runtime::host::HostState;
use crate::runtime::manifest::Manifest;
use crate::runtime::vtx::VirtualTx;
use crate::runtime::{self, Limits};
use crate::storage::{self, Store};
use serde_json::json;
use std::path::Path;

pub struct RunLocalArgs<'a> {
    pub manifest: &'a Path,
    pub payload: &'a str,
    pub max_cycles: Option<u64>,
    pub max_memory_mb: Option<u32>,
    pub max_wall_seconds: Option<u32>,
    pub block_info: Option<&'a str>,
}

/// Parses a `0x`-prefixed hex string.
pub fn parse_hex(s: &str, what: &str) -> Result<Vec<u8>, String> {
    let h = s
        .strip_prefix("0x")
        .ok_or_else(|| format!("{what} must start with 0x"))?;
    hex::decode(h).map_err(|_| format!("{what} is not valid hex"))
}

fn parse_u64(s: &str) -> Result<u64, String> {
    let r = match s.strip_prefix("0x") {
        Some(h) => u64::from_str_radix(h, 16),
        None => s.parse(),
    };
    r.map_err(|_| format!("invalid number {s:?}"))
}

/// `TIP:EPOCH:MEDIAN_MS` (decimal or 0x hex).
pub fn parse_block_info(s: &str) -> Result<[u64; 3], String> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 3 {
        return Err("--block-info must be TIP:EPOCH:MEDIAN_MS".into());
    }
    Ok([
        parse_u64(parts[0])?,
        parse_u64(parts[1])?,
        parse_u64(parts[2])?,
    ])
}

/// `<data-root>/<network>/<storage.path>`.
pub fn storage_root(cfg: &Config, data_root: &Path) -> std::path::PathBuf {
    data_root.join(&cfg.network).join(&cfg.storage.path)
}

/// `csr storage simulate-consumed`: applies the `delete_on_consume` rules for an outpoint.
pub fn simulate_consumed(cfg: &Config, data_root: &Path, outpoint: &str) -> Result<usize, String> {
    let (hash, index) = outpoint
        .split_once(':')
        .ok_or("--outpoint must be 0x<64 hex>:<index>")?;
    let hash = parse_hex(hash, "outpoint hash")?;
    let index: u32 = index.parse().map_err(|_| "invalid outpoint index")?;
    if hash.len() != 32 {
        return Err("outpoint hash must be 32 bytes".into());
    }
    let mut op = hash;
    op.extend_from_slice(&index.to_le_bytes());
    let op: [u8; 36] = op.try_into().expect("36 bytes");
    let net_dir = data_root.join(&cfg.network);
    let identity = Identity::load_or_create(&net_dir)?;
    let root = storage_root(cfg, data_root);
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("deletions.log"))
        .map_err(|e| format!("cannot open deletion log: {e}"))?;
    storage::simulate_consumed(&root, &identity.storage_key, &op, &mut log)
}

/// Runs a service once in the sandbox and returns the JSON object to print.
pub fn run_local(
    cfg: &Config,
    data_root: &Path,
    a: &RunLocalArgs,
) -> Result<serde_json::Value, String> {
    let payload = parse_hex(a.payload, "--payload")?;
    let block_info = match a.block_info {
        Some(b) => parse_block_info(b)?,
        None => [0; 3],
    };
    let service = Manifest::load(a.manifest)?.read_files()?;
    let limits = Limits {
        max_cycles: a.max_cycles.unwrap_or(cfg.limits.max_cycles_per_job),
        max_memory_mb: a.max_memory_mb.unwrap_or(cfg.limits.max_vm_memory_mb),
        max_wall_seconds: a
            .max_wall_seconds
            .unwrap_or(cfg.limits.max_job_wall_seconds),
    };
    // Same as `csr run`: the network's keys are created when they are missing.
    let net_dir = data_root.join(&cfg.network);
    let identity = Identity::load_or_create(&net_dir)?;

    let code_hash = service.code_hash;
    let vtx = VirtualTx::new(
        service.elf.clone(),
        service.data_cells,
        service.manifest.args,
        payload,
    );
    let store = Store::new(
        &storage_root(cfg, data_root),
        &hex::encode(code_hash),
        identity.storage_key,
        cfg.storage.per_service_quota_mb,
        cfg.storage.total_quota_mb,
    );
    let state = HostState::new(vtx, block_info)
        .with_store(store)
        .with_signer(identity.service_key, code_hash);
    let (out, _) = runtime::run(&service.elf, state, limits)?;
    if let Some(e) = &out.error {
        eprintln!("vm: {e}");
    }
    Ok(json!({
        "status": out.status.as_str(),
        "exit_code": out.exit_code,
        "cycles": out.cycles,
        "result_hex": format!("0x{}", hex::encode(&out.result)),
        "code_hash": format!("0x{}", hex::encode(code_hash)),
        "debug": out.debug,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_info_parsing() {
        assert_eq!(parse_block_info("10:0x20:30").unwrap(), [10, 32, 30]);
        assert!(parse_block_info("1:2").is_err());
        assert!(parse_block_info("a:b:c").is_err());
    }

    #[test]
    fn hex_parsing() {
        assert_eq!(parse_hex("0x0102", "p").unwrap(), vec![1, 2]);
        assert!(parse_hex("0102", "p").is_err());
        assert!(parse_hex("0xzz", "p").is_err());
    }
}
