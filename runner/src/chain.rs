//! On-chain operator flows: runner key cell, payout cell and transaction submission.
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
    /// The anyone-can-pay lock of the payout cell.
    acp: Deployed,
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
        acp: load_deployed(&v, "anyone_can_pay", f)?,
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
    fn registry_data(&self, payout_lock_hash: &H256) -> Vec<u8> {
        mol_table(&[
            mol_bytes(&self.peer_id),
            self.pubkey.clone(),
            payout_lock_hash.as_bytes().to_vec(),
            vec![self.network],
            self.molecule(),
            mol_bytes(&[]),
        ])
    }
}

/// `(peer_id, service_pubkey, payout_lock_hash, expiry_ms)` of a molecule `Registry` (6 fields,
/// or 7 with the `stake` out point of registry v2).
fn parse_registry_data(d: &[u8]) -> Option<(String, Vec<u8>, Vec<u8>, u64)> {
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
    acp: Deployed,
    /// `payout.lock_args` (anyone-can-pay args of the payout cell), if configured.
    payout_args: Option<Vec<u8>>,
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
            acp: dep.acp,
            payout_args: cfg.payout.args()?,
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

    /// `payout.lock_args`; every payout command needs it.
    fn payout_args(&self) -> Result<&[u8], String> {
        self.payout_args.as_deref().ok_or_else(|| {
            "payout.lock_args is not set: put the 20-byte args of your payout (anyone-can-pay) cell \
             into [payout] lock_args of the config"
                .to_string()
        })
    }

    /// The anyone-can-pay lock of the payout cell.
    pub fn payout_lock(&self) -> Result<Script, String> {
        Ok(self.acp.script(self.payout_args()?))
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

    /// The payout cell: the oldest live cell under the exact payout lock, without type and data.
    pub async fn payout(&self) -> Result<Option<Cell>, String> {
        let lock = self.payout_lock()?;
        let cells = self
            .get_cells(json!({
                "script": json::Script::from(lock),
                "script_type": "lock",
                "script_search_mode": "exact",
                "filter": {"script_len_range": ["0x0", "0x1"], "output_data_len_range": ["0x0", "0x1"]},
            }))
            .await?;
        Ok(cells.into_iter().next())
    }

    pub async fn payout_status(&self) -> Result<Value, String> {
        let lock = self.payout_lock()?;
        let cell = self.payout().await?;
        Ok(json!({
            "found": cell.is_some(),
            "outpoint": cell.as_ref().map(|c| out_point_str(&c.out_point)),
            "capacity_shannons": cell.as_ref().map(|c| c.capacity),
            "lock_args": hx(lock.args().raw_data()),
            "lock_hash": hx(script_hash(&lock).as_bytes()),
        }))
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

    /// Unsigned transaction creating the payout cell with `ckb` CKB, paid by the secp256k1 lock
    /// of `owner_lock_args`.
    pub async fn prepare_payout_create(
        &self,
        ckb: &str,
        owner_lock_args: &str,
    ) -> Result<Prepared, String> {
        let lock = self.payout_lock()?;
        let owner = sighash_lock(&parse_lock_args(owner_lock_args)?);
        let cap = ckb_to_shannons(ckb)?;
        let min = occupied(&lock, None, 0);
        if cap < min {
            return Err(format!("a payout cell needs at least {} CKB", min / CKB));
        }
        if let Some(c) = self.payout().await? {
            return Err(format!(
                "a payout cell already exists ({}); anyone can add CKB to it",
                out_point_str(&c.out_point)
            ));
        }
        self.assemble(&owner, vec![], vec![Out::plain(lock, cap)], vec![], false)
            .await
    }

    /// Unsigned transaction moving `ckb` CKB (or everything with `all`) from the payout cell to the
    /// secp256k1 lock of `payout.lock_args`. The payout cell is spent on the anyone-can-pay
    /// signature path; the fee comes out of the withdrawn amount. If what arrives is below a
    /// cell's minimum capacity, it is added to one of the receiver's plain cells instead.
    pub async fn prepare_payout_withdraw(
        &self,
        ckb: Option<&str>,
        all: bool,
    ) -> Result<Prepared, String> {
        let receiver = sighash_lock(self.payout_args()?);
        let payout = self
            .payout()
            .await?
            .ok_or("no payout cell found; use prepare-create")?;
        let keep = occupied(&payout.lock, None, 0);
        let (amount, left) = if all {
            (payout.capacity, None)
        } else {
            let amount = ckb_to_shannons(ckb.ok_or("give --ckb N or --all")?)?;
            let left = payout
                .capacity
                .checked_sub(amount)
                .filter(|l| *l >= keep)
                .ok_or_else(|| {
                    format!(
                        "the payout cell keeps at least {} CKB (its occupied capacity); use --all to close it",
                        keep / CKB
                    )
                })?;
            (amount, Some(left))
        };
        if amount == 0 {
            return Err("nothing to withdraw".into());
        }
        let change_min = occupied(&receiver, None, 0);
        let mut inputs = vec![payout.clone()];
        let mut into = 0u64;
        // Upper bound of the fee: the transaction is far below 10 KB.
        if amount < change_min + 20_000 {
            let cell = self
                .plain_cells(&receiver)
                .await?
                .into_iter()
                .next()
                .ok_or_else(|| {
                    format!(
                        "{} CKB minus the fee is below the {} CKB a cell needs, and the receiving \
                         lock has no cell to add it to; withdraw more",
                        amount as f64 / CKB as f64,
                        change_min / CKB
                    )
                })?;
            into = cell.capacity;
            inputs.push(cell);
        }
        let deps = vec![self.secp_dep().await?, self.acp.cell_dep()];
        let placeholder = WitnessArgs::new_builder()
            .lock(Some(Bytes::from(vec![0u8; 65])).pack())
            .build()
            .as_bytes();
        let build = |fee: u64| -> TransactionView {
            let mut b = TransactionBuilder::default()
                .cell_deps(deps.clone())
                .inputs(
                    inputs
                        .iter()
                        .map(|c| CellInput::new(c.out_point.clone(), 0)),
                )
                .witnesses(vec![placeholder.pack(); inputs.len()]);
            let mut outs = Vec::new();
            if let Some(left) = left {
                outs.push(Out::plain(payout.lock.clone(), left));
            }
            outs.push(Out::plain(receiver.clone(), into + amount - fee));
            for o in outs {
                b = b
                    .output(
                        CellOutput::new_builder()
                            .capacity(Pack::<packed::Uint64>::pack(&o.cap))
                            .lock(o.lock)
                            .build(),
                    )
                    .output_data(o.data.pack());
            }
            b.build()
        };
        // 2 shannons per byte (the node's minimum is 1) plus a constant.
        let fee = build(0).data().as_slice().len() as u64 * 2 + 1000;
        if into + amount < fee + change_min {
            return Err("the withdrawn amount cannot cover the fee".into());
        }
        let tx = build(fee);
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
                let (peer, pk, payout, exp) = parse_registry_data(&c.data)?;
                (pk == *mine).then_some((c, peer, payout, exp))
            })
            .max_by_key(|r| r.3))
    }

    pub async fn register_status(&self) -> Result<Value, String> {
        Ok(match self.registry_cell().await? {
            Some((c, peer, payout, exp)) => json!({
                "registered": true,
                "outpoint": out_point_str(&c.out_point),
                "peer_id": peer,
                "attestation_expiry_ms": exp,
                "payout_lock_hash": hx(payout),
            }),
            None => json!({
                "registered": false,
                "outpoint": null,
                "peer_id": null,
                "attestation_expiry_ms": null,
                "payout_lock_hash": null,
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
        // Tips go to the payout lock; the cell itself may be created later (anyone can).
        let payout_lock = self.payout_lock()?;
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
        let data = Bytes::from(att.registry_data(&script_hash(&payout_lock)));
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

    /// Everything a run transaction needs besides the job: payout cell, runner key cell and deps.
    pub async fn run_context(&self) -> Result<RunContext, String> {
        let payout = self
            .payout()
            .await?
            .ok_or("no payout cell found; create the payout cell first")?;
        let runner = self
            .runner_cells()
            .await?
            .into_iter()
            .next()
            .ok_or("no runner key cell found")?;
        let (registry, _, payout_hash, _) = self
            .registry_cell()
            .await?
            .ok_or("this node is not registered")?;
        if payout_hash != script_hash(&payout.lock).as_bytes() {
            return Err(
                "the registry cell points to another payout lock (payout.lock_args changed?)"
                    .into(),
            );
        }
        let (config, _) = self.config_cell().await?;
        let code = |d: &Deployed| {
            CellDep::new_builder()
                .out_point(d.code_cell.clone())
                .dep_type(packed::Byte::new(DepType::Code as u8))
                .build()
        };
        let cell_dep = |op: OutPoint| {
            CellDep::new_builder()
                .out_point(op)
                .dep_type(packed::Byte::new(DepType::Code as u8))
                .build()
        };
        Ok(RunContext {
            payout,
            runner,
            deps: vec![
                self.secp_dep().await?,
                code(&self.job_lock),
                code(&self.job_type),
                self.acp.cell_dep(),
                cell_dep(registry.out_point),
                cell_dep(config.out_point),
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
    /// §5): inputs `[runner key cell, job, payout cell, skeleton inputs…]`, outputs `[runner key
    /// cell, payout cell + tip, recreated job | refund, skeleton outputs…]`. The fee comes from the
    /// job. The payout cell is spent on the anyone-can-pay path (empty witness, capacity grows).
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
        let payout_cap = ctx
            .payout
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
            CellInput::new(ctx.payout.out_point.clone(), 0),
        ];
        let mut deps = ctx.deps.clone();
        let mut header_deps = Vec::new();
        let mut witnesses = vec![
            WitnessArgs::new_builder()
                .lock(Some(Bytes::from(vec![0u8; 65])).pack())
                .build()
                .as_bytes(),
            Bytes::new(),
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
                        .capacity(Pack::<packed::Uint64>::pack(&ctx.runner.capacity))
                        .lock(ctx.runner.lock.clone())
                        .build(),
                )
                .output_data(Bytes::new().pack())
                .output(
                    CellOutput::new_builder()
                        .capacity(Pack::<packed::Uint64>::pack(&payout_cap))
                        .lock(ctx.payout.lock.clone())
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
        let tx = sign_first_input(&build(fee)?, key)?;
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
                ..ctx.runner.clone()
            },
            payout: Cell {
                out_point: op(1),
                capacity: payout_cap,
                ..ctx.payout.clone()
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

/// Payout cell, runner key cell and cell deps of run transactions; advanced after each submitted
/// run so that runs of one batch chain on each other's outputs.
#[derive(Clone)]
pub struct RunContext {
    pub payout: Cell,
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
    #[test]
    fn embedded_testnet_deployment_parses() {
        let d = parse_deployments(TESTNET_DEPLOYMENTS, "deployments/testnet.json").unwrap();
        assert_eq!(
            format!("{:x}", d.registry.code_hash),
            "47fee83ea91eabdfc8827045f63fe051b8fee5c05b9280a35f42f2e3597d3f36"
        );
        assert_eq!(
            format!("{:x}", d.acp.code_hash),
            "3419a1c09eb2567f6552ee7a8ecffd64155cffe0f1796e6e61ec088d740c1356"
        );
        assert_eq!(d.acp.hash_type, ScriptHashType::Type);
        assert_eq!(d.acp.dep_type, DepType::DepGroup);
        assert_eq!(d.registry.dep_type, DepType::Code);
    }

    #[test]
    fn deployments_reject_unknown_dep_type() {
        let mut v: Value = serde_json::from_str(TESTNET_DEPLOYMENTS).unwrap();
        v["scripts"]["anyone_can_pay"]["dep_type"] = json!("group");
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
        let (peer, pk, payout, exp) = parse_registry_data(&data).unwrap();
        assert_eq!(peer, "QmPeer");
        assert_eq!(pk.len(), 33);
        assert_eq!(payout, vec![7u8; 32]);
        assert_eq!(exp, 12345);
    }

    #[test]
    fn registry_v2_vector_parses() {
        let v = "7501000020000000540000007500000095000000960000004d0100005101000030000000516d5465737450656572313233343536373839306162636465666768696a6b6c6d6e6f707172737475767778797a414202466d7fcae563e5cb09a0d1870bb580344804617879a14949cf22285f1bae3f27777777777777777777777777777777777777777777777777777777777777777701b7000000180000004c0000006d0000006e0000007600000030000000516d5465737450656572313233343536373839306162636465666768696a6b6c6d6e6f707172737475767778797a414202466d7fcae563e5cb09a0d1870bb580344804617879a14949cf22285f1bae3f270100505c18a301000004d5db9878eceb46d700d0c55677777b3b1d2a401773b24dae15789c071ef7d36794160555e176e0b1ca6b997892e7c9ce7320b2a2016e3102c46a495421ed5f01000000005a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a03000000";
        let (peer, pk, payout, exp) = parse_registry_data(&hex::decode(v).unwrap()).unwrap();
        assert_eq!(peer, "QmTestPeer1234567890abcdefghijklmnopqrstuvwxyzAB");
        assert_eq!(pk[0], 2);
        assert_eq!(payout, vec![0x77u8; 32]);
        assert_eq!(exp, 1_800_000_000_000);
    }

    #[test]
    fn attestation_needs_known_network() {
        let v = json!({"peer_id": "Qm", "service_pubkey": format!("0x02{}", "11".repeat(32)),
            "network": "moon", "expiry_ms": 1, "signature": format!("0x{}", "22".repeat(65))});
        assert!(Attestation::parse(&v).is_err());
    }

    #[test]
    fn payout_cell_occupies_61_ckb() {
        let l = sighash_lock(&[0; 20]);
        assert_eq!(occupied(&l, None, 0), 61 * CKB);
    }
}
