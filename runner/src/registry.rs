//! Local services registry: add, list and approval state.

use crate::catalog::{self, RawManifest};
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

/// Live cell data from the node; `None` if the cell is not live.
async fn live_data(client: &RpcClient, outpoint: &str) -> Result<Option<Vec<u8>>, String> {
    let (tx_hash, index) = crate::chain::parse_out_point(outpoint)?;
    let cell = client
        .call(
            "get_live_cell",
            json!([{"tx_hash": tx_hash, "index": format!("0x{index:x}")}, true]),
        )
        .await
        .map_err(|e| format!("cannot read {outpoint}: {e}"))?;
    if cell["status"] != "live" {
        return Ok(None);
    }
    Ok(cell["cell"]["data"]["content"]
        .as_str()
        .and_then(|d| hex::decode(d.trim_start_matches("0x")).ok()))
}

/// Stores the service of a manifest cell: checks the manifest, the code cell data and the data cell data
/// against the hashes, recomputes the service ID and registers the service (a known service keeps its
/// status and gets the manifest fields).
pub fn store_manifest(
    db: &Db,
    raw: &RawManifest,
    code: &[u8],
    data: &[Vec<u8>],
) -> Result<Value, String> {
    let m = catalog::decode(&raw.data).map_err(|e| format!("invalid manifest: {e}"))?;
    if ckb_hash::blake2b_256(code) != m.code_hash {
        return Err(format!("the code cell {} does not hold the manifest's code hash", m.code_cell));
    }
    if data.len() != m.data_hashes.len() {
        return Err("data cells and data hashes differ in number".into());
    }
    for (i, d) in data.iter().enumerate() {
        if ckb_hash::blake2b_256(d) != m.data_hashes[i] {
            return Err(format!("the data cell {} does not hold its data hash", m.data_cells[i]));
        }
    }
    let id = format!("0x{}", hex::encode(m.service_id()));
    let language = if m.language.is_empty() {
        "onchain".to_string()
    } else {
        m.language.clone()
    };
    db.add_service(&id, &m.name, &language)?;
    db.set_service_outpoint(&id, &m.code_cell)?;
    db.set_service_manifest(
        &id,
        &m.data_cells,
        &hex::encode(&m.args),
        &raw.type_id,
        &raw.outpoint,
    )?;
    Ok(json!({"service_id": id, "code_hash": id, "name": m.name, "type_id": raw.type_id}))
}

/// Adds a service from its manifest cell: `target` is the manifest's Type ID (`0x` + 64 hex) or the out point
/// of the manifest cell. Reads the manifest, the code cell and the data cells from the node.
pub async fn add_from_manifest(
    db: &Db,
    rpc: &RpcClient,
    script: &ckb_jsonrpc_types::Script,
    network: &str,
    target: &str,
) -> Result<Value, String> {
    let type_id = if target.contains(':') {
        let cell = live_cell_type_args(rpc, target).await?;
        format!("0x{}", hex::encode(cell))
    } else {
        target.to_ascii_lowercase()
    };
    if type_id.len() != 66 || hex::decode(&type_id[2..]).is_err() {
        return Err("--manifest must be a Type ID (0x + 64 hex) or a manifest out point 0x<tx hash>:<index>".into());
    }
    let (cells, _) = catalog::read_manifests(rpc, script, network, Some(&type_id)).await?;
    let raw = cells
        .into_iter()
        .find(|c| c.type_id == type_id)
        .ok_or_else(|| format!("no live manifest with Type ID {type_id}"))?;
    let m = catalog::decode(&raw.data).map_err(|e| format!("invalid manifest: {e}"))?;
    let code = live_data(rpc, &m.code_cell)
        .await?
        .ok_or_else(|| format!("the code cell {} is not live", m.code_cell))?;
    let mut data = Vec::new();
    for c in &m.data_cells {
        data.push(
            live_data(rpc, c)
                .await?
                .ok_or_else(|| format!("data cell {c} is not live"))?,
        );
    }
    store_manifest(db, &raw, &code, &data)
}

