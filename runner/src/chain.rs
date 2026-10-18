//! On-chain operator flows: runner key cell, float cell and transaction submission.
//!
//! CSR never holds the owner's key. The `prepare_*` functions build unsigned transactions (ckb-cli
//! "tx" file format); the owner signs them in their wallet and `submit` broadcasts the result.

use crate::config::Config;
use crate::node::r2::expected_chain;
use crate::node::rpc::{hex_u64, RpcClient, RpcError};
use ckb_hash::blake2b_256;
use ckb_jsonrpc_types as json;
use ckb_types::{
    bytes::Bytes,
    core::{DepType, ScriptHashType, TransactionBuilder, TransactionView},
    h256,
    packed::{self, CellDep, CellInput, CellOutput, OutPoint, Script, WitnessArgs},
    prelude::*,
    H256,
};
use serde_json::{json, Value};
use std::path::Path;

pub const CKB: u64 = 100_000_000;
/// Default `max_fee_shannons` in the float args (0.01 CKB). The float lock ignores it (reserved).
pub const DEFAULT_MAX_FEE: u64 = 1_000_000;
/// Fee reserve when picking the owner's cells; the exact fee comes out of the change.
const FEE_RESERVE: u64 = 1_000_000;

const SIGHASH_CODE_HASH: H256 =
    h256!("0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8");

pub fn sighash_lock(args: &[u8]) -> Script {
    Script::new_builder()
        .code_hash(SIGHASH_CODE_HASH.pack())
        .hash_type(packed::Byte::new(ScriptHashType::Type as u8))
        .args(Bytes::copy_from_slice(args).pack())
        .build()
}

fn is_sighash(s: &Script) -> bool {
    s.code_hash() == SIGHASH_CODE_HASH.pack()
        && s.hash_type() == packed::Byte::new(ScriptHashType::Type as u8)
        && s.args().raw_data().len() == 20
}

fn script_hash(s: &Script) -> H256 {
    s.calc_script_hash().unpack()
}

fn hx(b: impl AsRef<[u8]>) -> String {
    format!("0x{}", hex::encode(b))
}

/// Occupied capacity in shannons (1 byte = 1 CKB shannon-unit of 1e8).
fn occupied(lock: &Script, type_: Option<&Script>, data_len: usize) -> u64 {
    let script = |s: &Script| 32 + 1 + s.args().raw_data().len() as u64;
    (8 + script(lock) + type_.map(script).unwrap_or(0) + data_len as u64) * CKB
}

/// `N` or `N.NNNNNNNN` CKB to shannons.
pub fn ckb_to_shannons(s: &str) -> Result<u64, String> {
    let bad = || format!("bad CKB amount '{s}'");
    let (int, frac) = s.split_once('.').unwrap_or((s, ""));
    if frac.len() > 8 || (int.is_empty() && frac.is_empty()) {
        return Err(bad());
    }
    let int: u64 = if int.is_empty() {
        0
    } else {
        int.parse().map_err(|_| bad())?
    };
    let frac: u64 = if frac.is_empty() {
        0
    } else {
        format!("{frac:0<8}").parse().map_err(|_| bad())?
    };
    int.checked_mul(CKB)
        .and_then(|v| v.checked_add(frac))
        .ok_or_else(bad)
}

pub fn parse_lock_args(s: &str) -> Result<Vec<u8>, String> {
    let b = hex::decode(s.trim().trim_start_matches("0x"))
        .map_err(|_| "owner lock args are not hex".to_string())?;
    if b.len() != 20 {
        return Err("owner lock args must be 20 bytes (secp256k1_blake160)".into());
    }
    Ok(b)
}

fn out_point_str(op: &OutPoint) -> String {
    let i: u32 = op.index().unpack();
    format!("{}:{i}", hx(op.tx_hash().as_slice()))
}

fn out_point_from(tx_hash: &str, index: u32) -> Result<OutPoint, String> {
    let hash: H256 = tx_hash
        .trim_start_matches("0x")
        .parse()
        .map_err(|e| format!("bad tx hash {tx_hash}: {e}"))?;
    Ok(OutPoint::new_builder()
        .tx_hash(hash.pack())
        .index(Pack::<packed::Uint32>::pack(&index))
        .build())
}

#[derive(Clone)]
pub struct Cell {
    pub out_point: OutPoint,
    pub capacity: u64,
    pub lock: Script,
    pub data: Vec<u8>,
}

fn parse_cell(c: &Value) -> Result<Cell, String> {
    let lock: json::Script =
        serde_json::from_value(c["output"]["lock"].clone()).map_err(|e| e.to_string())?;
    Ok(Cell {
        out_point: out_point_from(
            c["out_point"]["tx_hash"].as_str().unwrap_or(""),
            hex_u64(&c["out_point"]["index"]).ok_or("bad cell index")? as u32,
        )?,
        capacity: hex_u64(&c["output"]["capacity"]).ok_or("bad cell capacity")?,
        lock: lock.into(),
        data: c["output_data"]
            .as_str()
            .map(|d| hex::decode(d.trim_start_matches("0x")).unwrap_or_default())
            .unwrap_or_default(),
    })
}

/// A deployed script: how to reference it and where its code cell is.
struct Deployed {
    code_hash: H256,
    hash_type: ScriptHashType,
    code_cell: OutPoint,
}

