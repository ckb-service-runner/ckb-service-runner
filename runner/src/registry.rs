//! Local services registry: add, list and approval state.

use crate::config::Config;
use crate::db::{Db, ServiceRow};
use crate::node::rpc::RpcClient;
use crate::runtime::manifest::Manifest;
use crate::storage;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;

pub const NOT_APPROVED: &str = "not_approved";
pub const ENABLED: &str = "approved_enabled";
pub const DISABLED: &str = "approved_disabled";

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Approve,
    Revoke,
    Enable,
    Disable,
}

impl Action {
    pub fn parse(s: &str) -> Option<Action> {
        match s {
            "approve" => Some(Action::Approve),
            "revoke" => Some(Action::Revoke),
            "enable" => Some(Action::Enable),
            "disable" => Some(Action::Disable),
            _ => None,
        }
    }
}

pub enum ActionError {
    Unknown,
    Conflict(String),
    Other(String),
}

impl ActionError {
    pub fn message(&self) -> String {
        match self {
            ActionError::Unknown => "unknown service".into(),
            ActionError::Conflict(m) | ActionError::Other(m) => m.clone(),
        }
    }
}

/// Reads the manifest and registers the service (a known service keeps its status).
pub fn add_local(db: &Db, manifest: &Path) -> Result<Value, String> {
    let m = Manifest::load(manifest)?;
    let (name, language) = (m.name.clone(), m.language.clone());
    let loaded = m.read_files()?;
    let code_hash = format!("0x{}", hex::encode(loaded.code_hash));
    db.add_service(&code_hash, &name, &language)?;
    Ok(json!({"code_hash": code_hash, "name": name}))
}

/// Reads a code cell from the node, computes its data hash and registers the service
/// (a known service keeps its status).
pub async fn add_onchain(db: &Db, rpc_url: &str, outpoint: &str) -> Result<Value, String> {
    let (tx_hash, index) = outpoint
        .split_once(':')
        .ok_or("--outpoint must be 0x<tx hash>:<index>")?;
    let index: u32 = index.parse().map_err(|_| "invalid outpoint index")?;
    if tx_hash.len() != 66 || hex::decode(tx_hash.trim_start_matches("0x")).is_err() {
        return Err("--outpoint must be 0x<64 hex>:<index>".into());
    }
    let client = RpcClient::new(rpc_url);
    let cell = client
        .call(
            "get_live_cell",
            json!([{"tx_hash": tx_hash, "index": format!("0x{index:x}")}, true]),
        )
        .await
        .map_err(|e| format!("cannot read the code cell: {e}"))?;
    if cell["status"] != "live" {
        return Err(format!("{outpoint} is not a live cell"));
    }
    let data = cell["cell"]["data"]["content"]
        .as_str()
        .and_then(|d| hex::decode(d.trim_start_matches("0x")).ok())
        .filter(|d| !d.is_empty())
        .ok_or("the cell has no code")?;
    let code_hash = format!("0x{}", hex::encode(ckb_hash::blake2b_256(&data)));
    let name = code_hash[2..10].to_string();
    db.add_service(&code_hash, &name, "onchain")?;
    db.set_service_outpoint(
        &code_hash,
        &format!("{}:{index}", tx_hash.to_ascii_lowercase()),
    )?;
    Ok(json!({"code_hash": code_hash, "name": name}))
}

fn entry(row: &ServiceRow, storage_root: &Path, totals: &HashMap<String, (u64, u64)>) -> Value {
    let (jobs, earned) = totals.get(&row.code_hash).copied().unwrap_or((0, 0));
    json!({
        "code_hash": row.code_hash,
        "name": row.name,
        "language": row.language,
        "status": row.status,
        "storage_bytes": storage::service_usage(storage_root, row.code_hash.trim_start_matches("0x")),
        "jobs": jobs,
        "earned_shannons": earned,
    })
}

pub fn list(db: &Db, storage_root: &Path) -> Result<Vec<Value>, String> {
    let totals = db.service_totals()?;
    Ok(db
        .services()?
        .iter()
        .map(|r| entry(r, storage_root, &totals))
        .collect())
}

/// Applies an action: updates `runtime.allowed_services` in `cfg`, saves the
/// config to `config_path` (atomic, `.bak`) and then stores the new status.
/// On error `cfg` may be modified; callers pass a clone.
pub fn act(
    db: &Db,
    cfg: &mut Config,
    config_path: &Path,
    storage_root: &Path,
    code_hash: &str,
    action: Action,
) -> Result<Value, ActionError> {
    let code_hash = code_hash.to_ascii_lowercase();
    let row = db
        .service(&code_hash)
        .map_err(ActionError::Other)?
        .ok_or(ActionError::Unknown)?;
    let new = match action {
        Action::Approve => ENABLED,
        Action::Revoke => NOT_APPROVED,
        Action::Enable | Action::Disable if row.status == NOT_APPROVED => {
            return Err(ActionError::Conflict("service is not approved".into()))
        }
        Action::Enable => ENABLED,
        Action::Disable => DISABLED,
    };
    let allowed = &mut cfg.runtime.allowed_services;
    allowed.retain(|a| {
        !a.trim_start_matches("0x")
            .eq_ignore_ascii_case(&code_hash[2..])
    });
    if new != NOT_APPROVED {
        allowed.push(code_hash.clone());
    }
    cfg.save(config_path).map_err(ActionError::Other)?;
    db.set_service_status(&code_hash, new)
        .map_err(ActionError::Other)?;
    let totals = db.service_totals().map_err(ActionError::Other)?;
    Ok(entry(
        &ServiceRow {
            status: new.into(),
            ..row
        },
        storage_root,
        &totals,
    ))
}