/// The type script args of a live cell.
async fn live_cell_type_args(rpc: &RpcClient, outpoint: &str) -> Result<Vec<u8>, String> {
    let (tx_hash, index) = crate::chain::parse_out_point(outpoint)?;
    let cell = rpc
        .call(
            "get_live_cell",
            json!([{"tx_hash": tx_hash, "index": format!("0x{index:x}")}, false]),
        )
        .await
        .map_err(|e| format!("cannot read {outpoint}: {e}"))?;
    if cell["status"] != "live" {
        return Err(format!("{outpoint} is not a live cell"));
    }
    cell["cell"]["output"]["type"]["args"]
        .as_str()
        .and_then(|a| hex::decode(a.trim_start_matches("0x")).ok())
        .filter(|a| a.len() == 32)
        .ok_or_else(|| format!("{outpoint} is not a manifest cell"))
}

fn entry(row: &ServiceRow, storage_root: &Path, totals: &HashMap<String, (u64, u64)>) -> Value {
    let (jobs, earned) = totals.get(&row.code_hash).copied().unwrap_or((0, 0));
    json!({
        "code_hash": row.code_hash,
        "service_id": row.id(),
        "data_cells": row.data_cells,
        "args": row.args,
        "manifest_type_id": row.manifest_type_id,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{sample_manifest, sample_raw};

    fn composite() -> (RawManifest, Vec<u8>, Vec<Vec<u8>>) {
        let code = vec![1u8, 2, 3];
        let data = vec![vec![9u8, 9], vec![8u8]];
        let mut m = sample_manifest("js-app");
        m.code_hash = ckb_hash::blake2b_256(&code);
        m.args = vec![0xaa];
        m.data_hashes = data.iter().map(|d| ckb_hash::blake2b_256(d)).collect();
        m.data_cells = vec![format!("0x{}:1", "bb".repeat(32)), format!("0x{}:2", "cc".repeat(32))];
        (sample_raw(5, &m, 6), code, data)
    }

    #[test]
    fn add_from_manifest_stores_composite_fields() {
        let db = Db::open_memory();
        let (raw, code, data) = composite();
        let v = store_manifest(&db, &raw, &code, &data).unwrap();
        let id = v["service_id"].as_str().unwrap().to_string();
        let m = catalog::decode(&raw.data).unwrap();
        assert_eq!(id, format!("0x{}", hex::encode(m.service_id())));
        let row = db.service(&id).unwrap().unwrap();
        assert_eq!(row.status, NOT_APPROVED);
        assert_eq!(row.outpoint.as_deref(), Some(m.code_cell.as_str()));
        assert_eq!(row.data_cells, m.data_cells);
        assert_eq!((row.args.as_str(), row.manifest_type_id.as_deref()), ("aa", Some(raw.type_id.as_str())));
        // Wrong data is refused.
        assert!(store_manifest(&db, &raw, &code, &[vec![0], vec![8]]).is_err());
        assert!(store_manifest(&db, &raw, &[7], &data).is_err());
    }

    #[test]
    fn add_from_manifest_keeps_existing_status() {
        let db = Db::open_memory();
        let (raw, code, data) = composite();
        let id = store_manifest(&db, &raw, &code, &data).unwrap()["service_id"]
            .as_str()
            .unwrap()
            .to_string();
        db.set_service_status(&id, ENABLED).unwrap();
        // A new manifest cell (an update) is added again.
        let mut m = catalog::decode(&raw.data).unwrap();
        m.version = "2.0.0".into();
        let raw2 = sample_raw(5, &m, 8);
        store_manifest(&db, &raw2, &code, &data).unwrap();
        let row = db.service(&id).unwrap().unwrap();
        assert_eq!(row.status, ENABLED);
        assert_eq!(row.manifest_outpoint.as_deref(), Some(raw2.outpoint.as_str()));
        assert_eq!(db.services().unwrap().len(), 1);
    }

    #[test]
    fn add_from_manifest_approve_updates_allowed_services() {
        let db = Db::open_memory();
        let (raw, code, data) = composite();
        let id = store_manifest(&db, &raw, &code, &data).unwrap()["service_id"]
            .as_str()
            .unwrap()
            .to_string();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("csr.toml");
        let mut cfg = Config::default();
        let e = act(&db, &mut cfg, &path, dir.path(), &id, Action::Approve)
            .map_err(|e| e.message())
            .unwrap();
        assert_eq!(e["status"], ENABLED);
        assert_eq!(e["service_id"], id.as_str());
        assert_eq!(cfg.runtime.allowed_services, vec![id.clone()]);
        assert_eq!(Config::load(&path).unwrap().runtime.allowed_services, vec![id]);
    }
}
