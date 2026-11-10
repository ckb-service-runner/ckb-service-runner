//! On-chain service catalog: reads `csr-service-manifest` cells from the runner's own node (D47) and
//! keeps them in the `catalog` table (`spec/service-manifest.md`). Information only: nothing runs without
//! the admin's click.

use crate::config::Config;
use crate::db::{CatalogRow, Db};
use crate::jobs::table_fields;
use crate::node::rpc::{hex_u64, RpcClient};
use ckb_hash::blake2b_256;
use ckb_jsonrpc_types as json;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

pub const LAST_SCAN: &str = "catalog_last_scan_ms";
pub const INITIALISED: &str = "catalog_initialised";
pub const UNSEEN: &str = "catalog_unseen";
pub const LAST_ERROR: &str = "catalog_last_error";
/// Retry after a skipped or failed scan.
pub const RETRY_MS: i64 = 30 * 60 * 1000;
pub const NO_SCRIPT: &str = "no manifest script in this deployment";

const MAX_DATA_CELLS: usize = 4;

/// The service ID (D49): the code hash for a single binary, else a hash over code hash, args and data hashes.
pub fn service_id(code_hash: &[u8; 32], args: &[u8], data_hashes: &[[u8; 32]]) -> [u8; 32] {
    if args.is_empty() && data_hashes.is_empty() {
        return *code_hash;
    }
    let mut m = b"CSR-SERVICE-v1".to_vec();
    m.extend_from_slice(code_hash);
    m.extend_from_slice(&(args.len() as u32).to_le_bytes());
    m.extend_from_slice(args);
    m.push(data_hashes.len() as u8);
    data_hashes.iter().for_each(|h| m.extend_from_slice(h));
    blake2b_256(&m)
}

fn hx(b: &[u8]) -> String {
    format!("0x{}", hex::encode(b))
}

/// A decoded and checked `ServiceManifestV1`.
#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    pub code_hash: [u8; 32],
    pub code_cell: String,
    pub data_hashes: Vec<[u8; 32]>,
    pub data_cells: Vec<String>,
    pub args: Vec<u8>,
    pub name: String,
    pub version: String,
    pub description: String,
    pub url: String,
    pub source: String,
    pub language: String,
    pub min_csr: String,
    pub deprecated: bool,
}

impl Manifest {
    pub fn service_id(&self) -> [u8; 32] {
        service_id(&self.code_hash, &self.args, &self.data_hashes)
    }
}

fn u32_at(d: &[u8], off: usize) -> Option<usize> {
    Some(u32::from_le_bytes(d.get(off..off + 4)?.try_into().ok()?) as usize)
}

/// A molecule `Bytes` field.
fn bytes_field(f: &[u8]) -> Result<&[u8], String> {
    let n = u32_at(f, 0).ok_or("truncated Bytes field")?;
    if f.len() != 4 + n {
        return Err("malformed Bytes field".into());
    }
    Ok(&f[4..])
}

/// A molecule vector of fixed-size items.
fn fixvec(f: &[u8], item: usize) -> Result<Vec<&[u8]>, String> {
    let n = u32_at(f, 0).ok_or("truncated vector")?;
    if f.len() != 4 + n * item {
        return Err("malformed vector".into());
    }
    Ok(f[4..].chunks(item).collect())
}

fn out_point(b: &[u8]) -> String {
    format!(
        "{}:{}",
        hx(&b[..32]),
        u32::from_le_bytes(b[32..36].try_into().expect("4 bytes"))
    )
}

fn text(f: &[u8], what: &str, min: usize, max: usize) -> Result<String, String> {
    let b = bytes_field(f)?;
    if b.len() < min || b.len() > max {
        return Err(format!("{what} must be {min}..{max} bytes"));
    }
    String::from_utf8(b.to_vec()).map_err(|_| format!("{what} is not UTF-8"))
}

