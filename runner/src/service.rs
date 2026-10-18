//! `csr service ...` commands.

use crate::config::Config;
use crate::identity::Identity;
use crate::runtime::host::HostState;
use crate::runtime::manifest::Manifest;
use crate::runtime::vtx::VirtualTx;
use crate::runtime::{self, Limits};
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
    Identity::load_or_create(&net_dir)?;

    let code_hash = service.code_hash;
    let vtx = VirtualTx::new(
        service.elf.clone(),
        service.data_cells,
        service.manifest.args,
        payload,
    );
    let (out, _) = runtime::run(&service.elf, HostState::new(vtx, block_info), limits)?;
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
