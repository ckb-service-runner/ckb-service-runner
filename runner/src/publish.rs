//! `csr service publish`: the developer side of the service catalog. Builds, checks and sends the transaction
//! that creates, updates, deprecates or destroys a `csr-service-manifest` cell. It uses the publisher's own
//! key file, never the runner wallet.

use crate::catalog::{self, Manifest};
use crate::chain::{self, Cell, CKB};
use crate::config::Config;
use crate::node::rpc::{hex_u64, RpcClient, RpcError};
use ckb_hash::blake2b_256;
use ckb_jsonrpc_types as json;
use ckb_types::{
    bytes::Bytes,
    core::{TransactionBuilder, TransactionView},
    packed::{self, CellDep, CellInput, CellOutput, Script, WitnessArgs},
    prelude::*,
};
use serde_json::{json, Value};
use std::path::PathBuf;

pub enum Mode {
    Create,
    /// Update (or deprecate) the manifest with this Type ID.
    Update(String),
    /// Destroy the manifest with this Type ID.
    Destroy(String),
}

pub struct PublishArgs {
    pub mode: Mode,
    pub code_outpoint: Option<String>,
    pub data_outpoints: Vec<String>,
    pub args: Option<String>,
    pub name: Option<String>,
    pub version: Option<String>,
    pub description: Option<String>,
    pub description_file: Option<PathBuf>,
    pub url: Option<String>,
    pub source: Option<String>,
    pub language: Option<String>,
    pub min_csr: Option<String>,
    pub deprecate: bool,
    pub key: PathBuf,
    pub dry_run: bool,
}

struct Out {
    cap: u64,
    lock: Script,
    type_: Option<Script>,
    data: Vec<u8>,
}

fn hx(b: impl AsRef<[u8]>) -> String {
    format!("0x{}", hex::encode(b))
}

fn read_key(path: &PathBuf) -> Result<secp256k1::SecretKey, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read the key file {}: {e}", path.display()))?;
    let raw = hex::decode(text.trim().trim_start_matches("0x"))
        .map_err(|_| "the key file must hold the 32-byte secp256k1 key as hex".to_string())?;
    secp256k1::SecretKey::from_slice(&raw)
        .map_err(|_| "the key file must hold the 32-byte secp256k1 key as hex".to_string())
}

fn rpc_err(m: &str, e: RpcError) -> String {
    format!("{m}: {e}")
}

/// The live cell at `outpoint` (`0x<tx hash>:<index>`): lock, capacity, data.
async fn live(rpc: &RpcClient, outpoint: &str) -> Result<(Script, u64, Vec<u8>), String> {
    let (tx, i) = chain::parse_out_point(outpoint)?;
    let r = rpc
        .call(
            "get_live_cell",
            json!([{"tx_hash": tx, "index": format!("0x{i:x}")}, true]),
        )
        .await
        .map_err(|e| rpc_err("get_live_cell", e))?;
    if r["status"] != "live" {
        return Err(format!("{outpoint} is not a live cell"));
    }
    let lock: json::Script = serde_json::from_value(r["cell"]["output"]["lock"].clone())
        .map_err(|e| e.to_string())?;
    let cap = hex_u64(&r["cell"]["output"]["capacity"]).ok_or("bad capacity")?;
    let data = hex::decode(
        r["cell"]["data"]["content"]
            .as_str()
            .unwrap_or("")
            .trim_start_matches("0x"),
    )
    .map_err(|e| e.to_string())?;
    Ok((lock.into(), cap, data))
}

/// Plain cells (no type script, no data) of the publisher's lock.
async fn plain_cells(rpc: &RpcClient, lock: &Script) -> Result<Vec<Cell>, String> {
    catalog::wait_indexer(rpc).await?;
    let key = json!({
        "script": json::Script::from(lock.clone()),
        "script_type": "lock",
        "script_search_mode": "exact",
        "with_data": true,
    });
    let mut out = Vec::new();
    let mut cursor = Value::Null;
    for _ in 0..50 {
        let r = rpc
            .call("get_cells", json!([key, "asc", "0x200", cursor]))
            .await
            .map_err(|e| rpc_err("get_cells", e))?;
        let page = r["objects"].as_array().ok_or("get_cells: no objects")?;
        for c in page {
            if c["output"]["type"].is_null() && c["output_data"].as_str().is_none_or(|d| d == "0x")
            {
                out.push(chain::parse_cell(c)?);
            }
        }
        cursor = r["last_cursor"].clone();
        if page.len() < 0x200 || cursor.is_null() {
            break;
        }
    }
    Ok(out)
}