fn link(f: &[u8], what: &str) -> Result<String, String> {
    let s = text(f, what, 0, 128)?;
    if !s.is_empty() && !s.starts_with("https://") {
        return Err(format!("{what} is not an https:// URL"));
    }
    Ok(s)
}

fn version_parts(v: &str) -> Option<[u64; 3]> {
    let mut it = v.split('.');
    let mut out = [0u64; 3];
    for o in out.iter_mut() {
        let p = it.next()?;
        if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        *o = p.parse().ok()?;
    }
    it.next().is_none().then_some(out)
}

/// Decodes the data of a manifest cell and re-checks every rule the runner can check
/// (spec §2, §3 rules 1 and 4–7 of the table; creation-time cell dep rules are the contract's).
pub fn decode(data: &[u8]) -> Result<Manifest, String> {
    let f = table_fields(data, 14).ok_or("data is not a well-formed ServiceManifestV1")?;
    if f[0] != [1] {
        return Err("version_tag is not 1".into());
    }
    if f[1].len() != 32 || f[2].len() != 36 || f[13].len() != 1 {
        return Err("data is not a well-formed ServiceManifestV1".into());
    }
    let data_hashes: Vec<[u8; 32]> = fixvec(f[3], 32)?
        .into_iter()
        .map(|h| h.try_into().expect("32 bytes"))
        .collect();
    let data_cells: Vec<String> = fixvec(f[4], 36)?.into_iter().map(out_point).collect();
    if data_hashes.len() != data_cells.len() || data_hashes.len() > MAX_DATA_CELLS {
        return Err("data_hashes and data_cells differ in length or there are more than 4".into());
    }
    let args = bytes_field(f[5])?.to_vec();
    if args.len() > 64 {
        return Err("args must be at most 64 bytes".into());
    }
    let min_csr = text(f[12], "min_csr", 0, 16)?;
    if !min_csr.is_empty() && version_parts(&min_csr).is_none() {
        return Err("min_csr is not X.Y.Z".into());
    }
    if f[13][0] > 1 {
        return Err("deprecated is not 0 or 1".into());
    }
    Ok(Manifest {
        code_hash: f[1].try_into().expect("32 bytes"),
        code_cell: out_point(f[2]),
        data_hashes,
        data_cells,
        args,
        name: text(f[6], "name", 1, 48)?,
        version: text(f[7], "version", 1, 16)?,
        description: text(f[8], "description", 0, 512)?,
        url: link(f[9], "url")?,
        source: link(f[10], "source")?,
        language: text(f[11], "language", 0, 16)?,
        min_csr,
        deprecated: f[13][0] == 1,
    })
}

/// Can a runner of version `current` run a service that needs `min_csr`?
pub fn runnable(min_csr: &str, current: &str) -> Result<(), String> {
    if min_csr.is_empty() {
        return Ok(());
    }
    match (version_parts(min_csr), version_parts(current)) {
        (Some(need), Some(have)) if have < need => Err(format!(
            "needs csr {min_csr} or newer, this runner is {current}"
        )),
        _ => Ok(()),
    }
}

/// A live manifest cell as read from the node.
#[derive(Debug, Clone)]
pub struct RawManifest {
    /// `0x` + 64 hex: the script args of the cell.
    pub type_id: String,
    pub outpoint: String,
    pub lock_hash: String,
    pub address: String,
    pub data: Vec<u8>,
}

