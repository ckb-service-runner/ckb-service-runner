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
    })
}

/// A deployed script: how to reference it and where its code cell is.
struct Deployed {
    code_hash: H256,
    hash_type: ScriptHashType,
    code_cell: OutPoint,
}

fn load_float_deployment(cfg: &Config) -> Result<Deployed, String> {
    let Some(dev) = cfg.devnet.as_ref().filter(|_| cfg.network == "devnet") else {
        // TODO: shipped script hashes of testnet/mainnet once the scripts are deployed there.
        return Err(format!(
            "the CSR scripts are not deployed on {} yet",
            cfg.network
        ));
    };
    let text = std::fs::read_to_string(&dev.deployments)
        .map_err(|e| format!("cannot read {}: {e}", dev.deployments))?;
    let v: Value =
        serde_json::from_str(&text).map_err(|e| format!("{} is not JSON: {e}", dev.deployments))?;
    let s = &v["scripts"]["float_lock"];
    let code_hash: H256 = s["code_hash"]
        .as_str()
        .unwrap_or("")
        .trim_start_matches("0x")
        .parse()
        .map_err(|_| format!("{}: bad float_lock.code_hash", dev.deployments))?;
    let hash_type = match s["hash_type"].as_str().unwrap_or("") {
        "data" => ScriptHashType::Data,
        "data1" => ScriptHashType::Data1,
        "data2" => ScriptHashType::Data2,
        "type" => ScriptHashType::Type,
        _ => return Err(format!("{}: bad float_lock.hash_type", dev.deployments)),
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

pub struct Chain {
    rpc: RpcClient,
    float: Deployed,
    runner_lock: Script,
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
        let float = load_float_deployment(cfg)?;
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
            float,
            runner_lock: sighash_lock(&blake2b_256(pk)[..20]),
        })
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
        fixed: Vec<(Script, u64)>,
        mut deps: Vec<CellDep>,
    ) -> Result<Prepared, String> {
        deps.insert(0, self.secp_dep().await?);
        let change_min = occupied(owner, None, 0);
        let in_pre: u64 = pre.iter().map(|c| c.capacity).sum();
        let out_fixed: u64 = fixed.iter().map(|o| o.1).sum();
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
        witnesses[owner_first] = WitnessArgs::new_builder()
            .lock(Some(Bytes::from(vec![0u8; 65])).pack())
            .build()
            .as_bytes();
        let build = |change: u64| -> TransactionView {
            let mut b = TransactionBuilder::default()
                .cell_deps(deps.clone())
                .inputs(inputs.clone())
                .witnesses(witnesses.iter().map(|w| w.pack()).collect::<Vec<_>>());
            for (lock, cap) in fixed.iter().chain([&(owner.clone(), change)]) {
                b = b
                    .output(
                        CellOutput::new_builder()
                            .capacity(Pack::<packed::Uint64>::pack(cap))
                            .lock(lock.clone())
                            .build(),
                    )
                    .output_data(Bytes::new().pack());
            }
            b.build()
        };
        // 2 shannons per byte (the node's minimum is 1) plus a constant.
        let fee = build(have - out_fixed).data().as_slice().len() as u64 * 2 + 1000;
        let change = have - out_fixed - fee;
        if change < change_min {
            return Err("the owner's cells cannot cover the change output and the fee".into());
        }
        let tx = build(change);
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
        self.assemble(&owner, vec![], vec![(lock, cap)], vec![])
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
            vec![(lock, cap)],
            vec![self.float_dep()],
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
            vec![(f.cell.lock.clone(), left)]
        };
        self.assemble(&owner, vec![f.cell], fixed, vec![self.float_dep()])
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
    fn float_cell_occupies_113_ckb() {
        let l = sighash_lock(&[0; 72]);
        assert_eq!(occupied(&l, None, 0), 113 * CKB);
    }
}