struct Deployments {
    float: Deployed,
    dual: Deployed,
    registry: Deployed,
    job_type: Deployed,
    job_lock: Deployed,
    /// Type hash of the registry script of this deployment (args of the job lock).
    registry_type_hash: Vec<u8>,
    /// Args of the Type ID type script of the monitor config cell.
    config_type_args: Vec<u8>,
    config_type_hash: H256,
}

fn load_deployed(v: &Value, name: &str, file: &str) -> Result<Deployed, String> {
    let s = &v["scripts"][name];
    let code_hash: H256 = s["code_hash"]
        .as_str()
        .unwrap_or("")
        .trim_start_matches("0x")
        .parse()
        .map_err(|_| format!("{file}: bad {name}.code_hash"))?;
    let hash_type = match s["hash_type"].as_str().unwrap_or("") {
        "data" => ScriptHashType::Data,
        "data1" => ScriptHashType::Data1,
        "data2" => ScriptHashType::Data2,
        "type" => ScriptHashType::Type,
        _ => return Err(format!("{file}: bad {name}.hash_type")),
    };
    let code_cell = out_point_from(
        s["tx_hash"].as_str().unwrap_or(""),
        s["index"].as_u64().unwrap_or(0) as u32,
    )?;
    Ok(Deployed {
        code_hash,
        hash_type,
        code_cell,
    })
}

fn load_deployments(cfg: &Config) -> Result<Deployments, String> {
    let Some(dev) = cfg.devnet.as_ref().filter(|_| cfg.network == "devnet") else {
        // TODO: shipped script hashes of testnet/mainnet once the scripts are deployed there.
        return Err(format!(
            "the CSR scripts are not deployed on {} yet",
            cfg.network
        ));
    };
    let f = &dev.deployments;
    let text = std::fs::read_to_string(f).map_err(|e| format!("cannot read {f}: {e}"))?;
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("{f} is not JSON: {e}"))?;
    let hex32 = |x: &Value, what: &str| -> Result<Vec<u8>, String> {
        hex::decode(x.as_str().unwrap_or("").trim_start_matches("0x"))
            .map_err(|_| format!("{f}: bad {what}"))
    };
    Ok(Deployments {
        float: load_deployed(&v, "float_lock", f)?,
        dual: load_deployed(&v, "dual_lock", f)?,
        registry: load_deployed(&v, "registry", f)?,
        job_type: load_deployed(&v, "job_type", f)?,
        job_lock: load_deployed(&v, "job_lock", f)?,
        registry_type_hash: hex32(&v["registry_type_hash"], "registry_type_hash")?,
        config_type_args: hex32(&v["config_cell"]["type_args"], "config_cell.type_args")?,
        config_type_hash: H256::from_slice(&hex32(
            &v["config_cell"]["type_hash"],
            "config_cell.type_hash",
        )?)
        .map_err(|_| format!("{f}: bad config_cell.type_hash"))?,
    })
}

fn type_id_script(args: &[u8]) -> Script {
    let mut code = [0u8; 32];
    code[25..32].copy_from_slice(b"TYPE_ID");
    Script::new_builder()
        .code_hash(packed::Byte32::from_slice(&code).unwrap())
        .hash_type(packed::Byte::new(ScriptHashType::Type as u8))
        .args(Bytes::copy_from_slice(args).pack())
        .build()
}

/// An output of a prepared transaction.
struct Out {
    lock: Script,
    type_: Option<Script>,
    data: Bytes,
    cap: u64,
}

impl Out {
    fn plain(lock: Script, cap: u64) -> Out {
        Out {
            lock,
            type_: None,
            data: Bytes::new(),
            cap,
        }
    }
}

/// Molecule table: header (total size, field offsets) followed by the fields.
fn mol_table(fields: &[Vec<u8>]) -> Vec<u8> {
    let header = 4 + 4 * fields.len();
    let total = header + fields.iter().map(Vec::len).sum::<usize>();
    let mut out = (total as u32).to_le_bytes().to_vec();
    let mut off = header;
    for f in fields {
        out.extend_from_slice(&(off as u32).to_le_bytes());
        off += f.len();
    }
    for f in fields {
        out.extend_from_slice(f);
    }
    out
}

fn mol_bytes(b: &[u8]) -> Vec<u8> {
    let mut v = (b.len() as u32).to_le_bytes().to_vec();
    v.extend_from_slice(b);
    v
}

/// A monitor attestation as written by `csr-dev attest` (and the monitor).
pub struct Attestation {
    peer_id: Vec<u8>,
    pubkey: Vec<u8>,
    network: u8,
    expiry_ms: u64,
    signature: Vec<u8>,
}

fn network_id(name: &str) -> Option<u8> {
    match name {
        "mainnet" => Some(0),
        "testnet" => Some(1),
        "devnet" => Some(2),
        _ => None,
    }
}