fn parse_raw(c: &Value, network: &str) -> Option<RawManifest> {
    let ty: json::Script = serde_json::from_value(c["output"]["type"].clone()).ok()?;
    let ty: ckb_types::packed::Script = ty.into();
    let args = ckb_types::prelude::Unpack::<ckb_types::bytes::Bytes>::unpack(&ty.args());
    if args.len() != 32 {
        return None;
    }
    let lock: json::Script = serde_json::from_value(c["output"]["lock"].clone()).ok()?;
    let lock: ckb_types::packed::Script = lock.into();
    let lock_hash = hx(ckb_types::prelude::Entity::as_slice(&lock.calc_script_hash()));
    let ch: [u8; 32] = ckb_types::prelude::Entity::as_slice(&lock.code_hash())
        .try_into()
        .ok()?;
    let la = ckb_types::prelude::Unpack::<ckb_types::bytes::Bytes>::unpack(&lock.args()).to_vec();
    let hash_type = u8::from(lock.hash_type());
    let address = crate::address::encode(
        network,
        &crate::address::LockScript {
            code_hash: ch,
            hash_type,
            args: la,
        },
    );
    let index = hex_u64(&c["out_point"]["index"])?;
    Some(RawManifest {
        type_id: hx(&args),
        outpoint: format!("{}:{index}", c["out_point"]["tx_hash"].as_str()?),
        lock_hash,
        address,
        data: hex::decode(c["output_data"].as_str()?.trim_start_matches("0x")).ok()?,
    })
}