/// Builds and signs the transaction. Inputs: `first` (the manifest cell for an update) then publisher cells
/// as needed; outputs: `fixed` then the change. With `type_id`, `fixed[0]` is a new manifest whose Type ID
/// comes from the first input.
fn assemble(
    deps: Vec<CellDep>,
    first: Vec<Cell>,
    fixed: Vec<Out>,
    type_id: bool,
    pool: Vec<Cell>,
    owner: &Script,
    key: &secp256k1::SecretKey,
) -> Result<(TransactionView, u64), String> {
    let change_min = chain::occupied(owner, None, 0);
    let in_first: u64 = first.iter().map(|c| c.capacity).sum();
    let out_fixed: u64 = fixed.iter().map(|o| o.cap).sum();
    let need = (out_fixed + chain::FEE_RESERVE + change_min).saturating_sub(in_first);
    let mut picked: Vec<Cell> = Vec::new();
    let mut have = 0u64;
    for c in pool {
        if have >= need && (!first.is_empty() || !picked.is_empty()) {
            break;
        }
        have += c.capacity;
        picked.push(c);
    }
    let short = |have: u64, need: u64| {
        format!(
            "the publisher's lock holds {} CKB in plain cells, but this transaction needs {} CKB \
             (including the fee and {} CKB for the change cell)",
            have as f64 / CKB as f64,
            need as f64 / CKB as f64,
            change_min / CKB
        )
    };
    if have < need || (first.is_empty() && picked.is_empty()) {
        return Err(short(have, need));
    }
    let all: Vec<&Cell> = first.iter().chain(&picked).collect();
    let inputs: Vec<CellInput> = all
        .iter()
        .map(|c| CellInput::new(c.out_point.clone(), 0))
        .collect();
    let mut fixed = fixed;
    if type_id {
        let mut h = ckb_hash::new_blake2b();
        h.update(inputs[0].as_slice());
        h.update(&0u64.to_le_bytes());
        let mut id = [0u8; 32];
        h.finalize(&mut id);
        let t = fixed[0].type_.take().ok_or("manifest output has no type")?;
        fixed[0].type_ = Some(t.as_builder().args(Bytes::copy_from_slice(&id).pack()).build());
    }
    let mut witnesses = vec![Bytes::new(); inputs.len()];
    witnesses[0] = WitnessArgs::new_builder()
        .lock(Some(Bytes::from(vec![0u8; 65])).pack())
        .build()
        .as_bytes();
    let build = |change: u64| -> TransactionView {
        let mut b = TransactionBuilder::default()
            .cell_deps(deps.clone())
            .inputs(inputs.clone())
            .witnesses(witnesses.iter().map(|w| w.pack()).collect::<Vec<_>>());
        let change_out = Out {
            cap: change,
            lock: owner.clone(),
            type_: None,
            data: vec![],
        };
        for o in fixed.iter().chain([&change_out]) {
            b = b
                .output(
                    CellOutput::new_builder()
                        .capacity(Pack::<packed::Uint64>::pack(&o.cap))
                        .lock(o.lock.clone())
                        .type_(o.type_.clone().pack())
                        .build(),
                )
                .output_data(o.data.pack());
        }
        b.build()
    };
    let gross = in_first + have - out_fixed;
    // 2 shannons per byte plus 1,000
    let fee = build(gross).data().as_slice().len() as u64 * 2 + 1000;
    let change = gross.saturating_sub(fee);
    if change < change_min {
        return Err(short(have, need.max(change_min) + fee));
    }
    let tx = chain::sign_wallet_inputs(&build(change), inputs.len(), key)?;
    Ok((tx, fee))
}

/// Checks the transaction with the node (`test_tx_pool_accept`, else `dry_run_transaction`).
async fn check(rpc: &RpcClient, tx: &TransactionView) -> Result<(), String> {
    let t = json::TransactionView::from(tx.clone()).inner;
    match rpc
        .call("test_tx_pool_accept", json!([t, "passthrough"]))
        .await
    {
        Ok(_) => Ok(()),
        Err(e) if e.is_method_not_found() => rpc
            .call("dry_run_transaction", json!([t]))
            .await
            .map(|_| ())
            .map_err(|e| format!("the node would reject the transaction: {e}")),
        Err(e) => Err(format!("the node would reject the transaction: {e}")),
    }
}

