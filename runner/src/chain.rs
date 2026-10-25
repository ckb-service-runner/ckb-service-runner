//! On-chain flows: the runner wallet, the registration, job runs and transaction submission.
//!
//! The runner wallet is the set of plain cells of the runner's own secp256k1 lock (`blake160` of the
//! service key). CSR builds and signs every transaction itself with the service key (decision D-A8-1); the
//! operator only funds the wallet and signs the stake consent text in its own wallet.

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
/// Fee reserve when picking runner wallet cells; the exact fee comes out of the change.
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

/// Minimum registration stake (`spec/registry.md`, stake rule 3): 10,000 CKB.
pub const MIN_STAKE: u64 = 1_000_000_000_000;

/// Type script code hash of the Nervos DAO (hash type `type`, empty args; the same on every chain).
const DAO_TYPE_CODE_HASH: H256 =
    h256!("0x82d76d1b75fe2fd9a27dfbaa65a039221a380d76c926f378d3f81cf3e7e13f2e");

/// A stake out point `0x<64 hex>:<index decimal>` (hex in any case) → the lower-case form.
pub fn parse_stake(s: &str) -> Result<String, String> {
    let bad = || format!("bad stake out point '{s}': expected 0x<64 hex tx hash>:<index>");
    let (h, i) = s.trim().split_once(':').ok_or_else(bad)?;
    let h = h.strip_prefix("0x").ok_or_else(bad)?;
    if h.len() != 64 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(bad());
    }
    if i.is_empty() || !i.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad());
    }
    let i: u32 = i.parse().map_err(|_| bad())?;
    Ok(format!("0x{}:{i}", h.to_ascii_lowercase()))
}

/// Why a cell is not a valid stake (stake rules 2–3: a Nervos DAO deposit of at least `MIN_STAKE`), or
/// `None` if it is. Who owns it is checked by the consent (rule 4).
fn stake_problem(out: &CellOutput, data: &[u8]) -> Option<String> {
    let dao = out.type_().to_opt().is_some_and(|t| {
        t.code_hash().as_slice() == DAO_TYPE_CODE_HASH.as_bytes()
            && t.hash_type().as_slice() == [ScriptHashType::Type as u8]
            && t.args().raw_data().is_empty()
    });
    if !dao || data != [0u8; 8] {
        return Some("the stake is not a Nervos DAO deposit".into());
    }
    let cap: u64 = out.capacity().unpack();
    if cap < MIN_STAKE {
        return Some(format!(
            "the stake holds {} CKB, below the minimum of {} CKB",
            cap as f64 / CKB as f64,
            MIN_STAKE / CKB
        ));
    }
    None
}