async fn wait_indexer(rpc: &RpcClient) -> Result<(), String> {
    for _ in 0..100 {
        let tip = rpc
            .call("get_tip_header", json!([]))
            .await
            .map_err(|e| format!("get_tip_header: {e}"))?;
        let it = rpc
            .call("get_indexer_tip", json!([]))
            .await
            .map_err(|e| format!("get_indexer_tip: {e}"))?;
        if let (Some(t), Some(i)) = (hex_u64(&tip["number"]), hex_u64(&it["block_number"])) {
            if crate::chain::indexer_ready(t, i) {
                return Ok(());
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    Err("the node's indexer does not catch up with the tip".into())
}

/// Reads the live cells with the manifest type script (indexer `get_cells`, prefix search on the code
/// hash; with `type_id` only that manifest). `complete` is false when the cursor had not ended.
pub async fn read_manifests(
    rpc: &RpcClient,
    script: &json::Script,
    network: &str,
    type_id: Option<&str>,
) -> Result<(Vec<RawManifest>, bool), String> {
    wait_indexer(rpc).await?;
    let mut script = serde_json::to_value(script).map_err(|e| e.to_string())?;
    if let Some(t) = type_id {
        script["args"] = json!(t);
    }
    let key = json!({
        "script": script,
        "script_type": "type",
        "script_search_mode": "prefix",
        "with_data": true,
    });
    let mut out = Vec::new();
    let mut cursor = Value::Null;
    let mut complete = false;
    for _ in 0..200 {
        let r = rpc
            .call("get_cells", json!([key, "asc", "0x400", cursor]))
            .await
            .map_err(|e| format!("get_cells: {e}"))?;
        let page = r["objects"].as_array().ok_or("get_cells: no objects")?;
        out.extend(page.iter().filter_map(|c| parse_raw(c, network)));
        cursor = r["last_cursor"].clone();
        if page.len() < 0x400 || cursor.is_null() {
            complete = true;
            break;
        }
    }
    Ok((out, complete))
}

/// Builds the catalog row of a manifest cell; `Err` if the cell breaks a rule.
pub fn row_of(raw: &RawManifest, now_ms: i64) -> Result<CatalogRow, String> {
    let m = decode(&raw.data)?;
    Ok(CatalogRow {
        type_id: raw.type_id.clone(),
        service_id: hx(&m.service_id()),
        code_hash: hx(&m.code_hash),
        outpoint: raw.outpoint.clone(),
        publisher_lock_hash: raw.lock_hash.clone(),
        publisher_address: raw.address.clone(),
        name: m.name,
        version: m.version,
        description: m.description,
        url: m.url,
        source: m.source,
        language: m.language,
        min_csr: m.min_csr,
        deprecated: m.deprecated,
        code_cell: m.code_cell,
        data_cells: m.data_cells,
        args: hex::encode(&m.args),
        first_seen_ms: now_ms,
        last_seen_ms: now_ms,
        removed: false,
    })
}

#[derive(Debug, Default, PartialEq)]
pub struct ScanReport {
    pub live: usize,
    pub invalid: usize,
    /// Type IDs that are new since the last scan (and not marked seen by a first scan).
    pub new: Vec<String>,
}

pub fn unseen(db: &Db) -> Vec<String> {
    db.get_setting(UNSEEN)
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default()
}

pub fn clear_unseen(db: &Db) -> Result<(), String> {
    db.put_setting(UNSEEN, "[]")
}

/// Upserts the scanned manifests, marks the entries no longer live as removed (only when the list is
/// `complete`: a cut-off list never marks anything removed) and records the new ones as unseen. The first
/// scan after installing marks every entry as seen (D51).
pub fn apply_scan(
    db: &Db,
    cells: &[RawManifest],
    complete: bool,
    now_ms: i64,
) -> Result<ScanReport, String> {
    let initialised = db.get_setting(INITIALISED)?.is_some();
    let known: HashMap<String, i64> = db
        .catalog()?
        .into_iter()
        .map(|r| (r.type_id, r.first_seen_ms))
        .collect();
    let mut report = ScanReport::default();
    let mut live = Vec::new();
    for raw in cells {
        let mut row = match row_of(raw, now_ms) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("catalog: skipping manifest cell {}: {e}", raw.outpoint);
                report.invalid += 1;
                continue;
            }
        };
        match known.get(&row.type_id) {
            Some(first) => row.first_seen_ms = *first,
            None if initialised && !report.new.contains(&row.type_id) => {
                report.new.push(row.type_id.clone())
            }
            None => {}
        }
        db.upsert_catalog(&row)?;
        live.push(row.type_id);
    }
    report.live = live.len();
    if complete {
        db.mark_catalog_removed(&live)?;
    }
    let mut list = unseen(db);
    for t in &report.new {
        if !list.contains(t) {
            list.push(t.clone());
        }
    }
    if complete {
        let live: HashSet<&String> = live.iter().collect();
        list.retain(|t| live.contains(t));
    }
    db.put_setting(UNSEEN, &serde_json::to_string(&list).unwrap_or_else(|_| "[]".into()))?;
    db.put_setting(INITIALISED, "1")?;
    db.put_setting(LAST_SCAN, &now_ms.to_string())?;
    Ok(report)
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Why the catalog is off (`None` = on).
pub fn off_reason(cfg: &Config) -> Result<Option<String>, String> {
    if !cfg.catalog.enabled {
        return Ok(Some("catalog disabled in the config".into()));
    }
    Ok(match crate::chain::manifest_script(cfg)? {
        Some(_) => None,
        None => Some(NO_SCRIPT.into()),
    })
}

/// One scan of the node. The caller checks the R2 guard.
pub async fn scan(db: &Db, cfg: &Config) -> Result<ScanReport, String> {
    let script = crate::chain::manifest_script(cfg)?.ok_or(NO_SCRIPT)?;
    let rpc = RpcClient::new(&cfg.node.rpc_url);
    let (cells, complete) = read_manifests(&rpc, &script, &cfg.network, None).await?;
    if !complete {
        tracing::warn!("catalog: more than 200 pages of manifest cells, the list is incomplete");
    }
    apply_scan(db, &cells, complete, now_ms())
}

/// Scans once if no scan runs (`Err("busy")` otherwise); needs the R2 guard `Synced`. The outcome is
/// recorded in the settings (`catalog_last_error`).
pub async fn scan_guarded(st: &crate::app::AppState, cfg: &Config) -> Result<ScanReport, String> {
    use std::sync::atomic::Ordering;
    if st.catalog_busy.swap(true, Ordering::AcqRel) {
        return Err("busy".into());
    }
    struct Reset<'a>(&'a std::sync::atomic::AtomicBool);
    impl Drop for Reset<'_> {
        fn drop(&mut self) {
            self.0.store(false, Ordering::Release);
        }
    }
    let _reset = Reset(&st.catalog_busy);
    let r2 = st.node.read().unwrap().report.state;
    let res = if r2 != crate::node::r2::R2State::Synced {
        Err(format!("node not synced (R2: {})", r2.as_str()))
    } else {
        scan(&st.db, cfg).await
    };
    let _ = st
        .db
        .put_setting(LAST_ERROR, res.as_ref().err().map_or("", |e| e.as_str()));
    match &res {
        Ok(r) => tracing::info!(
            "service catalog: {} manifest(s) on chain, {} new, {} invalid",
            r.live,
            r.new.len(),
            r.invalid
        ),
        Err(_) => {}
    }
    res
}