impl Attestation {
    pub fn parse(v: &Value) -> Result<Attestation, String> {
        let hex_field = |k: &str, len: usize| -> Result<Vec<u8>, String> {
            let b = hex::decode(v[k].as_str().unwrap_or("").trim_start_matches("0x"))
                .map_err(|_| format!("attestation: {k} is not hex"))?;
            if b.len() != len {
                return Err(format!("attestation: {k} must be {len} bytes"));
            }
            Ok(b)
        };
        let peer_id = v["peer_id"]
            .as_str()
            .filter(|p| !p.is_empty())
            .ok_or("attestation: peer_id missing")?
            .as_bytes()
            .to_vec();
        let network = v["network"]
            .as_str()
            .and_then(network_id)
            .ok_or("attestation: network must be mainnet, testnet or devnet")?;
        Ok(Attestation {
            peer_id,
            pubkey: hex_field("service_pubkey", 33)?,
            network,
            expiry_ms: v["expiry_ms"]
                .as_u64()
                .ok_or("attestation: expiry_ms missing")?,
            signature: hex_field("signature", 65)?,
        })
    }

    fn molecule(&self) -> Vec<u8> {
        mol_table(&[
            mol_bytes(&self.peer_id),
            self.pubkey.clone(),
            vec![self.network],
            self.expiry_ms.to_le_bytes().to_vec(),
            self.signature.clone(),
        ])
    }

    /// Molecule `Registry` data (`contracts/schemas/csr.mol`), prekeys empty.
    fn registry_data(&self, float_lock_hash: &H256) -> Vec<u8> {
        mol_table(&[
            mol_bytes(&self.peer_id),
            self.pubkey.clone(),
            float_lock_hash.as_bytes().to_vec(),
            vec![self.network],
            self.molecule(),
            mol_bytes(&[]),
        ])
    }
}

/// `(peer_id, service_pubkey, float_lock_hash, expiry_ms)` of a molecule `Registry`.
fn parse_registry_data(d: &[u8]) -> Option<(String, Vec<u8>, Vec<u8>, u64)> {
    let u32_at = |o: usize| -> Option<usize> {
        Some(u32::from_le_bytes(d.get(o..o + 4)?.try_into().ok()?) as usize)
    };
    if u32_at(0)? != d.len() || u32_at(4)? != 28 {
        return None;
    }
    let field = |i: usize| -> Option<&[u8]> {
        let start = u32_at(4 + 4 * i)?;
        let end = if i == 5 { d.len() } else { u32_at(8 + 4 * i)? };
        d.get(start..end)
    };
    let peer = field(0)?;
    let peer = std::str::from_utf8(peer.get(4..)?).ok()?.to_string();
    let att = field(4)?;
    // Attestation table: 5 offsets; expiry is field 3.
    let a_off = |i: usize| -> Option<usize> {
        Some(u32::from_le_bytes(att.get(4 + 4 * i..8 + 4 * i)?.try_into().ok()?) as usize)
    };
    let expiry = att.get(a_off(3)?..a_off(3)? + 8)?;
    Some((
        peer,
        field(1)?.to_vec(),
        field(2)?.to_vec(),
        u64::from_le_bytes(expiry.try_into().ok()?),
    ))
}

/// Signs the sighash group made of input 0 only (the runner key cell) with the service key.
fn sign_first_input(
    tx: &TransactionView,
    key: &secp256k1::SecretKey,
) -> Result<TransactionView, String> {
    let mut witnesses: Vec<Bytes> = tx.witnesses().into_iter().map(|w| w.raw_data()).collect();
    let placeholder = WitnessArgs::new_builder()
        .lock(Some(Bytes::from(vec![0u8; 65])).pack())
        .build();
    let n_inputs = tx.inputs().len();
    let mut h = ckb_hash::new_blake2b();
    h.update(tx.hash().as_slice());
    h.update(&(placeholder.as_bytes().len() as u64).to_le_bytes());
    h.update(&placeholder.as_bytes());
    for w in witnesses.iter().skip(n_inputs) {
        h.update(&(w.len() as u64).to_le_bytes());
        h.update(w);
    }
    let mut digest = [0u8; 32];
    h.finalize(&mut digest);
    let (rec, compact) = secp256k1::Secp256k1::new()
        .sign_ecdsa_recoverable(&secp256k1::Message::from_digest(digest), key)
        .serialize_compact();
    let mut sig = compact.to_vec();
    sig.push(rec.to_i32() as u8);
    witnesses[0] = WitnessArgs::new_builder()
        .lock(Some(Bytes::from(sig)).pack())
        .build()
        .as_bytes();
    Ok(tx
        .as_advanced_builder()
        .set_witnesses(witnesses.into_iter().map(|w| w.pack()).collect())
        .build())
}

pub struct Chain {
    rpc: RpcClient,
    network: String,
    float: Deployed,
    dual: Deployed,
    registry: Deployed,
    job_type: Deployed,
    job_lock: Deployed,
    registry_type_hash: Vec<u8>,
    config_type_args: Vec<u8>,
    config_type_hash: H256,
    runner_lock: Script,
    service_key: Option<secp256k1::SecretKey>,
    service_pubkey: Vec<u8>,
}

pub struct TipInfo {
    pub number: u64,
    /// Packed epoch (`number | index << 24 | length << 40`).
    pub epoch: u64,
    pub median_ms: u64,
}

/// A live cell with job lock and job type, data not yet decoded.
pub struct RawJob {
    pub outpoint: String,
    pub service_code_hash: String,
    pub capacity: u64,
    /// Block that created the cell.
    pub block: u64,
    pub data: Vec<u8>,
}