/// The 20 lock args of a `secp256k1_blake160_sighash_all` lock in the `type` form (what a wallet's
/// "Sign message" key controls); `None` for any other lock.
fn sighash_args(lock: &Script) -> Option<[u8; 20]> {
    (lock.code_hash().as_slice() == SIGHASH_CODE_HASH.as_bytes()
        && lock.hash_type().as_slice() == [ScriptHashType::Type as u8])
    .then(|| lock.args().raw_data().as_ref().try_into().ok())
    .flatten()
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

/// A deployed script: how to reference it and where its code cell (or dep group) is.
struct Deployed {
    code_hash: H256,
    hash_type: ScriptHashType,
    code_cell: OutPoint,
    dep_type: DepType,
}

impl Deployed {
    fn cell_dep(&self) -> CellDep {
        CellDep::new_builder()
            .out_point(self.code_cell.clone())
            .dep_type(packed::Byte::new(self.dep_type as u8))
            .build()
    }

    fn script(&self, args: &[u8]) -> Script {
        Script::new_builder()
            .code_hash(self.code_hash.pack())
            .hash_type(packed::Byte::new(self.hash_type as u8))
            .args(Bytes::copy_from_slice(args).pack())
            .build()
    }
}

struct Deployments {
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
    // `dep_type` is optional and means `code` when absent.
    let dep_type = match s["dep_type"].as_str() {
        None | Some("code") => DepType::Code,
        Some("dep_group") => DepType::DepGroup,
        Some(_) => return Err(format!("{file}: bad {name}.dep_type")),
    };
    Ok(Deployed {
        code_hash,
        hash_type,
        code_cell,
        dep_type,
    })
}

/// The CSR testnet deployment shipped with the runner.
const TESTNET_DEPLOYMENTS: &str = include_str!("../../deployments/testnet.json");

fn load_deployments(cfg: &Config) -> Result<Deployments, String> {
    match cfg.network.as_str() {
        "testnet" => parse_deployments(TESTNET_DEPLOYMENTS, "deployments/testnet.json"),
        "devnet" => {
            let dev = cfg
                .devnet
                .as_ref()
                .ok_or_else(|| "the CSR scripts are not deployed on devnet yet".to_string())?;
            let f = &dev.deployments;
            let text = std::fs::read_to_string(f).map_err(|e| format!("cannot read {f}: {e}"))?;
            parse_deployments(&text, f)
        }
        other => Err(format!("the CSR scripts are not deployed on {other} yet")),
    }
}

fn parse_deployments(text: &str, source: &str) -> Result<Deployments, String> {
    let f = source;
    let v: Value = serde_json::from_str(text).map_err(|e| format!("{f} is not JSON: {e}"))?;
    let hex32 = |x: &Value, what: &str| -> Result<Vec<u8>, String> {
        hex::decode(x.as_str().unwrap_or("").trim_start_matches("0x"))
            .map_err(|_| format!("{f}: bad {what}"))
    };
    Ok(Deployments {
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

    /// Molecule `Registry` v2 data (`contracts/schemas/csr.mol`), prekeys empty.
    fn registry_data(&self, payout_lock_hash: &H256, stake: &OutPoint) -> Vec<u8> {
        mol_table(&[
            mol_bytes(&self.peer_id),
            self.pubkey.clone(),
            payout_lock_hash.as_bytes().to_vec(),
            vec![self.network],
            self.molecule(),
            mol_bytes(&[]),
            stake.as_slice().to_vec(),
        ])
    }
}

/// Decoded fields of a molecule `Registry`.
struct RegistryData {
    peer_id: String,
    service_pubkey: Vec<u8>,
    payout_lock_hash: Vec<u8>,
    expiry_ms: u64,
    /// `stake` of registry v2; `None` for a 6-field (v1) cell.
    stake: Option<OutPoint>,
}

/// Parses a molecule `Registry` (6 fields, or 7 with the `stake` out point of registry v2).
fn parse_registry_data(d: &[u8]) -> Option<RegistryData> {
    let u32_at = |o: usize| -> Option<usize> {
        Some(u32::from_le_bytes(d.get(o..o + 4)?.try_into().ok()?) as usize)
    };
    let header = u32_at(4)?;
    if u32_at(0)? != d.len() || (header != 28 && header != 32) {
        return None;
    }
    let last = header / 4 - 2;
    let field = |i: usize| -> Option<&[u8]> {
        let start = u32_at(4 + 4 * i)?;
        let end = if i == last {
            d.len()
        } else {
            u32_at(8 + 4 * i)?
        };
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
    let stake = if last == 6 {
        Some(OutPoint::from_slice(field(6)?).ok()?)
    } else {
        None
    };
    Some(RegistryData {
        peer_id: peer,
        service_pubkey: field(1)?.to_vec(),
        payout_lock_hash: field(2)?.to_vec(),
        expiry_ms: u64::from_le_bytes(expiry.try_into().ok()?),
        stake,
    })
}

/// Signs the sighash group made of inputs `0..group` (runner wallet cells) with the service key. Witness 0
/// keeps its other fields (the consent in `output_type` is covered by the signature).
fn sign_wallet_inputs(
    tx: &TransactionView,
    group: usize,
    key: &secp256k1::SecretKey,
) -> Result<TransactionView, String> {
    let mut witnesses: Vec<Bytes> = tx.witnesses().into_iter().map(|w| w.raw_data()).collect();
    let n_inputs = tx.inputs().len();
    while witnesses.len() < n_inputs {
        witnesses.push(Bytes::new());
    }
    let args = if witnesses[0].is_empty() {
        WitnessArgs::default()
    } else {
        WitnessArgs::from_slice(&witnesses[0]).map_err(|e| format!("bad witness 0: {e}"))?
    };
    let placeholder = args
        .clone()
        .as_builder()
        .lock(Some(Bytes::from(vec![0u8; 65])).pack())
        .build();
    let mut h = ckb_hash::new_blake2b();
    h.update(tx.hash().as_slice());
    h.update(&(placeholder.as_bytes().len() as u64).to_le_bytes());
    h.update(&placeholder.as_bytes());
    for w in witnesses
        .iter()
        .take(group)
        .skip(1)
        .chain(witnesses.iter().skip(n_inputs))
    {
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
    witnesses[0] = args
        .as_builder()
        .lock(Some(Bytes::from(sig)).pack())
        .build()
        .as_bytes();
    Ok(tx
        .as_advanced_builder()
        .set_witnesses(witnesses.into_iter().map(|w| w.pack()).collect())
        .build())
}

fn code_dep(op: OutPoint) -> CellDep {
    CellDep::new_builder()
        .out_point(op)
        .dep_type(packed::Byte::new(DepType::Code as u8))
        .build()
}

pub struct Chain {
    rpc: RpcClient,
    network: String,
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

/// The job cells read from the chain; `complete` is false when the list was cut off.
pub struct JobCells {
    pub jobs: Vec<RawJob>,
    pub complete: bool,
}

static JOB_LIST_WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A live cell with job lock and job type, data not yet decoded.
pub struct RawJob {
    pub outpoint: String,
    pub service_code_hash: String,
    pub capacity: u64,
    /// Block that created the cell.
    pub block: u64,
    pub data: Vec<u8>,
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

    /// The service key signs the runner wallet inputs of every transaction CSR builds.
    pub fn with_service_key(mut self, key: secp256k1::SecretKey) -> Chain {
        self.service_key = Some(key);
        self
    }

    pub fn runner_lock_hash(&self) -> H256 {
        script_hash(&self.runner_lock)
    }

    /// The runner address: full-format address of the runner wallet lock.
    pub fn runner_address(&self) -> String {
        crate::address::sighash_address(&self.network, &self.runner_lock.args().raw_data())
    }

    pub fn network(&self) -> &str {
        &self.network
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
                if indexer_ready(tip, idx) {
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
                return Ok(cells);
            }
        }
        Err("get_cells: more than 20 pages of cells, the list would be incomplete".into())
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
            "runner_address": self.runner_address(),
        }))
    }

    /// Capacity of the runner wallet: the sum of all plain runner-lock cells.
    pub async fn wallet_capacity(&self) -> Result<u64, String> {
        Ok(self.runner_cells().await?.iter().map(|c| c.capacity).sum())
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

    /// The refusal when the runner wallet cannot pay a transaction.
    fn wallet_short(&self, have: u64, need: u64) -> String {
        format!(
            "the runner wallet has {} CKB, but this transaction needs {} CKB (including the {} CKB \
             that stay in the wallet and the fee); send at least {} CKB to the runner address {}",
            have as f64 / CKB as f64,
            need as f64 / CKB as f64,
            occupied(&self.runner_lock, None, 0) / CKB,
            need.saturating_sub(have).div_ceil(CKB).max(1),
            self.runner_address()
        )
    }

    /// Builds and signs a runner wallet transaction: inputs `[runner wallet cells…, extra…]`, outputs
    /// `[fixed…, wallet change]`. The wallet pays what the outputs need beyond the extra inputs, plus the fee,
    /// and keeps at least 61 CKB. `stake_dep` becomes `cell_deps[0]`; `consent` goes into
    /// `WitnessArgs.output_type` of witness 0 (output 0 is then the registry cell).
    async fn wallet_tx(
        &self,
        extra: Vec<Cell>,
        fixed: Vec<Out>,
        mut deps: Vec<CellDep>,
        stake_dep: Option<CellDep>,
        consent: Option<Vec<u8>>,
    ) -> Result<TransactionView, String> {
        let key = self
            .service_key
            .as_ref()
            .ok_or("the service key is not available")?;
        deps.insert(0, self.secp_dep().await?);
        if let Some(stake) = stake_dep {
            // First, so only code deps precede it (`spec/registry.md`, stake rule 1).
            deps.insert(0, stake);
        }
        let owner = &self.runner_lock;
        let change_min = occupied(owner, None, 0);
        let in_extra: u64 = extra.iter().map(|c| c.capacity).sum();
        let out_fixed: u64 = fixed.iter().map(|o| o.cap).sum();
        let need = (out_fixed + FEE_RESERVE + change_min).saturating_sub(in_extra);
        let cells = self.runner_cells().await?;
        let total: u64 = cells.iter().map(|c| c.capacity).sum();
        let mut picked: Vec<Cell> = Vec::new();
        let mut have = 0u64;
        for c in cells {
            if !picked.is_empty() && have >= need {
                break;
            }
            have += c.capacity;
            picked.push(c);
        }
        if picked.is_empty() || have < need {
            return Err(self.wallet_short(total, need.max(change_min)));
        }
        let group = picked.len();
        let inputs: Vec<CellInput> = picked
            .iter()
            .chain(&extra)
            .map(|c| CellInput::new(c.out_point.clone(), 0))
            .collect();
        let mut witnesses = vec![Bytes::new(); inputs.len()];
        witnesses[0] = WitnessArgs::new_builder()
            .lock(Some(Bytes::from(vec![0u8; 65])).pack())
            .output_type(consent.map(Bytes::from).pack())
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
        let gross = have + in_extra - out_fixed;
        // 2 shannons per byte (the node's minimum is 1) plus a constant.
        let fee = build(gross).data().as_slice().len() as u64 * 2 + 1000;
        let change = gross.saturating_sub(fee);
        if change < change_min {
            return Err(self.wallet_short(total, need.max(change_min) + fee));
        }
        sign_wallet_inputs(&build(change), group, key)
    }

    /// Broadcasts a transaction CSR built; the error carries the node's message.
    async fn broadcast(&self, tx: &TransactionView) -> Result<String, String> {
        self.send(tx)
            .await
            .map_err(|m| format!("the node rejected the transaction: {m}"))
    }

    /// Sends exactly `amount` shannons from the runner wallet to `to` (the fee comes from the wallet).
    pub async fn wallet_send(&self, to: Script, amount: u64) -> Result<String, String> {
        let min = occupied(&to, None, 0).max(61 * CKB);
        if amount < min {
            return Err(format!(
                "the amount must be at least {} CKB (a cell needs it)",
                min / CKB
            ));
        }
        let tx = self
            .wallet_tx(vec![], vec![Out::plain(to, amount)], vec![], None, None)
            .await?;
        self.broadcast(&tx).await
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
    /// Reads up to 200 pages; `complete` is false when the cursor had not ended by then.
    pub async fn job_cells(&self) -> Result<JobCells, String> {
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
        let mut complete = false;
        for _ in 0..200 {
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
                complete = true;
                break;
            }
        }
        if !complete && !JOB_LIST_WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            tracing::warn!("job discovery: more than 200 pages of job cells, the list is incomplete");
        }
        Ok(JobCells { jobs: out, complete })
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
    async fn registry_cell(&self) -> Result<Option<(Cell, RegistryData)>, String> {
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
                let r = parse_registry_data(&c.data)?;
                (r.service_pubkey == *mine).then_some((c, r))
            })
            .max_by_key(|(_, r)| r.expiry_ms))
    }

    /// The registration's stake now: `deposited` (stake rules 2–3 hold), `withdrawn` (the cell is dead)
    /// or `invalid` (live but not a deposit of at least 10,000 CKB).
    async fn stake_status(&self, stake: &OutPoint) -> Result<Value, String> {
        let op = out_point_str(stake);
        Ok(match self.live_cell(&op).await? {
            None => json!({"outpoint": op, "status": "withdrawn", "capacity_shannons": null}),
            Some((out, data)) => {
                let cap: u64 = out.capacity().unpack();
                let status = if stake_problem(&out, &data).is_none() {
                    "deposited"
                } else {
                    "invalid"
                };
                json!({"outpoint": op, "status": status, "capacity_shannons": cap})
            }
        })
    }

    pub async fn register_status(&self) -> Result<Value, String> {
        Ok(match self.registry_cell().await? {
            Some((c, r)) => {
                let stake = match &r.stake {
                    Some(op) => self.stake_status(op).await?,
                    None => Value::Null,
                };
                json!({
                    "registered": true,
                    "outpoint": out_point_str(&c.out_point),
                    "peer_id": r.peer_id,
                    "attestation_expiry_ms": r.expiry_ms,
                    "payout_lock_hash": hx(&r.payout_lock_hash),
                    "stake": stake,
                })
            }
            None => json!({
                "registered": false,
                "outpoint": null,
                "peer_id": null,
                "attestation_expiry_ms": null,
                "payout_lock_hash": null,
                "stake": null,
            }),
        })
    }

    /// The registry lock owned by the runner wallet: dual lock `runner_lock_hash ‖ runner_lock_hash`.
    fn runner_owned_lock(&self) -> Script {
        let h = self.runner_lock_hash();
        let mut args = h.as_bytes().to_vec();
        args.extend_from_slice(h.as_bytes());
        self.dual.script(&args)
    }

    /// The stake as a live, valid deposit: `(out point, lock args of its sighash owner)`.
    async fn checked_stake(&self, stake: &str) -> Result<(OutPoint, [u8; 20]), String> {
        let stake = parse_stake(stake)?;
        let (out, data) = self
            .live_cell(&stake)
            .await?
            .ok_or_else(|| format!("the stake {stake} is not a live cell"))?;
        if let Some(p) = stake_problem(&out, &data) {
            return Err(format!("{p} ({stake})"));
        }
        let owner = sighash_args(&out.lock()).ok_or_else(|| {
            format!(
                "the stake {stake} is not locked by a standard secp256k1_blake160 wallet lock, so its \
                 owner cannot consent by a signed message"
            )
        })?;
        let (h, i) = parse_out_point(&stake)?;
        Ok((out_point_from(&h, i)?, owner))
    }

    /// The text the stake owner signs for registering this runner with `stake` on the local node.
    pub async fn consent_message(&self, stake: &str) -> Result<Value, String> {
        let (stake, owner) = self.checked_stake(stake).await?;
        let info = self.call("local_node_info", json!([])).await?;
        let peer_id = info["node_id"]
            .as_str()
            .ok_or("the node reported no node_id")?;
        let stake_bytes: [u8; 36] = stake.as_slice().try_into().expect("36-byte out point");
        let message =
            crate::consent::message(&self.network, &stake_bytes, &self.service_pubkey, peer_id)?;
        Ok(json!({
            "message": message,
            "stake_owner_lock_args": hx(owner),
            "stake_owner_address": crate::address::sighash_address(&self.network, &owner),
        }))
    }

    /// Registers this node with `stake` and the stake owner's `consent` signature (`spec/registry.md` §9):
    /// creates the runner-owned registry cell, or updates the existing one (new stake and attestation).
    /// Paid from and signed by the runner wallet.
    pub async fn register_create(
        &self,
        attestation: &Value,
        stake: &str,
        consent: &str,
    ) -> Result<Value, String> {
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
        let (stake_point, owner) = self.checked_stake(stake).await?;
        // The consent: the text of this registration (the attestation's peer id), signed by the stake owner.
        let sig = crate::consent::parse_signature_hex(consent)
            .map_err(|e| format!("bad consent: {e}"))?;
        let peer_id = std::str::from_utf8(&att.peer_id)
            .map_err(|_| "attestation: peer_id is not UTF-8".to_string())?;
        let stake_bytes: [u8; 36] = stake_point.as_slice().try_into().expect("36 bytes");
        let text =
            crate::consent::message(&self.network, &stake_bytes, &self.service_pubkey, peer_id)?;
        let signer =
            crate::consent::recover_signer(&text, &sig).map_err(|e| format!("bad consent: {e}"))?;
        if signer != owner {
            return Err(format!(
                "the consent is not the stake owner's: the signature recovers to lock args {}, the \
                 stake belongs to {}; the owner must sign exactly the text of `csr register \
                 consent-message`",
                hx(signer),
                hx(owner)
            ));
        }

        let lock = self.runner_owned_lock();
        let existing = self
            .registry_cell()
            .await?
            .filter(|(c, _)| c.lock == lock)
            .map(|(c, _)| c);
        let type_ = self.registry_type();
        let data = Bytes::from(att.registry_data(&self.runner_lock_hash(), &stake_point));
        let cap = occupied(&lock, Some(&type_), data.len());
        let registry = Out {
            lock,
            type_: Some(type_),
            data,
            cap,
        };
        let mut deps = vec![
            code_dep(self.registry.code_cell.clone()),
            code_dep(config.out_point),
        ];
        if existing.is_some() {
            deps.push(self.dual.cell_dep());
        }
        let updated = existing.is_some();
        let tx = self
            .wallet_tx(
                existing.into_iter().collect(),
                vec![registry],
                deps,
                Some(code_dep(stake_point)),
                Some(sig),
            )
            .await?;
        let tx_hash = self.broadcast(&tx).await?;
        Ok(json!({
            "tx_hash": tx_hash,
            "outpoint": format!("{tx_hash}:0"),
            "stake_owner_lock_args": hx(owner),
            "updated": updated,
        }))
    }

    /// Destroys the runner-owned registry cell; its capacity returns to the runner wallet.
    pub async fn register_remove(&self) -> Result<Value, String> {
        let lock = self.runner_owned_lock();
        let (cell, _) = self
            .registry_cell()
            .await?
            .ok_or("this node is not registered")?;
        if cell.lock != lock {
            return Err(
                "the registry cell is not owned by the runner wallet (an A7 registration); only its \
                 owner's wallet can remove it"
                    .into(),
            );
        }
        let (config, _) = self.config_cell().await?;
        let capacity = cell.capacity;
        let deps = vec![
            self.dual.cell_dep(),
            code_dep(self.registry.code_cell.clone()),
            code_dep(config.out_point),
        ];
        let tx = self.wallet_tx(vec![cell], vec![], deps, None, None).await?;
        let tx_hash = self.broadcast(&tx).await?;
        Ok(json!({"tx_hash": tx_hash, "capacity_shannons": capacity}))
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

    // ---- Job runs (A3c) ----

    fn job_lock_script(&self) -> Script {
        Script::new_builder()
            .code_hash(self.job_lock.code_hash.pack())
            .hash_type(packed::Byte::new(self.job_lock.hash_type as u8))
            .args(Bytes::copy_from_slice(&self.registry_type_hash).pack())
            .build()
    }

    fn job_type_script(&self, service_code_hash: &[u8]) -> Script {
        Script::new_builder()
            .code_hash(self.job_type.code_hash.pack())
            .hash_type(packed::Byte::new(self.job_type.hash_type as u8))
            .args(Bytes::copy_from_slice(service_code_hash).pack())
            .build()
    }

    /// A live cell (output and data); `None` if it is dead or unknown.
    pub async fn live_cell(&self, outpoint: &str) -> Result<Option<(CellOutput, Vec<u8>)>, String> {
        let (h, i) = parse_out_point(outpoint)?;
        let r = self
            .call(
                "get_live_cell",
                json!([{"tx_hash": h, "index": format!("0x{i:x}")}, true]),
            )
            .await?;
        if r["status"] != "live" {
            return Ok(None);
        }
        let out: json::CellOutput =
            serde_json::from_value(r["cell"]["output"].clone()).map_err(|e| e.to_string())?;
        let data = r["cell"]["data"]["content"]
            .as_str()
            .and_then(|d| hex::decode(d.trim_start_matches("0x")).ok())
            .unwrap_or_default();
        Ok(Some((out.into(), data)))
    }

    pub async fn is_live(&self, outpoint: &str) -> Result<bool, String> {
        Ok(self.live_cell(outpoint).await?.is_some())
    }

    /// Everything a run transaction needs besides the job: a runner wallet cell and the deps. The
    /// registration must pay its tips into the runner wallet (`payout_lock_hash = runner_lock_hash`).
    pub async fn run_context(&self) -> Result<RunContext, String> {
        let (registry, reg) = self
            .registry_cell()
            .await?
            .ok_or("this node is not registered")?;
        if reg.payout_lock_hash != self.runner_lock_hash().as_bytes() {
            return Err(
                "the registry cell pays the tips to another lock (an A7 payout cell); register \
                 again (csr register create) so that they go into the runner wallet"
                    .into(),
            );
        }
        let runner = self
            .runner_cells()
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| {
                format!(
                    "the runner wallet is empty; send CKB to the runner address {}",
                    self.runner_address()
                )
            })?;
        let (config, clock_ms) = self.config_cell().await?;
        attestation_usable(reg.expiry_ms, clock_ms)?;
        Ok(RunContext {
            runner,
            deps: vec![
                self.secp_dep().await?,
                code_dep(self.job_lock.code_cell.clone()),
                code_dep(self.job_type.code_cell.clone()),
                code_dep(registry.out_point),
                code_dep(config.out_point),
            ],
        })
    }

    /// Finds the script with lock hash `want` among the cells around the transaction that created
    /// `outpoint`: its outputs and spent inputs, following earlier job cells backwards.
    pub async fn find_lock(&self, outpoint: &str, want: &[u8]) -> Result<Option<Script>, String> {
        let tx_of = |r: &Value| -> Value {
            let t = &r["transaction"];
            t.get("inner").unwrap_or(t).clone()
        };
        let job_lock = self.job_lock_script();
        let (mut hash, _) = parse_out_point(outpoint)?;
        for _ in 0..64 {
            let tx = tx_of(&self.call("get_transaction", json!([hash])).await?);
            let lock_of = |o: &Value| serde_json::from_value::<json::Script>(o["lock"].clone());
            for o in tx["outputs"].as_array().into_iter().flatten() {
                if let Ok(l) = lock_of(o) {
                    let l = Script::from(l);
                    if script_hash(&l).as_bytes() == want {
                        return Ok(Some(l));
                    }
                }
            }
            let mut previous_job = None;
            for input in tx["inputs"].as_array().into_iter().flatten() {
                let prev = &input["previous_output"];
                let Some(h) = prev["tx_hash"].as_str() else {
                    continue;
                };
                let idx = hex_u64(&prev["index"]).unwrap_or(0) as usize;
                let ptx = tx_of(&self.call("get_transaction", json!([h])).await?);
                let Ok(l) = lock_of(&ptx["outputs"][idx]) else {
                    continue;
                };
                let l = Script::from(l);
                if script_hash(&l).as_bytes() == want {
                    return Ok(Some(l));
                }
                if l == job_lock {
                    previous_job = Some(h.to_string());
                }
            }
            match previous_job {
                Some(h) => hash = h,
                None => return Ok(None),
            }
        }
        Ok(None)
    }

    /// Builds and signs the run transaction of `job` on top of `ctx` (see `job-cell-standard.md`
    /// §5, `registry.md` §9): inputs `[runner wallet cell, job, skeleton inputs…]`, outputs `[runner
    /// wallet cell + tip, recreated job | refund, skeleton outputs…]`. The fee comes from the job, so
    /// the wallet grows by exactly the tip.
    pub fn build_run(&self, ctx: &RunContext, job: &RunJob) -> Result<BuiltRun, String> {
        let key = self
            .service_key
            .as_ref()
            .ok_or("the service key is not available")?;
        let (h, i) = parse_out_point(&job.outpoint)?;
        let job_op = out_point_from(&h, i)?;
        let sk = &job.skeleton;
        let sk_in: u64 = job.skeleton_input_capacity;
        let sk_out: u64 = sk
            .as_ref()
            .map(|t| {
                t.raw()
                    .outputs()
                    .into_iter()
                    .map(|o| Unpack::<u64>::unpack(&o.capacity()))
                    .sum()
            })
            .unwrap_or(0);
        if sk_out > sk_in {
            return Err("the transaction skeleton creates capacity it does not bring".into());
        }
        let wallet_cap = ctx
            .runner
            .capacity
            .checked_add(job.tip_shannons)
            .ok_or("capacity overflow")?;
        let last = job.remaining_runs <= 1;
        let refund_lock = if last {
            Some(job.refund_lock.clone().ok_or(
                "the refund lock of the job is not known (its script is not found on chain)",
            )?)
        } else {
            None
        };
        let mut data = job.data.clone();
        if !last {
            let n = data.len();
            data[n - 4..].copy_from_slice(&(job.remaining_runs - 1).to_le_bytes());
        }
        let job_lock = self.job_lock_script();
        let job_type = self.job_type_script(&job.service_code_hash);
        let mut inputs = vec![
            CellInput::new(ctx.runner.out_point.clone(), 0),
            CellInput::new(job_op, job.since),
        ];
        let mut deps = ctx.deps.clone();
        let mut header_deps = Vec::new();
        let mut witnesses = vec![
            WitnessArgs::new_builder()
                .lock(Some(Bytes::from(vec![0u8; 65])).pack())
                .build()
                .as_bytes(),
            Bytes::new(),
        ];
        let mut extra_outputs = Vec::new();
        if let Some(t) = sk {
            let raw = t.raw();
            inputs.extend(raw.inputs());
            deps.extend(raw.cell_deps());
            header_deps.extend(raw.header_deps());
            for (o, d) in raw.outputs().into_iter().zip(raw.outputs_data()) {
                extra_outputs.push((o, d.raw_data()));
            }
            witnesses.extend(t.witnesses().into_iter().map(|w| w.raw_data()));
        }
        let build = |fee: u64| -> Result<TransactionView, String> {
            let job_out = job
                .capacity
                .checked_sub(job.tip_shannons)
                .and_then(|c| c.checked_sub(fee))
                .ok_or("the job cell cannot pay tip and fee")?;
            let third = match &refund_lock {
                Some(l) => CellOutput::new_builder()
                    .capacity(Pack::<packed::Uint64>::pack(&job_out))
                    .lock(l.clone())
                    .build(),
                None => CellOutput::new_builder()
                    .capacity(Pack::<packed::Uint64>::pack(&job_out))
                    .lock(job_lock.clone())
                    .type_(Some(job_type.clone()).pack())
                    .build(),
            };
            let third_data = if last {
                Bytes::new()
            } else {
                Bytes::from(data.clone())
            };
            let mut b = TransactionBuilder::default()
                .cell_deps(deps.clone())
                .header_deps(header_deps.clone())
                .inputs(inputs.clone())
                .output(
                    CellOutput::new_builder()
                        .capacity(Pack::<packed::Uint64>::pack(&wallet_cap))
                        .lock(ctx.runner.lock.clone())
                        .build(),
                )
                .output_data(Bytes::new().pack())
                .output(third)
                .output_data(third_data.pack());
            for (o, d) in &extra_outputs {
                b = b.output(o.clone()).output_data(d.pack());
            }
            Ok(
                b.witnesses(witnesses.iter().map(|w| w.pack()).collect::<Vec<_>>())
                    .build(),
            )
        };
        // 1,100 shannons per KB of the serialized transaction (the node's minimum is 1,000).
        let size = build(0)?.data().as_slice().len() as u64 + 4;
        let fee = (size * 1100).div_ceil(1000);
        if fee > job.max_fee_shannons {
            return Err(format!(
                "the fee ({fee} shannons) is above the job's max_fee_shannons ({})",
                job.max_fee_shannons
            ));
        }
        let tx = sign_wallet_inputs(&build(fee)?, 1, key)?;
        let hash = tx.hash();
        let op = |i: u32| {
            OutPoint::new_builder()
                .tx_hash(hash.clone())
                .index(Pack::<packed::Uint32>::pack(&i))
                .build()
        };
        let next = RunContext {
            runner: Cell {
                out_point: op(0),
                capacity: wallet_cap,
                ..ctx.runner.clone()
            },
            deps: ctx.deps.clone(),
        };
        Ok(BuiltRun {
            tx_hash: hx(hash.as_slice()),
            tx,
            fee,
            next,
        })
    }

    /// Broadcasts a run transaction; the error carries the node's message.
    pub async fn send(&self, tx: &TransactionView) -> Result<String, String> {
        let t = json::TransactionView::from(tx.clone()).inner;
        match self
            .rpc
            .call("send_transaction", json!([t, "passthrough"]))
            .await
        {
            Ok(h) => Ok(h.as_str().unwrap_or_default().to_string()),
            Err(RpcError::Rpc { message, .. }) => Err(message),
            Err(e) => Err(e.to_string()),
        }
    }

    /// `(status, block number)` of a transaction: `pending`, `proposed`, `committed`,
    /// `rejected` or `unknown`.
    pub async fn tx_status(&self, tx_hash: &str) -> Result<(String, Option<u64>), String> {
        let r = self.call("get_transaction", json!([tx_hash])).await?;
        let st = &r["tx_status"];
        let status = st["status"].as_str().unwrap_or("unknown").to_string();
        let mut block = hex_u64(&st["block_number"]);
        if status == "committed" && block.is_none() {
            if let Some(bh) = st["block_hash"].as_str() {
                let h = self.call("get_header", json!([bh])).await?;
                block = hex_u64(&h["number"]);
            }
        }
        Ok((status, block))
    }

    /// Total capacity of the live inputs of a skeleton (all of them must be live).
    pub async fn inputs_capacity(&self, tx: &packed::Transaction) -> Result<u64, String> {
        let mut sum = 0u64;
        for i in tx.raw().inputs() {
            let op = out_point_str(&i.previous_output());
            let (o, _) = self
                .live_cell(&op)
                .await?
                .ok_or_else(|| format!("skeleton input {op} is not live"))?;
            sum = sum
                .checked_add(Unpack::<u64>::unpack(&o.capacity()))
                .ok_or("capacity overflow")?;
        }
        Ok(sum)
    }

    /// Molecule `CellOutput` and data of a live cell, molecule `Header` or `Transaction` for the
    /// `chain_read` service call.
    pub async fn read_for_service(&self, kind: u64, key: &[u8]) -> Result<Option<Vec<u8>>, String> {
        match kind {
            0 | 1 => {
                let h = hx(&key[..32]);
                let i = u32::from_le_bytes(key[32..36].try_into().unwrap());
                Ok(self.live_cell(&format!("{h}:{i}")).await?.map(|(o, d)| {
                    if kind == 0 {
                        o.as_slice().to_vec()
                    } else {
                        d
                    }
                }))
            }
            2 => {
                let r = self.call("get_header", json!([hx(key)])).await?;
                if r.is_null() {
                    return Ok(None);
                }
                let h: json::HeaderView = serde_json::from_value(r).map_err(|e| e.to_string())?;
                Ok(Some(packed::Header::from(h.inner).as_slice().to_vec()))
            }
            _ => {
                let r = self.call("get_transaction", json!([hx(key)])).await?;
                if r["transaction"].is_null() {
                    return Ok(None);
                }
                let t: json::TransactionView =
                    serde_json::from_value(r["transaction"].clone()).map_err(|e| e.to_string())?;
                Ok(Some(packed::Transaction::from(t.inner).as_slice().to_vec()))
            }
        }
    }

    /// Out points of live cells whose lock (`script_type` 0) or type (1) starts with `script`
    /// (args as prefix), at most `limit`.
    pub async fn query_for_service(
        &self,
        script: &Script,
        script_type: u64,
        limit: usize,
    ) -> Result<Vec<OutPoint>, String> {
        self.wait_indexer().await?;
        let key = json!({
            "script": json::Script::from(script.clone()),
            "script_type": if script_type == 0 { "lock" } else { "type" },
            "script_search_mode": "prefix",
            "with_data": false,
        });
        let r = self
            .call(
                "get_cells",
                json!([key, "asc", format!("0x{limit:x}"), Value::Null]),
            )
            .await?;
        let mut out = Vec::new();
        for c in r["objects"].as_array().into_iter().flatten().take(limit) {
            let op: json::OutPoint =
                serde_json::from_value(c["out_point"].clone()).map_err(|e| e.to_string())?;
            out.push(op.into());
        }
        Ok(out)
    }
}

