//! Renewal of the monitor's attestation in the runner-owned registry cell (`spec/registry.md` §9):
//! `csr register refresh`, `POST /api/registry/refresh` and the background task.

use crate::app::AppState;
use crate::chain::Chain;
use crate::config::Config;
use crate::db::Db;
use crate::wallet::{REFRESH_PENDING, SWEEP_PENDING_AFTER_BLOCKS};
use serde_json::Value;

/// Settings key of the last renewal error (shown on the dashboard).
pub const RENEWAL_ERROR: &str = "renewal_error";

/// Asks the monitor for a new attestation (same node, key and stake) and sends the refresh transaction.
/// The transaction is stored as pending so that the runner's other transactions wait for it.
pub async fn renew(
    chain: &Chain,
    cfg: &Config,
    service_key: &secp256k1::SecretKey,
    service_pubkey: &str,
    db: &Db,
) -> Result<Value, String> {
    let (owned, _, _, stake) = chain
        .renewal_state()
        .await?
        .ok_or("this node is not registered")?;
    if !owned {
        return Err(
            "the registry cell is not owned by the runner wallet; only the owner's wallet can refresh it"
                .into(),
        );
    }
    let stake = stake.ok_or("the registration has no stake (v1 cell); register again with `csr register create`")?;
    let att = crate::attest::request(cfg, service_key, service_pubkey, &stake).await?;
    let block = chain.tip_info().await.map(|t| t.number).unwrap_or(0);
    let r = chain.register_refresh(&att).await?;
    if let Some(h) = r["tx_hash"].as_str() {
        db.put_setting(REFRESH_PENDING, &format!("{h}:{block}"))?;
    }
    Ok(r)
}

/// Follows the pending refresh: cleared when committed, rejected or after `SWEEP_PENDING_AFTER_BLOCKS`.
pub async fn follow_pending(state: &AppState, cfg: &Config) {
    let Ok(Some(p)) = state.db.get_setting(REFRESH_PENDING) else {
        return;
    };
    let clear = || {
        let _ = state.db.put_setting(REFRESH_PENDING, "");
    };
    let Some((hash, block)) = p.split_once(':') else {
        return clear();
    };
    let block: u64 = block.parse().unwrap_or(0);
    let Ok(chain) = Chain::connect(cfg, &state.service_pubkey).await else {
        return;
    };
    match chain.tx_status(hash).await {
        Ok((s, _)) if s == "committed" => {
            tracing::info!("attestation refresh {hash} committed");
            clear();
        }
        Ok((s, _)) if s == "pending" || s == "proposed" => {
            if let Ok(t) = chain.tip_info().await {
                if t.number.saturating_sub(block) > SWEEP_PENDING_AFTER_BLOCKS {
                    tracing::warn!("attestation refresh {hash} was not committed in time; given up");
                    clear();
                }
            }
        }
        Ok((s, _)) => {
            tracing::warn!("attestation refresh {hash} was not committed ({s})");
            clear();
        }
        Err(_) => {}
    }
}