fn added_by_id(db: &Db) -> Result<HashMap<String, crate::db::ServiceRow>, String> {
    Ok(db
        .services()?
        .into_iter()
        .map(|s| (s.id().to_ascii_lowercase(), s))
        .collect())
}

/// The catalog entries with the local state of each service and the open job cells that use it.
pub fn items(db: &Db, csr_version: &str) -> Result<Vec<Value>, String> {
    let added = added_by_id(db)?;
    let open = db.open_by_service()?;
    Ok(db
        .catalog()?
        .into_iter()
        .filter(|r| !r.removed)
        .map(|r| {
            let svc = added.get(&r.service_id.to_ascii_lowercase());
            let (jobs, tips) = open.get(&r.service_id.to_ascii_lowercase()).copied().unwrap_or((0, 0));
            let reason = runnable(&r.min_csr, csr_version).err();
            json!({
                "type_id": r.type_id,
                "service_id": r.service_id,
                "code_hash": r.code_hash,
                "outpoint": r.outpoint,
                "publisher_lock_hash": r.publisher_lock_hash,
                "publisher_address": r.publisher_address,
                "name": r.name,
                "version": r.version,
                "description": r.description,
                "url": r.url,
                "source": r.source,
                "language": r.language,
                "min_csr": r.min_csr,
                "deprecated": r.deprecated,
                "code_cell": r.code_cell,
                "data_cells": r.data_cells,
                "args": r.args,
                "first_seen_ms": r.first_seen_ms,
                "last_seen_ms": r.last_seen_ms,
                "added": svc.is_some(),
                "status": svc.map(|s| s.status.clone()),
                "update_available": svc.is_some_and(|s| {
                    s.manifest_type_id.as_deref() == Some(r.type_id.as_str())
                        && s.manifest_outpoint.as_deref() != Some(r.outpoint.as_str())
                }),
                "open_jobs": jobs,
                "tips_waiting_shannons": tips,
                "runnable": reason.is_none(),
                "runnable_reason": reason,
            })
        })
        .collect())
}

/// Service IDs of open job cells that have no catalog entry and are not added locally.
pub fn unknown(db: &Db) -> Result<Vec<Value>, String> {
    let added = added_by_id(db)?;
    let listed: HashSet<String> = db
        .catalog()?
        .into_iter()
        .filter(|r| !r.removed)
        .map(|r| r.service_id.to_ascii_lowercase())
        .collect();
    let mut out: Vec<_> = db
        .open_by_service()?
        .into_iter()
        .filter(|(id, _)| !listed.contains(id) && !added.contains_key(id))
        .map(|(id, (jobs, tips))| {
            json!({"service_id": id, "open_jobs": jobs, "tips_waiting_shannons": tips})
        })
        .collect();
    out.sort_by(|a, b| a["service_id"].as_str().cmp(&b["service_id"].as_str()));
    Ok(out)
}

/// `{unseen, newest}` for `/api/status`.
pub fn status_json(db: &Db) -> Value {
    let unseen = unseen(db);
    let rows = db.catalog().unwrap_or_default();
    let newest = unseen
        .iter()
        .rev()
        .find_map(|t| rows.iter().find(|r| &r.type_id == t && !r.removed))
        .map(|r| json!({"name": r.name, "type_id": r.type_id}));
    json!({"unseen": unseen.len(), "newest": newest})
}