pub struct Float {
    pub cell: Cell,
    owner_lock_hash: H256,
    runner_lock_hash: H256,
    max_fee: u64,
}

/// An unsigned transaction in `ckb-cli` tx file format.
pub struct Prepared {
    pub file: Value,
    pub fee_shannons: u64,
    pub tx_hash: String,
}

impl Chain {
    /// Connects to the configured node; the chain must be the network's chain.
    pub async fn connect(cfg: &Config, service_pubkey: &str) -> Result<Chain, String> {
        let dep = load_deployments(cfg)?;
        let rpc = RpcClient::new(&cfg.node.rpc_url);
        let info = rpc
            .call("get_blockchain_info", json!([]))
            .await
            .map_err(|e| format!("cannot reach the node: {e}"))?;
        let chain = info["chain"].as_str().unwrap_or("");
        if chain != expected_chain(&cfg.network) {
            return Err(format!(
                "node is on chain '{chain}', expected '{}'",
                expected_chain(&cfg.network)
            ));
        }
        let pk = hex::decode(service_pubkey.trim_start_matches("0x")).map_err(|e| e.to_string())?;
        Ok(Chain {
            rpc,
            network: cfg.network.clone(),
            float: dep.float,
            dual: dep.dual,
            registry: dep.registry,
            job_type: dep.job_type,
            job_lock: dep.job_lock,
            registry_type_hash: dep.registry_type_hash,
            config_type_args: dep.config_type_args,
            config_type_hash: dep.config_type_hash,
            runner_lock: sighash_lock(&blake2b_256(&pk)[..20]),
            service_key: None,
            service_pubkey: pk,
        })
    }

    /// The service key signs the runner key cell input of registration transactions.
    pub fn with_service_key(mut self, key: secp256k1::SecretKey) -> Chain {
        self.service_key = Some(key);
        self
    }

    pub fn runner_lock_hash(&self) -> H256 {
        script_hash(&self.runner_lock)
    }

    fn float_lock(&self, owner_hash: &H256, max_fee: u64) -> Script {
        let mut args = owner_hash.as_bytes().to_vec();
        args.extend_from_slice(self.runner_lock_hash().as_bytes());
        args.extend_from_slice(&max_fee.to_le_bytes());
        Script::new_builder()
            .code_hash(self.float.code_hash.pack())
            .hash_type(packed::Byte::new(self.float.hash_type as u8))
            .args(Bytes::from(args).pack())
            .build()
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        self.rpc
            .call(method, params)
            .await
            .map_err(|e| format!("{method}: {e}"))
    }