/// Runner wallet cell and cell deps of run transactions; advanced after each submitted run so that
/// runs of one batch chain on each other's outputs.
#[derive(Clone)]
pub struct RunContext {
    pub runner: Cell,
    deps: Vec<CellDep>,
}

/// A job ready for its run transaction.
pub struct RunJob {
    pub outpoint: String,
    pub service_code_hash: Vec<u8>,
    pub capacity: u64,
    pub data: Vec<u8>,
    pub since: u64,
    pub tip_shannons: u64,
    pub max_fee_shannons: u64,
    pub remaining_runs: u32,
    pub refund_lock: Option<Script>,
    pub skeleton: Option<packed::Transaction>,
    pub skeleton_input_capacity: u64,
}

pub struct BuiltRun {
    pub tx: TransactionView,
    pub tx_hash: String,
    pub fee: u64,
    pub next: RunContext,
}

/// `0x<tx hash>:<index>` → (tx hash, index).
pub fn parse_out_point(s: &str) -> Result<(String, u32), String> {
    let (h, i) = s
        .split_once(':')
        .ok_or_else(|| format!("bad outpoint {s}"))?;
    let i: u32 = i.parse().map_err(|_| format!("bad outpoint {s}"))?;
    Ok((h.to_string(), i))
}

pub fn read_json(p: &Path) -> Result<Value, String> {
    let t = std::fs::read_to_string(p).map_err(|e| format!("cannot read {}: {e}", p.display()))?;
    serde_json::from_str(&t).map_err(|e| format!("{} is not JSON: {e}", p.display()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn embedded_testnet_deployment_parses() {
        let d = parse_deployments(TESTNET_DEPLOYMENTS, "deployments/testnet.json").unwrap();
        assert_eq!(
            format!("{:x}", d.registry.code_hash),
            "411aceb357a61684226169902cab2ccf64f470f0aaf93687d7cd2904f3c741d8"
        );
        assert_eq!(d.registry.dep_type, DepType::Code);
    }

    #[test]
    fn deployments_reject_unknown_dep_type() {
        let mut v: Value = serde_json::from_str(TESTNET_DEPLOYMENTS).unwrap();
        v["scripts"]["registry"]["dep_type"] = json!("group");
        assert!(parse_deployments(&v.to_string(), "x").is_err());
    }

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
    fn registry_data_roundtrip() {
        let att = Attestation::parse(&json!({
            "peer_id": "QmPeer", "service_pubkey": format!("0x02{}", "11".repeat(32)),
            "network": "devnet", "expiry_ms": 12345, "signature": format!("0x{}", "22".repeat(65)),
        }))
        .unwrap();
        let stake = out_point_from(&format!("0x{}", "5a".repeat(32)), 3).unwrap();
        let data = att.registry_data(&H256::from_slice(&[7u8; 32]).unwrap(), &stake);
        let r = parse_registry_data(&data).unwrap();
        assert_eq!(r.peer_id, "QmPeer");
        assert_eq!(r.service_pubkey.len(), 33);
        assert_eq!(r.payout_lock_hash, vec![7u8; 32]);
        assert_eq!(r.expiry_ms, 12345);
        assert_eq!(r.stake.unwrap(), stake);
    }

    #[test]
    fn registry_v2_vector() {
        let v = "7501000020000000540000007500000095000000960000004d0100005101000030000000516d5465737450656572313233343536373839306162636465666768696a6b6c6d6e6f707172737475767778797a414202466d7fcae563e5cb09a0d1870bb580344804617879a14949cf22285f1bae3f27777777777777777777777777777777777777777777777777777777777777777701b7000000180000004c0000006d0000006e0000007600000030000000516d5465737450656572313233343536373839306162636465666768696a6b6c6d6e6f707172737475767778797a414202466d7fcae563e5cb09a0d1870bb580344804617879a14949cf22285f1bae3f270100505c18a301000004d5db9878eceb46d700d0c55677777b3b1d2a401773b24dae15789c071ef7d36794160555e176e0b1ca6b997892e7c9ce7320b2a2016e3102c46a495421ed5f01000000005a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a03000000";
        let r = parse_registry_data(&hex::decode(v).unwrap()).unwrap();
        assert_eq!(
            r.peer_id,
            "QmTestPeer1234567890abcdefghijklmnopqrstuvwxyzAB"
        );
        assert_eq!(r.service_pubkey[0], 2);
        assert_eq!(r.payout_lock_hash, vec![0x77u8; 32]);
        assert_eq!(r.expiry_ms, 1_800_000_000_000);
        let stake = r.stake.unwrap();
        assert_eq!(out_point_str(&stake), format!("0x{}:3", "5a".repeat(32)));
        // The runner writes the same bytes.
        let att = Attestation::parse(&json!({
            "peer_id": "QmTestPeer1234567890abcdefghijklmnopqrstuvwxyzAB",
            "service_pubkey": "0x02466d7fcae563e5cb09a0d1870bb580344804617879a14949cf22285f1bae3f27",
            "network": "testnet", "expiry_ms": 1_800_000_000_000u64,
            "signature": "0x04d5db9878eceb46d700d0c55677777b3b1d2a401773b24dae15789c071ef7d36794160555e176e0b1ca6b997892e7c9ce7320b2a2016e3102c46a495421ed5f01",
        }))
        .unwrap();
        let data = att.registry_data(&H256::from_slice(&[0x77u8; 32]).unwrap(), &stake);
        assert_eq!(hex::encode(data), v);
    }

    #[test]
    fn stake_out_points() {
        let h = "ab".repeat(32);
        assert_eq!(
            parse_stake(&format!("0x{}:2", h.to_uppercase())).unwrap(),
            format!("0x{h}:2")
        );
        for bad in [
            "0x12:0".to_string(),
            format!("0x{h}"),
            format!("0x{h}:x"),
            format!("0x{h}:"),
            format!("0x{h}:-1"),
            format!("{h}:0"),
            format!("0x{h}:4294967296"),
        ] {
            assert!(parse_stake(&bad).unwrap_err().contains("stake"), "{bad}");
        }
    }

    #[test]
    fn stake_rules() {
        let owner = sighash_lock(&[1u8; 20]);
        let dao = Script::new_builder()
            .code_hash(DAO_TYPE_CODE_HASH.pack())
            .hash_type(packed::Byte::new(ScriptHashType::Type as u8))
            .build();
        let cell = |cap: u64, lock: &Script, ty: Option<Script>| {
            CellOutput::new_builder()
                .capacity(Pack::<packed::Uint64>::pack(&cap))
                .lock(lock.clone())
                .type_(ty.pack())
                .build()
        };
        let ok = cell(MIN_STAKE, &owner, Some(dao.clone()));
        assert_eq!(stake_problem(&ok, &[0; 8]), None);
        let p = |o: &CellOutput, d: &[u8]| stake_problem(o, d).unwrap();
        assert!(p(&ok, &[1, 0, 0, 0, 0, 0, 0, 0]).contains("DAO"));
        assert!(p(&ok, &[]).contains("DAO"));
        assert!(p(&cell(MIN_STAKE, &owner, None), &[0; 8]).contains("DAO"));
        assert!(p(&cell(MIN_STAKE - 1, &owner, Some(dao)), &[0; 8]).contains("minimum"));
        // The owner may be any wallet; only the sighash `type` form can consent by a signed message.
        assert_eq!(sighash_args(&owner), Some([1u8; 20]));
        let data_form = owner
            .clone()
            .as_builder()
            .hash_type(packed::Byte::new(ScriptHashType::Data1 as u8))
            .build();
        assert_eq!(sighash_args(&data_form), None);
        let long = sighash_lock(&[1u8; 21]);
        assert_eq!(sighash_args(&long), None);
    }

    #[test]
    fn wallet_signature_covers_the_consent() {
        let key = secp256k1::SecretKey::from_slice(&[3u8; 32]).unwrap();
        let w = WitnessArgs::new_builder()
            .output_type(Some(Bytes::from(vec![9u8; 65])).pack())
            .build();
        let tx = TransactionBuilder::default()
            .input(CellInput::new(OutPoint::default(), 0))
            .input(CellInput::new(OutPoint::default(), 0))
            .witness(w.as_bytes().pack())
            .witness(Bytes::new().pack())
            .build();
        let signed = sign_wallet_inputs(&tx, 2, &key).unwrap();
        let w0 = WitnessArgs::from_slice(&signed.witnesses().get(0).unwrap().raw_data()).unwrap();
        assert_eq!(w0.output_type().to_opt().unwrap().raw_data().len(), 65);
        assert_eq!(w0.lock().to_opt().unwrap().raw_data().len(), 65);
        // another consent gives another signature
        let w2 = WitnessArgs::new_builder()
            .output_type(Some(Bytes::from(vec![8u8; 65])).pack())
            .build();
        let tx2 = tx
            .as_advanced_builder()
            .set_witnesses(vec![w2.as_bytes().pack(), Bytes::new().pack()])
            .build();
        let s2 = sign_wallet_inputs(&tx2, 2, &key).unwrap();
        let l2 = WitnessArgs::from_slice(&s2.witnesses().get(0).unwrap().raw_data()).unwrap();
        assert_ne!(w0.lock().as_slice(), l2.lock().as_slice());
    }

    #[test]
    fn attestation_needs_known_network() {
        let v = json!({"peer_id": "Qm", "service_pubkey": format!("0x02{}", "11".repeat(32)),
            "network": "moon", "expiry_ms": 1, "signature": format!("0x{}", "22".repeat(65))});
        assert!(Attestation::parse(&v).is_err());
    }

    #[test]
    fn wallet_cell_occupies_61_ckb() {
        let l = sighash_lock(&[0; 20]);
        assert_eq!(occupied(&l, None, 0), 61 * CKB);
    }
}

/// The indexer is usable when it is at most `LAG_LIMIT` blocks behind the node tip (as in R2).
pub fn indexer_ready(tip: u64, indexer_tip: u64) -> bool {
    tip.saturating_sub(indexer_tip) <= crate::node::r2::LAG_LIMIT
}

/// The attestation may be used while its expiry lies after the chain clock.
pub fn attestation_usable(expiry_ms: u64, clock_ms: u64) -> Result<(), String> {
    if expiry_ms > clock_ms {
        Ok(())
    } else {
        Err(format!(
            "attestation expired (expiry {expiry_ms} ms, chain time {clock_ms} ms)"
        ))
    }
}