/// The body of `GET /api/catalog` and `csr service catalog --json`.
pub fn catalog_json(db: &Db, cfg: &Config) -> Result<Value, String> {
    let reason = off_reason(cfg).unwrap_or_else(|e| Some(e));
    let last: Option<i64> = db.get_setting(LAST_SCAN)?.and_then(|v| v.parse().ok());
    let next = reason
        .is_none()
        .then(|| last.map_or_else(now_ms, |l| l + cfg.catalog.scan_hours as i64 * 3_600_000));
    Ok(json!({
        "enabled": reason.is_none(),
        "reason": reason,
        "last_scan_ms": last,
        "next_scan_ms": next,
        "last_error": db.get_setting(LAST_ERROR)?.filter(|e| !e.is_empty()),
        "unseen": unseen(db).len(),
        "items": items(db, env!("CARGO_PKG_VERSION"))?,
        "unknown": unknown(db)?,
    }))
}

#[cfg(test)]
pub fn encode_manifest(m: &Manifest) -> Vec<u8> {
    fn bytes(b: &[u8]) -> Vec<u8> {
        [(b.len() as u32).to_le_bytes().to_vec(), b.to_vec()].concat()
    }
    fn vecof(items: Vec<Vec<u8>>) -> Vec<u8> {
        let n = items.len() as u32;
        [n.to_le_bytes().to_vec(), items.concat()].concat()
    }
    fn op(s: &str) -> Vec<u8> {
        crate::jobs::outpoint_bytes(s).unwrap().to_vec()
    }
    let fields: Vec<Vec<u8>> = vec![
        vec![1],
        m.code_hash.to_vec(),
        op(&m.code_cell),
        vecof(m.data_hashes.iter().map(|h| h.to_vec()).collect()),
        vecof(m.data_cells.iter().map(|c| op(c)).collect()),
        bytes(&m.args),
        bytes(m.name.as_bytes()),
        bytes(m.version.as_bytes()),
        bytes(m.description.as_bytes()),
        bytes(m.url.as_bytes()),
        bytes(m.source.as_bytes()),
        bytes(m.language.as_bytes()),
        bytes(m.min_csr.as_bytes()),
        vec![m.deprecated as u8],
    ];
    let header = 4 + 4 * fields.len();
    let mut out = ((header + fields.iter().map(Vec::len).sum::<usize>()) as u32)
        .to_le_bytes()
        .to_vec();
    let mut off = header;
    for f in &fields {
        out.extend_from_slice(&(off as u32).to_le_bytes());
        off += f.len();
    }
    fields.iter().for_each(|f| out.extend_from_slice(f));
    out
}

#[cfg(test)]
pub fn sample_manifest(name: &str) -> Manifest {
    Manifest {
        code_hash: [0x11; 32],
        code_cell: format!("0x{}:0", "aa".repeat(32)),
        data_hashes: vec![],
        data_cells: vec![],
        args: vec![],
        name: name.into(),
        version: "1.0.0".into(),
        description: "does things".into(),
        url: "https://example.org".into(),
        source: "".into(),
        language: "rust".into(),
        min_csr: "".into(),
        deprecated: false,
    }
}