fn arg_bytes(a: &Option<String>) -> Result<Vec<u8>, String> {
    hex::decode(
        a.as_deref()
            .unwrap_or("")
            .trim_start_matches("0x"),
    )
    .map_err(|_| "--args must be hex".to_string())
}

fn description(a: &PublishArgs) -> Result<Option<String>, String> {
    match (&a.description, &a.description_file) {
        (Some(_), Some(_)) => Err("use --description or --description-file, not both".into()),
        (Some(d), None) => Ok(Some(d.clone())),
        (None, Some(f)) => {
            let t = std::fs::read_to_string(f)
                .map_err(|e| format!("cannot read {}: {e}", f.display()))?;
            Ok(Some(t.trim_end().to_string()))
        }
        (None, None) => Ok(None),
    }
}

fn apply_text(m: &mut Manifest, a: &PublishArgs) -> Result<(), String> {
    if let Some(v) = &a.name {
        m.name = v.clone();
    }
    if let Some(v) = &a.version {
        m.version = v.clone();
    }
    if let Some(v) = description(a)? {
        m.description = v;
    }
    if let Some(v) = &a.url {
        m.url = v.clone();
    }
    if let Some(v) = &a.source {
        m.source = v.clone();
    }
    if let Some(v) = &a.language {
        m.language = v.clone();
    }
    if let Some(v) = &a.min_csr {
        m.min_csr = v.clone();
    }
    Ok(())
}

/// Encodes and re-checks the manifest with the same decoder the runners use.
fn encoded(m: &Manifest) -> Result<Vec<u8>, String> {
    let data = catalog::encode_manifest(m);
    catalog::decode(&data).map_err(|e| format!("invalid manifest: {e}"))?;
    Ok(data)
}

