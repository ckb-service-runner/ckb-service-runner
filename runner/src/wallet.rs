//! The runner wallet (`csr wallet …`, `/api/wallet…`): status, manual send and the automatic sweep to the
//! operator's `payout.sweep_address`.

use crate::chain::{ckb_to_shannons, Chain, CKB};
use crate::config::Config;
use ckb_types::{bytes::Bytes, core::ScriptHashType, packed, prelude::*};
use serde_json::{json, Value};
use std::path::Path;

/// File in the network's data directory that remembers the last sweep.
const LAST_SWEEP: &str = "last_sweep.json";

fn lock_of(network: &str, address: &str) -> Result<packed::Script, String> {
    let l = crate::address::decode(network, address)?;
    let hash_type = match l.hash_type {
        0 => ScriptHashType::Data,
        1 => ScriptHashType::Type,
        2 => ScriptHashType::Data1,
        _ => ScriptHashType::Data2,
    };
    Ok(packed::Script::new_builder()
        .code_hash(packed::Byte32::from_slice(&l.code_hash).expect("32 bytes"))
        .hash_type(packed::Byte::new(hash_type as u8))
        .args(Bytes::from(l.args).pack())
        .build())
}

/// `{"tx_hash", "amount_shannons", "at_ms"}` of the last sweep, or `null`.
pub fn last_sweep(net_dir: &Path) -> Value {
    std::fs::read_to_string(net_dir.join(LAST_SWEEP))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .filter(Value::is_object)
        .unwrap_or(Value::Null)
}

fn record_sweep(net_dir: &Path, tx_hash: &str, amount: u64) -> Result<(), String> {
    let at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let v = json!({"tx_hash": tx_hash, "amount_shannons": amount, "at_ms": at_ms});
    let f = net_dir.join(LAST_SWEEP);
    let tmp = net_dir.join(format!("{LAST_SWEEP}.tmp"));
    std::fs::write(&tmp, v.to_string() + "\n")
        .and_then(|_| std::fs::rename(&tmp, &f))
        .map_err(|e| format!("cannot write {}: {e}", f.display()))
}

/// Warning when earnings pile up on the hot key and no sweep address is set.
pub fn sweep_warning(sweep_address: Option<&str>, capacity_shannons: u64, threshold_ckb: u64) -> Option<String> {
    if sweep_address.is_some() || capacity_shannons < threshold_ckb.saturating_mul(CKB) {
        return None;
    }
    Some(format!(
        "the runner wallet holds {} CKB and no sweep address is set: earnings stay on the runner's hot key. Set payout.sweep_address in the settings to sweep them to your own wallet",
        capacity_shannons / CKB
    ))
}

/// Why the runner wallet cannot be used for a dashboard action now: a keeper transaction (a run in state
/// `Submitting`) or a sweep is still pending. `None` when the wallet is free.
pub fn wallet_busy(submitting_runs: usize, sweep_pending: bool) -> Option<String> {
    if sweep_pending {
        Some("a wallet sweep is pending".into())
    } else if submitting_runs > 0 {
        Some(format!(
            "{submitting_runs} job transaction(s) are pending"
        ))
    } else {
        None
    }
}

/// `csr wallet status` / `GET /api/wallet`.
pub async fn status(chain: &Chain, cfg: &Config, net_dir: &Path) -> Result<Value, String> {
    let capacity = chain.wallet_capacity().await?;
    let sweep_warning = sweep_warning(
        cfg.payout.sweep_address(),
        capacity,
        cfg.payout.sweep_threshold_ckb,
    );
    Ok(json!({
        "address": chain.runner_address(),
        "capacity_shannons": capacity,
        "sweep_warning": sweep_warning,
        "sweep_address": cfg.payout.sweep_address(),
        "sweep_threshold_ckb": cfg.payout.sweep_threshold_ckb,
        "sweep_amount_ckb": cfg.payout.sweep_amount_ckb,
        "last_sweep": last_sweep(net_dir),
    }))
}

/// `csr wallet send` / `POST /api/wallet/send`: exactly `ckb` CKB to a full-format address of this network.
pub async fn send(chain: &Chain, to: &str, ckb: &str) -> Result<Value, String> {
    let lock = lock_of(chain.network(), to)?;
    let amount = ckb_to_shannons(ckb)?;
    if amount < 61 * CKB {
        return Err("the amount must be at least 61 CKB (a cell needs 61 CKB)".into());
    }
    let tx_hash = chain.wallet_send(lock, amount).await?;
    tracing::info!(
        "runner wallet: sent {} CKB to {to} in {tx_hash}",
        amount as f64 / CKB as f64
    );
    Ok(json!({"tx_hash": tx_hash}))
}

/// One sweep: with a sweep address and at least `sweep_threshold_ckb` in the wallet, sends exactly
/// `sweep_amount_ckb` there and records it.
pub async fn sweep(chain: &Chain, cfg: &Config, net_dir: &Path) -> Result<Value, String> {
    let none = json!({"swept": false, "tx_hash": null, "amount_shannons": null});
    let Some(addr) = cfg.payout.sweep_address() else {
        return Ok(none);
    };
    let lock = lock_of(chain.network(), addr)?;
    let have = chain.wallet_capacity().await?;
    let threshold = cfg
        .payout
        .sweep_threshold_ckb
        .checked_mul(CKB)
        .ok_or("payout.sweep_threshold_ckb is too large")?;
    if have < threshold {
        return Ok(none);
    }
    let amount = cfg
        .payout
        .sweep_amount_ckb
        .checked_mul(CKB)
        .ok_or("payout.sweep_amount_ckb is too large")?;
    let tx_hash = chain.wallet_send(lock, amount).await?;
    record_sweep(net_dir, &tx_hash, amount)?;
    tracing::info!(
        "runner wallet: swept {} CKB to {addr} in {tx_hash} (wallet held {} CKB)",
        cfg.payout.sweep_amount_ckb,
        have as f64 / CKB as f64
    );
    Ok(json!({"swept": true, "tx_hash": tx_hash, "amount_shannons": amount}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_sweep_round_trip() {
        let tmp = crate::test_tmp();
        assert_eq!(last_sweep(tmp.path()), Value::Null);
        record_sweep(tmp.path(), "0xab", 5).unwrap();
        let v = last_sweep(tmp.path());
        assert_eq!(v["tx_hash"], "0xab");
        assert_eq!(v["amount_shannons"], 5);
        assert!(v["at_ms"].as_u64().unwrap() > 0);
    }

    #[test]
    fn address_locks() {
        let addr = crate::address::sighash_address("devnet", &[7u8; 20]);
        let l = lock_of("devnet", &addr).unwrap();
        assert_eq!(l, crate::chain::sighash_lock(&[7u8; 20]));
        assert!(lock_of("mainnet", &addr).is_err());
    }
}