#[cfg(test)]
pub fn sample_raw(type_byte: u8, m: &Manifest, outpoint_byte: u8) -> RawManifest {
    RawManifest {
        type_id: hx(&[type_byte; 32]),
        outpoint: format!("{}:0", hx(&[outpoint_byte; 32])),
        lock_hash: hx(&[9; 32]),
        address: String::new(),
        data: encode_manifest(m),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(t: u8, name: &str) -> RawManifest {
        sample_raw(t, &sample_manifest(name), t)
    }

    #[test]
    fn decode_roundtrip() {
        let mut m = sample_manifest("a");
        m.args = vec![1, 2];
        m.data_hashes = vec![[3; 32]];
        m.data_cells = vec![format!("0x{}:1", "bb".repeat(32))];
        assert_eq!(decode(&encode_manifest(&m)).unwrap(), m);
    }

    #[test]
    fn decode_rejects_oversized_fields() {
        let mut m = sample_manifest("a");
        m.name = "n".repeat(49);
        assert!(decode(&encode_manifest(&m)).is_err());
        let mut m = sample_manifest("");
        assert!(decode(&encode_manifest(&m)).is_err());
        m.name = "a".into();
        m.description = "d".repeat(513);
        assert!(decode(&encode_manifest(&m)).is_err());
        m.description = "d".repeat(512);
        assert!(decode(&encode_manifest(&m)).is_ok());
        m.args = vec![0; 65];
        assert!(decode(&encode_manifest(&m)).is_err());
        let mut m = sample_manifest("a");
        m.data_hashes = vec![[1; 32]; 5];
        m.data_cells = vec![format!("0x{}:1", "bb".repeat(32)); 5];
        assert!(decode(&encode_manifest(&m)).is_err());
    }

    #[test]
    fn decode_rejects_non_https_links() {
        let mut m = sample_manifest("a");
        m.url = "http://example.org".into();
        assert!(decode(&encode_manifest(&m)).is_err());
        m.url = "".into();
        m.source = "javascript:alert(1)".into();
        assert!(decode(&encode_manifest(&m)).is_err());
        m.source = "https://github.com/x/y".into();
        assert!(decode(&encode_manifest(&m)).is_ok());
        let mut m = sample_manifest("a");
        m.min_csr = "1.x".into();
        assert!(decode(&encode_manifest(&m)).is_err());
    }

    #[test]
    fn scan_upserts_and_marks_removed() {
        let db = Db::open_memory();
        apply_scan(&db, &[raw(1, "one"), raw(2, "two")], true, 100).unwrap();
        assert_eq!(db.catalog().unwrap().len(), 2);
        // An invalid cell is skipped.
        let mut bad = raw(3, "bad");
        bad.data.push(0);
        let r = apply_scan(&db, &[raw(1, "one-b"), bad], true, 200).unwrap();
        assert_eq!((r.live, r.invalid), (1, 1));
        let rows = db.catalog().unwrap();
        let one = rows.iter().find(|r| r.name == "one-b").unwrap();
        assert_eq!((one.first_seen_ms, one.last_seen_ms, one.removed), (100, 200, false));
        assert!(rows.iter().find(|r| r.name == "two").unwrap().removed);
        // It comes back.
        apply_scan(&db, &[raw(2, "two")], true, 300).unwrap();
        assert!(!db.catalog().unwrap().iter().find(|r| r.name == "two").unwrap().removed);
    }

    #[test]
    fn cut_off_list_never_marks_removed() {
        let db = Db::open_memory();
        apply_scan(&db, &[raw(1, "one"), raw(2, "two")], true, 100).unwrap();
        apply_scan(&db, &[raw(1, "one")], false, 200).unwrap();
        assert!(db.catalog().unwrap().iter().all(|r| !r.removed));
    }

    #[test]
    fn first_scan_marks_all_seen() {
        let db = Db::open_memory();
        let r = apply_scan(&db, &[raw(1, "one"), raw(2, "two")], true, 100).unwrap();
        assert!(r.new.is_empty());
        assert!(unseen(&db).is_empty());
        assert_eq!(status_json(&db)["unseen"], 0);
        assert!(db.get_setting(INITIALISED).unwrap().is_some());
    }

    #[test]
    fn new_manifest_after_first_scan_is_unseen() {
        let db = Db::open_memory();
        apply_scan(&db, &[raw(1, "one")], true, 100).unwrap();
        let r = apply_scan(&db, &[raw(1, "one"), raw(2, "two")], true, 200).unwrap();
        assert_eq!(r.new, vec![hx(&[2; 32])]);
        let s = status_json(&db);
        assert_eq!(s["unseen"], 1);
        assert_eq!(s["newest"]["name"], "two");
        // A further scan does not repeat it; clearing removes it.
        apply_scan(&db, &[raw(1, "one"), raw(2, "two")], true, 300).unwrap();
        assert_eq!(unseen(&db).len(), 1);
        clear_unseen(&db).unwrap();
        assert_eq!(status_json(&db)["unseen"], 0);
        assert!(status_json(&db)["newest"].is_null());
    }

    #[test]
    fn update_of_added_service_is_not_a_notice() {
        let db = Db::open_memory();
        let m = sample_manifest("one");
        let first = sample_raw(1, &m, 1);
        apply_scan(&db, &[first.clone()], true, 100).unwrap();
        let id = hx(&m.service_id());
        db.add_service(&id, "one", "onchain").unwrap();
        db.set_service_manifest(&id, &[], "", &first.type_id, &first.outpoint).unwrap();
        // Same Type ID, new cell and version.
        let mut m2 = m.clone();
        m2.version = "1.1.0".into();
        let second = sample_raw(1, &m2, 7);
        let r = apply_scan(&db, &[second], true, 200).unwrap();
        assert!(r.new.is_empty());
        assert!(unseen(&db).is_empty());
        let it = items(&db, "0.6.0").unwrap();
        assert_eq!(it.len(), 1);
        assert_eq!(it[0]["added"], true);
        assert_eq!(it[0]["update_available"], true);
        assert_eq!(it[0]["version"], "1.1.0");
    }

    #[test]
    fn min_csr_above_version_is_not_runnable() {
        assert!(runnable("", "0.5.3").is_ok());
        assert!(runnable("0.5.3", "0.5.3").is_ok());
        assert!(runnable("0.5.4", "0.5.3").is_err());
        assert!(runnable("0.10.0", "0.9.9").is_err());
        let db = Db::open_memory();
        let mut m = sample_manifest("new");
        m.min_csr = "9.0.0".into();
        apply_scan(&db, &[sample_raw(1, &m, 1)], true, 1).unwrap();
        let it = items(&db, "0.6.0").unwrap();
        assert_eq!(it[0]["runnable"], false);
        assert!(it[0]["runnable_reason"].as_str().unwrap().contains("9.0.0"));
    }

    #[test]
    fn unknown_lists_job_service_ids_without_manifest() {
        use crate::db::JobCellRow;
        let db = Db::open_memory();
        let m = sample_manifest("one");
        apply_scan(&db, &[sample_raw(1, &m, 1)], true, 1).unwrap();
        db.add_service(&hx(&[0x33; 32]), "local", "onchain").unwrap();
        let job = |op: &str, id: &[u8; 32], tip: u64| JobCellRow {
            outpoint: op.into(),
            code_hash: hx(id),
            tip_shannons: tip,
            max_cycles: 1,
            trigger: "ondemand".into(),
            state: "excluded".into(),
            reason: None,
        };
        db.replace_job_cells(
            &[
                job("a", &[0x11; 32], 5),
                job("b", &[0x44; 32], 7),
                job("c", &[0x44; 32], 8),
                job("d", &[0x33; 32], 1),
            ],
            1,
        )
        .unwrap();
        let u = unknown(&db).unwrap();
        assert_eq!(u.len(), 1);
        assert_eq!(u[0]["service_id"], hx(&[0x44; 32]));
        assert_eq!((u[0]["open_jobs"].as_u64(), u[0]["tips_waiting_shannons"].as_u64()), (Some(2), Some(15)));
        let it = items(&db, "0.6.0").unwrap();
        assert_eq!((it[0]["open_jobs"].as_u64(), it[0]["tips_waiting_shannons"].as_u64()), (Some(1), Some(5)));
    }
}

#[cfg(test)]
#[path = "../../acceptance/aiworker-csr-service-catalog/service_id_tests.rs"]
mod aiworker_service_id_tests;