pub async fn run(cfg: &Config, a: &PublishArgs) -> Result<Value, String> {
    let (type_script, manifest_dep) = chain::manifest_deployment(cfg)?.ok_or(format!(
        "publishing is refused on {}: the manifest script is not deployed there yet",
        cfg.network
    ))?;
    let key = read_key(&a.key)?;
    let pubkey = secp256k1::PublicKey::from_secret_key(&secp256k1::Secp256k1::new(), &key);
    let owner = chain::sighash_lock(&blake2b_256(pubkey.serialize())[..20]);
    let rpc = RpcClient::new(&cfg.node.rpc_url);
    let secp = chain::secp_dep_of(&rpc).await?;
    let scan_script = json::Script::from(type_script.clone());
    // The new manifest's type script: the Type ID (32 bytes) is filled in once the first input is known.
    let type_script = type_script
        .as_builder()
        .args(Bytes::from(vec![0u8; 32]).pack())
        .build();
    let mut result = json!({"network": cfg.network, "dry_run": a.dry_run});

    let (tx, fee) = match &a.mode {
        Mode::Create => {
            let code_op = a.code_outpoint.as_deref().ok_or("--code-outpoint is required")?;
            if a.name.is_none() || a.version.is_none() {
                return Err("--name and --version are required".into());
            }
            let args = arg_bytes(&a.args)?;
            let (code_lock, _, code_data) = live(&rpc, code_op).await?;
            if code_lock != owner {
                return Err(format!(
                    "the key does not own the code cell {code_op}: its lock is not the key's \
                     secp256k1 lock (the manifest must be published by the lock that owns the code)"
                ));
            }
            let mut data_hashes = Vec::new();
            let mut deps = vec![manifest_dep, chain::code_dep(point(code_op)?)];
            for d in &a.data_outpoints {
                let (_, _, bytes) = live(&rpc, d).await?;
                data_hashes.push(blake2b_256(&bytes));
                deps.push(chain::code_dep(point(d)?));
            }
            deps.push(secp);
            let mut m = Manifest {
                code_hash: blake2b_256(&code_data),
                code_cell: normalise(code_op)?,
                data_hashes,
                data_cells: a
                    .data_outpoints
                    .iter()
                    .map(|d| normalise(d))
                    .collect::<Result<_, _>>()?,
                args,
                name: String::new(),
                version: String::new(),
                description: String::new(),
                url: String::new(),
                source: String::new(),
                language: String::new(),
                min_csr: String::new(),
                deprecated: false,
            };
            apply_text(&mut m, a)?;
            let data = encoded(&m)?;
            result["service_id"] = json!(hx(m.service_id()));
            let manifest_out = Out {
                cap: chain::occupied(&owner, Some(&type_script), data.len()),
                lock: owner.clone(),
                type_: Some(type_script.clone()),
                data,
            };
            let pool: Vec<Cell> = plain_cells(&rpc, &owner).await?;
            assemble(deps, vec![], vec![manifest_out], true, pool, &owner, &key)?
        }
        Mode::Update(id) | Mode::Destroy(id) => {
            let (raws, _) =
                catalog::read_manifests(&rpc, &scan_script, &cfg.network, Some(id)).await?;
            let raw = raws
                .into_iter()
                .find(|r| r.type_id.eq_ignore_ascii_case(id))
                .ok_or(format!("no live manifest with Type ID {id}"))?;
            let (lock, cap, _) = live(&rpc, &raw.outpoint).await?;
            if lock != owner {
                return Err("the key does not own this manifest (its lock is not the key's lock)".into());
            }
            let (txh, idx) = chain::parse_out_point(&raw.outpoint)?;
            let cell = Cell {
                out_point: chain::out_point_from(&txh, idx)?,
                capacity: cap,
                lock: owner.clone(),
                data: raw.data.clone(),
            };
            let mut m = catalog::decode(&raw.data)?;
            result["service_id"] = json!(hx(m.service_id()));
            let deps = vec![manifest_dep, secp];
            let pool = plain_cells(&rpc, &owner).await?;
            if let Mode::Destroy(_) = a.mode {
                assemble(deps, vec![cell], vec![], false, pool, &owner, &key)?
            } else {
                apply_text(&mut m, a)?;
                if a.deprecate {
                    m.deprecated = true;
                }
                let data = encoded(&m)?;
                let ty = Script::from(
                    serde_json::from_value::<json::Script>(
                        live_type(&rpc, &raw.outpoint).await?,
                    )
                    .map_err(|e| e.to_string())?,
                );
                let out = Out {
                    cap: cap.max(chain::occupied(&owner, Some(&ty), data.len())),
                    lock: owner.clone(),
                    type_: Some(ty),
                    data,
                };
                assemble(deps, vec![cell], vec![out], false, pool, &owner, &key)?
            }
        }
    };

    let out_type = tx.output(0).and_then(|o| o.type_().to_opt());
    if !matches!(a.mode, Mode::Destroy(_)) {
        let ty = out_type.ok_or("no manifest output")?;
        result["type_id"] = json!(hx(ty.args().raw_data()));
        result["manifest_outpoint"] = json!(format!("{}:0", hx(tx.hash().as_slice())));
    }
    if let Mode::Update(id) | Mode::Destroy(id) = &a.mode {
        result["type_id"] = json!(id);
    }
    result["tx_hash"] = json!(hx(tx.hash().as_slice()));
    result["fee_shannons"] = json!(fee);
    check(&rpc, &tx).await?;
    if !a.dry_run {
        let t = json::TransactionView::from(tx.clone()).inner;
        let sent = rpc
            .call("send_transaction", json!([t, "passthrough"]))
            .await
            .map_err(|e| format!("the node rejected the transaction: {e}"))?;
        result["tx_hash"] = sent;
    }
    Ok(result)
}

async fn live_type(rpc: &RpcClient, outpoint: &str) -> Result<Value, String> {
    let (tx, i) = chain::parse_out_point(outpoint)?;
    let r = rpc
        .call(
            "get_live_cell",
            json!([{"tx_hash": tx, "index": format!("0x{i:x}")}, false]),
        )
        .await
        .map_err(|e| rpc_err("get_live_cell", e))?;
    Ok(r["cell"]["output"]["type"].clone())
}

fn point(s: &str) -> Result<packed::OutPoint, String> {
    let (t, i) = chain::parse_out_point(s)?;
    chain::out_point_from(&t, i)
}

/// `0x<64 hex>:<index>` in lower case.
fn normalise(s: &str) -> Result<String, String> {
    let (t, i) = chain::parse_out_point(s)?;
    Ok(format!("{}:{i}", t.to_lowercase()))
}