    async fn wait_indexer(&self) -> Result<(), String> {
        for _ in 0..100 {
            let tip = hex_u64(&self.call("get_tip_header", json!([])).await?["number"]);
            let it = self.call("get_indexer_tip", json!([])).await?;
            if let (Some(tip), Some(idx)) = (tip, hex_u64(&it["block_number"])) {
                if idx >= tip {
                    return Ok(());
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        Err("the node's indexer does not catch up with the tip".into())
    }

    /// All live cells matching the search key (at most 20 pages of 1024).
    async fn get_cells(&self, key: Value) -> Result<Vec<Cell>, String> {
        self.wait_indexer().await?;
        let mut cells = Vec::new();
        let mut cursor = Value::Null;
        for _ in 0..20 {
            let r = self
                .call("get_cells", json!([key, "asc", "0x400", cursor]))
                .await?;
            let page = r["objects"].as_array().ok_or("get_cells: no objects")?;
            for c in page {
                cells.push(parse_cell(c)?);
            }
            cursor = r["last_cursor"].clone();
            if page.len() < 0x400 || cursor.is_null() {
                break;
            }
        }
        Ok(cells)
    }

    /// Cells without type script and data under a lock.
    async fn plain_cells(&self, lock: &Script) -> Result<Vec<Cell>, String> {
        self.get_cells(json!({
            "script": json::Script::from(lock.clone()),
            "script_type": "lock",
            "filter": {"script_len_range": ["0x0", "0x1"], "output_data_len_range": ["0x0", "0x1"]},
        }))
        .await
    }

    pub async fn runner_cells(&self) -> Result<Vec<Cell>, String> {
        self.plain_cells(&self.runner_lock).await
    }

    pub async fn runner_cell_status(&self) -> Result<Value, String> {
        let cells = self.runner_cells().await?;
        let total: u64 = cells.iter().map(|c| c.capacity).sum();
        Ok(json!({
            "found": !cells.is_empty(),
            "outpoint": cells.first().map(|c| out_point_str(&c.out_point)),
            "capacity_shannons": total,
        }))
    }

    /// The float cell of this runner (found by the runner lock hash inside the lock args).
    pub async fn float(&self) -> Result<Option<Float>, String> {
        let runner_hash = self.runner_lock_hash();
        let key = json!({
            "script": {
                "code_hash": hx(self.float.code_hash.as_bytes()),
                "hash_type": json::ScriptHashType::from(self.float.hash_type),
                "args": "0x",
            },
            "script_type": "lock",
            "script_search_mode": "prefix",
            "filter": {"output_data_len_range": ["0x0", "0x1"]},
        });
        // The runner lock hash sits at offset 32 of the args, so it cannot be a prefix: scan all
        // cells of the float lock and filter here (the default indexer has no partial search).
        for cell in self.get_cells(key).await? {
            let args = cell.lock.args().raw_data();
            if args.len() == 72 && args[32..64] == *runner_hash.as_bytes() {
                return Ok(Some(Float {
                    owner_lock_hash: H256::from_slice(&args[..32]).map_err(|e| e.to_string())?,
                    runner_lock_hash: runner_hash,
                    max_fee: u64::from_le_bytes(args[64..72].try_into().unwrap()),
                    cell,
                }));
            }
        }
        Ok(None)
    }

    pub async fn float_status(&self) -> Result<Value, String> {
        Ok(match self.float().await? {
            Some(f) => json!({
                "found": true,
                "outpoint": out_point_str(&f.cell.out_point),
                "capacity_shannons": f.cell.capacity,
                "max_fee_shannons": f.max_fee,
                "owner_lock_hash": hx(f.owner_lock_hash.as_bytes()),
                "runner_lock_hash": hx(f.runner_lock_hash.as_bytes()),
            }),
            None => json!({
                "found": false,
                "outpoint": null,
                "capacity_shannons": null,
                "max_fee_shannons": null,
                "owner_lock_hash": null,
                "runner_lock_hash": hx(self.runner_lock_hash().as_bytes()),
            }),
        })
    }

    /// Cell dep of the secp256k1_blake160 dep group (found in the genesis block).
    async fn secp_dep(&self) -> Result<CellDep, String> {
        let block = self.call("get_block_by_number", json!(["0x0"])).await?;
        let block: json::BlockView = serde_json::from_value(block).map_err(|e| e.to_string())?;
        for tx in &block.transactions {
            for (i, data) in tx.inner.outputs_data.iter().enumerate() {
                let Ok(vec) = packed::OutPointVec::from_slice(data.as_bytes()) else {
                    continue;
                };
                for member in vec.into_iter() {
                    let r = self
                        .call(
                            "get_live_cell",
                            json!([json::OutPoint::from(member), false]),
                        )
                        .await?;
                    let ty = r["cell"]["output"]["type"].clone();
                    let Ok(ty) = serde_json::from_value::<json::Script>(ty) else {
                        continue;
                    };
                    if script_hash(&Script::from(ty)) == SIGHASH_CODE_HASH {
                        let op = OutPoint::new_builder()
                            .tx_hash(tx.hash.pack())
                            .index(Pack::<packed::Uint32>::pack(&(i as u32)))
                            .build();
                        return Ok(CellDep::new_builder()
                            .out_point(op)
                            .dep_type(packed::Byte::new(DepType::DepGroup as u8))
                            .build());
                    }
                }
            }
        }
        Err("secp256k1_blake160 dep group not found in the genesis block".into())
    }

    fn float_dep(&self) -> CellDep {
        CellDep::new_builder()
            .out_point(self.float.code_cell.clone())
            .dep_type(packed::Byte::new(DepType::Code as u8))
            .build()
    }

    /// Finds the owner's lock script from the lock hash in the float args: from `--owner-lock-args`
    /// or from the inputs of the transaction that created the float cell.
    async fn owner_lock(&self, f: &Float, args: Option<&str>) -> Result<Script, String> {
        let want = &f.owner_lock_hash;
        let found = match args {
            Some(a) => {
                let l = sighash_lock(&parse_lock_args(a)?);
                if script_hash(&l) != *want {
                    return Err("owner lock args do not match the float's owner lock hash".into());
                }
                Some(l)
            }
            None => self.discover_owner_lock(f).await?,
        };
        let lock = found.ok_or(
            "cannot determine the owner's lock from the chain; pass --owner-lock-args".to_string(),
        )?;
        if !is_sighash(&lock) {
            return Err("only owners with a secp256k1_blake160 lock are supported".into());
        }
        Ok(lock)
    }

    async fn discover_owner_lock(&self, f: &Float) -> Result<Option<Script>, String> {
        let tx_of = |r: &Value| -> Value {
            let t = &r["transaction"];
            t.get("inner").unwrap_or(t).clone()
        };
        let r = self
            .call(
                "get_transaction",
                json!([hx(f.cell.out_point.tx_hash().as_slice())]),
            )
            .await?;
        let tx = tx_of(&r);
        for input in tx["inputs"].as_array().into_iter().flatten() {
            let prev = &input["previous_output"];
            let Some(h) = prev["tx_hash"].as_str() else {
                continue;
            };
            let idx = hex_u64(&prev["index"]).unwrap_or(0) as usize;
            let ptx = tx_of(&self.call("get_transaction", json!([h])).await?);
            let Ok(lock) =
                serde_json::from_value::<json::Script>(ptx["outputs"][idx]["lock"].clone())
            else {
                continue;
            };
            let lock = Script::from(lock);
            if script_hash(&lock) == f.owner_lock_hash {
                return Ok(Some(lock));
            }
        }
        Ok(None)
    }

    /// Builds the unsigned transaction: `pre` cells are spent first, then as many of the owner's
    /// plain cells as needed. Outputs are `fixed` plus the change to the owner, who pays the fee.
    async fn assemble(
        &self,
        owner: &Script,
        pre: Vec<Cell>,
        fixed: Vec<Out>,
        mut deps: Vec<CellDep>,
        sign_runner_input: bool,
    ) -> Result<Prepared, String> {
        deps.insert(0, self.secp_dep().await?);
        let change_min = occupied(owner, None, 0);
        let in_pre: u64 = pre.iter().map(|c| c.capacity).sum();
        let out_fixed: u64 = fixed.iter().map(|o| o.cap).sum();
        let need = out_fixed + FEE_RESERVE + change_min;
        let mut picked: Vec<Cell> = Vec::new();
        let mut have = in_pre;
        for c in self.plain_cells(owner).await? {
            if !picked.is_empty() && have >= need {
                break;
            }
            have += c.capacity;
            picked.push(c);
        }
        if picked.is_empty() || have < need {
            return Err(format!(
                "the owner's cells are insufficient: have {have} shannons, need {need}"
            ));
        }
        let owner_first = pre.len();
        let inputs: Vec<CellInput> = pre
            .iter()
            .chain(&picked)
            .map(|c| CellInput::new(c.out_point.clone(), 0))
            .collect();
        let mut witnesses = vec![Bytes::new(); inputs.len()];
        if sign_runner_input {
            witnesses[0] = WitnessArgs::new_builder()
                .lock(Some(Bytes::from(vec![0u8; 65])).pack())
                .build()
                .as_bytes();
        }
        witnesses[owner_first] = WitnessArgs::new_builder()
            .lock(Some(Bytes::from(vec![0u8; 65])).pack())
            .build()
            .as_bytes();
        let build = |change: u64| -> TransactionView {
            let mut b = TransactionBuilder::default()
                .cell_deps(deps.clone())
                .inputs(inputs.clone())
                .witnesses(witnesses.iter().map(|w| w.pack()).collect::<Vec<_>>());
            let change_out = Out::plain(owner.clone(), change);
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
        // 2 shannons per byte (the node's minimum is 1) plus a constant.
        let fee = build(have - out_fixed).data().as_slice().len() as u64 * 2 + 1000;
        let change = have - out_fixed - fee;
        if change < change_min {
            return Err("the owner's cells cannot cover the change output and the fee".into());
        }
        let mut tx = build(change);
        if sign_runner_input {
            let key = self
                .service_key
                .as_ref()
                .ok_or("the service key is not available")?;
            tx = sign_first_input(&tx, key)?;
        }
        Ok(Prepared {
            file: json!({
                "transaction": serde_json::to_value(json::TransactionView::from(tx.clone()).inner)
                    .map_err(|e| e.to_string())?,
                "multisig_configs": {},
                "signatures": {},
            }),
            fee_shannons: fee,
            tx_hash: hx(tx.hash().as_slice()),
        })
    }

    pub async fn prepare_create(
        &self,
        ckb: &str,
        owner_lock_args: &str,
        max_fee: u64,
    ) -> Result<Prepared, String> {
        let owner = sighash_lock(&parse_lock_args(owner_lock_args)?);
        let cap = ckb_to_shannons(ckb)?;
        let lock = self.float_lock(&script_hash(&owner), max_fee);
        if cap < occupied(&lock, None, 0) {
            return Err(format!(
                "a float cell needs at least {} CKB",
                occupied(&lock, None, 0) / CKB
            ));
        }
        if self.float().await?.is_some() {
            return Err("a float cell already exists; use prepare-topup".into());
        }
        self.assemble(&owner, vec![], vec![Out::plain(lock, cap)], vec![], false)
            .await
    }

    async fn existing_float(&self) -> Result<Float, String> {
        self.float()
            .await?
            .ok_or_else(|| "no float cell found; use prepare-create".to_string())
    }

    pub async fn prepare_topup(
        &self,
        ckb: &str,
        owner_lock_args: Option<&str>,
    ) -> Result<Prepared, String> {
        let add = ckb_to_shannons(ckb)?;
        let f = self.existing_float().await?;
        let owner = self.owner_lock(&f, owner_lock_args).await?;
        let cap = f
            .cell
            .capacity
            .checked_add(add)
            .ok_or("capacity overflow")?;
        let lock = f.cell.lock.clone();
        self.assemble(
            &owner,
            vec![f.cell],
            vec![Out::plain(lock, cap)],
            vec![self.float_dep()],
            false,
        )
        .await
    }

    /// Withdraws `ckb` to the owner; `all` closes the float cell instead.
    pub async fn prepare_withdraw(
        &self,
        ckb: Option<&str>,
        all: bool,
        owner_lock_args: Option<&str>,
    ) -> Result<Prepared, String> {
        let f = self.existing_float().await?;
        let owner = self.owner_lock(&f, owner_lock_args).await?;
        let fixed = if all {
            vec![]
        } else {
            let amount = ckb_to_shannons(ckb.ok_or("give --ckb N or --all")?)?;
            let keep = occupied(&f.cell.lock, None, 0);
            let left = f.cell.capacity.checked_sub(amount).filter(|l| *l >= keep);
            let left = left.ok_or_else(|| {
                format!(
                    "the float keeps at least {} CKB (its occupied capacity); use --all to close it",
                    keep / CKB
                )
            })?;
            vec![Out::plain(f.cell.lock.clone(), left)]
        };
        self.assemble(&owner, vec![f.cell], fixed, vec![self.float_dep()], false)
            .await
    }

    /// Tip block number, packed epoch and median time (ms) for trigger checks.
    pub async fn tip_info(&self) -> Result<TipInfo, String> {
        let h = self.call("get_tip_header", json!([])).await?;
        let number = hex_u64(&h["number"]).ok_or("bad tip number")?;
        let epoch = hex_u64(&h["epoch"]).ok_or("bad tip epoch")?;
        let hash = h["hash"].as_str().unwrap_or("").to_string();
        let median_ms = hex_u64(&self.call("get_block_median_time", json!([hash])).await?)
            .ok_or("bad median time")?;
        Ok(TipInfo {
            number,
            epoch,
            median_ms,
        })
    }

    /// Live cells with this deployment's job lock, restricted to the job type.
    pub async fn job_cells(&self) -> Result<Vec<RawJob>, String> {
        self.wait_indexer().await?;
        let lock = Script::new_builder()
            .code_hash(self.job_lock.code_hash.pack())
            .hash_type(packed::Byte::new(self.job_lock.hash_type as u8))
            .args(Bytes::copy_from_slice(&self.registry_type_hash).pack())
            .build();
        let key = json!({
            "script": json::Script::from(lock),
            "script_type": "lock",
            "with_data": true,
        });
        let mut out = Vec::new();
        let mut cursor = Value::Null;
        for _ in 0..20 {
            let r = self
                .call("get_cells", json!([key, "asc", "0x400", cursor]))
                .await?;
            let page = r["objects"].as_array().ok_or("get_cells: no objects")?;
            for c in page {
                let Some(t) = c["output"]["type"].as_object() else {
                    continue;
                };
                let ty: json::Script =
                    serde_json::from_value(Value::Object(t.clone())).map_err(|e| e.to_string())?;
                let ty: Script = ty.into();
                let args = ty.args().raw_data();
                if ty.code_hash() != self.job_type.code_hash.pack()
                    || ty.hash_type() != packed::Byte::new(self.job_type.hash_type as u8)
                    || args.len() != 32
                {
                    continue;
                }
                let cell = parse_cell(c)?;
                out.push(RawJob {
                    outpoint: out_point_str(&cell.out_point),
                    service_code_hash: hx(&args),
                    capacity: cell.capacity,
                    block: hex_u64(&c["block_number"]).unwrap_or(0),
                    data: cell.data,
                });
            }
            cursor = r["last_cursor"].clone();
            if page.len() < 0x400 || cursor.is_null() {
                break;
            }
        }
        Ok(out)
    }

    fn registry_type(&self) -> Script {
        Script::new_builder()
            .code_hash(self.registry.code_hash.pack())
            .hash_type(packed::Byte::new(self.registry.hash_type as u8))
            .args(Bytes::copy_from_slice(self.config_type_hash.as_bytes()).pack())
            .build()
    }

    /// The monitor config cell (found by its Type ID type script) and its `clock_ms`.
    async fn config_cell(&self) -> Result<(Cell, u64), String> {
        let cells = self
            .get_cells(json!({
                "script": json::Script::from(type_id_script(&self.config_type_args)),
                "script_type": "type",
            }))
            .await?;
        let cell = cells
            .into_iter()
            .next()
            .ok_or("the monitor config cell was not found on chain")?;
        if cell.data.len() < 41 {
            return Err("the monitor config cell is malformed".into());
        }
        let clock = u64::from_le_bytes(cell.data[33..41].try_into().unwrap());
        Ok((cell, clock))
    }

    /// Registry cells of this service key; the one with the latest attestation wins (audit F8).
    async fn registry_cell(&self) -> Result<Option<(Cell, String, Vec<u8>, u64)>, String> {
        let cells = self
            .get_cells(json!({
                "script": json::Script::from(self.registry_type()),
                "script_type": "type",
            }))
            .await?;
        let mine = &self.service_pubkey;
        Ok(cells
            .into_iter()
            .filter_map(|c| {
                let (peer, pk, float, exp) = parse_registry_data(&c.data)?;
                (pk == *mine).then_some((c, peer, float, exp))
            })
            .max_by_key(|r| r.3))
    }

    pub async fn register_status(&self) -> Result<Value, String> {
        Ok(match self.registry_cell().await? {
            Some((c, peer, float, exp)) => json!({
                "registered": true,
                "outpoint": out_point_str(&c.out_point),
                "peer_id": peer,
                "attestation_expiry_ms": exp,
                "float_lock_hash": hx(float),
            }),
            None => json!({
                "registered": false,
                "outpoint": null,
                "peer_id": null,
                "attestation_expiry_ms": null,
                "float_lock_hash": null,
            }),
        })
    }

    /// Unsigned registry creation: the owner pays; the runner key cell is spent and re-created and
    /// its input is signed here with the service key (`spec/registry.md`, rule 6).
    pub async fn prepare_register(
        &self,
        attestation: &Value,
        owner_lock_args: &str,
    ) -> Result<Prepared, String> {
        let polled = crate::node::poll(&self.rpc, &self.network).await;
        if polled.report.state != crate::node::r2::R2State::Synced {
            return Err(format!(
                "the node is not synced (R2: {}); registration needs a synced node",
                polled.report.state.as_str()
            ));
        }
        let att = Attestation::parse(attestation)?;
        if att.pubkey != self.service_pubkey {
            return Err("the attestation is for a different service key".into());
        }
        if Some(att.network) != network_id(&self.network) {
            return Err(format!(
                "the attestation is for another network, this node is on {}",
                self.network
            ));
        }
        let (config, clock) = self.config_cell().await?;
        if att.expiry_ms <= clock {
            return Err("the attestation has expired".into());
        }
        let float = self
            .float()
            .await?
            .ok_or("no float cell found; create the float before registering")?;
        let runner_cell = self
            .runner_cells()
            .await?
            .into_iter()
            .next()
            .ok_or("no runner key cell found; fund the runner lock first")?;
        let owner = sighash_lock(&parse_lock_args(owner_lock_args)?);
        let mut args = script_hash(&owner).as_bytes().to_vec();
        args.extend_from_slice(self.runner_lock_hash().as_bytes());
        let lock = Script::new_builder()
            .code_hash(self.dual.code_hash.pack())
            .hash_type(packed::Byte::new(self.dual.hash_type as u8))
            .args(Bytes::from(args).pack())
            .build();
        let type_ = self.registry_type();
        let data = Bytes::from(att.registry_data(&script_hash(&float.cell.lock)));
        let cap = occupied(&lock, Some(&type_), data.len());
        let registry = Out {
            lock,
            type_: Some(type_),
            data,
            cap,
        };
        let key_cell = Out::plain(runner_cell.lock.clone(), runner_cell.capacity);
        let deps = vec![
            CellDep::new_builder()
                .out_point(self.registry.code_cell.clone())
                .dep_type(packed::Byte::new(DepType::Code as u8))
                .build(),
            CellDep::new_builder()
                .out_point(config.out_point)
                .dep_type(packed::Byte::new(DepType::Code as u8))
                .build(),
        ];
        self.assemble(
            &owner,
            vec![runner_cell],
            vec![registry, key_cell],
            deps,
            true,
        )
        .await
    }

    /// Broadcasts a signed transaction file (ckb-cli tx file or a bare transaction).
    pub async fn submit(&self, file: &Value) -> Result<Value, String> {
        let raw = file.get("transaction").unwrap_or(file).clone();
        let tx: json::Transaction =
            serde_json::from_value(raw).map_err(|e| format!("not a transaction: {e}"))?;
        match self
            .rpc
            .call("send_transaction", json!([tx, "passthrough"]))
            .await
        {
            Ok(h) => Ok(json!({"tx_hash": h})),
            Err(RpcError::Rpc { message, .. }) => {
                Err(format!("the node rejected the transaction: {message}"))
            }
            Err(e) => Err(format!("cannot submit: {e}")),
        }
    }
}

pub fn read_json(p: &Path) -> Result<Value, String> {
    let t = std::fs::read_to_string(p).map_err(|e| format!("cannot read {}: {e}", p.display()))?;
    serde_json::from_str(&t).map_err(|e| format!("{} is not JSON: {e}", p.display()))
}

/// Writes a prepared transaction file; returns the CLI/API summary.
pub fn write_prepared(p: &Prepared, out: &Path) -> Result<Value, String> {
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let text = serde_json::to_string_pretty(&p.file).map_err(|e| e.to_string())?;
    std::fs::write(out, text + "\n").map_err(|e| format!("cannot write {}: {e}", out.display()))?;
    Ok(json!({"file": out, "fee_shannons": p.fee_shannons, "tx_hash": p.tx_hash}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts() {
        assert_eq!(ckb_to_shannons("1").unwrap(), CKB);
        assert_eq!(ckb_to_shannons("0.5").unwrap(), CKB / 2);
        assert!(ckb_to_shannons("1.123456789").is_err());
        assert!(ckb_to_shannons("x").is_err());
        assert!(ckb_to_shannons("").is_err());
    }

    #[test]
    fn lock_args_must_be_20_bytes() {
        assert!(parse_lock_args("0x00").is_err());
        assert!(parse_lock_args(&format!("0x{}", "ab".repeat(20))).is_ok());
    }

    #[test]
    fn registry_data_roundtrip() {
        let att = Attestation::parse(&json!({
            "peer_id": "QmPeer", "service_pubkey": format!("0x02{}", "11".repeat(32)),
            "network": "devnet", "expiry_ms": 12345, "signature": format!("0x{}", "22".repeat(65)),
        }))
        .unwrap();
        let data = att.registry_data(&H256::from_slice(&[7u8; 32]).unwrap());
        let (peer, pk, float, exp) = parse_registry_data(&data).unwrap();
        assert_eq!(peer, "QmPeer");
        assert_eq!(pk.len(), 33);
        assert_eq!(float, vec![7u8; 32]);
        assert_eq!(exp, 12345);
    }

    #[test]
    fn attestation_needs_known_network() {
        let v = json!({"peer_id": "Qm", "service_pubkey": format!("0x02{}", "11".repeat(32)),
            "network": "moon", "expiry_ms": 1, "signature": format!("0x{}", "22".repeat(65))});
        assert!(Attestation::parse(&v).is_err());
    }

    #[test]
    fn float_cell_occupies_113_ckb() {
        let l = sighash_lock(&[0; 72]);
        assert_eq!(occupied(&l, None, 0), 113 * CKB);
    }
}
