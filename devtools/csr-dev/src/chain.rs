//! Chain access: cells, transaction building, signing and waiting for commitment.

use crate::keys::{h, script_hash, sighash_lock, Key, SIGHASH_CODE_HASH};
use crate::rpc::{hex_u64, Rpc};
use ckb_hash::{blake2b_256, new_blake2b};
use ckb_jsonrpc_types as json;
use ckb_types::{
    bytes::Bytes,
    core::{DepType, ScriptHashType, TransactionBuilder, TransactionView},
    packed::{self, CellDep, CellInput, CellOutput, OutPoint, Script, WitnessArgs},
    prelude::*,
    H256,
};
use secp256k1::{Message, Secp256k1};
use serde_json::{json, Value};

pub const CKB: u64 = 100_000_000;
const TYPE_ID_CODE_HASH: [u8; 32] = {
    let mut a = [0u8; 32];
    let s = b"TYPE_ID";
    let mut i = 0;
    while i < s.len() {
        a[25 + i] = s[i];
        i += 1;
    }
    a
};

pub struct Chain {
    pub rpc: Rpc,
    pub devnet: bool,
}

#[derive(Clone)]
pub struct LiveCell {
    pub out_point: OutPoint,
    pub capacity: u64,
}

pub struct Out {
    pub capacity: u64,
    pub lock: Script,
    pub type_: Option<Script>,
    pub data: Vec<u8>,
    /// Make `type_` a Type ID script (args filled in from the first input).
    pub type_id: bool,
}

pub fn out_point(tx_hash: &str, index: u32) -> Result<OutPoint, String> {
    let hash: H256 = tx_hash
        .trim_start_matches("0x")
        .parse()
        .map_err(|e| format!("bad tx hash {tx_hash}: {e}"))?;
    Ok(OutPoint::new_builder()
        .tx_hash(hash.pack())
        .index(Pack::<packed::Uint32>::pack(&index))
        .build())
}

pub fn type_id_script(args: &[u8]) -> Script {
    Script::new_builder()
        .code_hash(TYPE_ID_CODE_HASH.pack())
        .hash_type(packed::Byte::new(ScriptHashType::Type as u8))
        .args(Bytes::copy_from_slice(args).pack())
        .build()
}

pub fn occupied(lock: &Script, type_: &Option<Script>, data_len: usize) -> u64 {
    let script = |s: &Script| 32 + 1 + s.args().raw_data().len() as u64;
    8 + script(lock) + type_.as_ref().map(script).unwrap_or(0) + data_len as u64
}

impl Chain {
    /// Connects and refuses everything but a dev chain or testnet.
    pub fn connect(url: &str) -> Result<Chain, String> {
        let rpc = Rpc::new(url)?;
        let info = rpc.call("get_blockchain_info", json!([]))?;
        let name = info["chain"].as_str().unwrap_or("").to_string();
        if name != "ckb_dev" && name != "ckb_testnet" {
            return Err(format!(
                "refusing to run: node reports chain '{name}', csr-dev only works on ckb_dev or ckb_testnet"
            ));
        }
        let devnet = name == "ckb_dev";
        Ok(Chain { rpc, devnet })
    }

    pub fn network_name(&self) -> &'static str {
        if self.devnet {
            "devnet"
        } else {
            "testnet"
        }
    }

    pub fn tip_timestamp(&self) -> Result<u64, String> {
        hex_u64(&self.rpc.call("get_tip_header", json!([]))?["timestamp"])
    }

    pub fn wait_indexer(&self) -> Result<(), String> {
        for _ in 0..200 {
            let tip = self.rpc.tip_number()?;
            let it = self.rpc.call("get_indexer_tip", json!([]))?;
            if it.is_object() && hex_u64(&it["block_number"])? >= tip {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        Err("indexer does not catch up with the tip".into())
    }

    /// Pure cells (no type script, no data) under a lock script.
    pub fn plain_cells(&self, lock: &Script) -> Result<Vec<LiveCell>, String> {
        self.wait_indexer()?;
        let key = json!({
            "script": json::Script::from(lock.clone()),
            "script_type": "lock",
            "filter": {"script_len_range": ["0x0", "0x1"], "output_data_len_range": ["0x0", "0x1"]},
        });
        let r = self
            .rpc
            .call("get_cells", json!([key, "asc", "0x400", Value::Null]))?;
        let mut cells = Vec::new();
        for c in r["objects"].as_array().ok_or("get_cells: no objects")? {
            cells.push(LiveCell {
                out_point: out_point(
                    c["out_point"]["tx_hash"].as_str().unwrap_or(""),
                    hex_u64(&c["out_point"]["index"])? as u32,
                )?,
                capacity: hex_u64(&c["output"]["capacity"])?,
            });
        }
        Ok(cells)
    }

    /// First live cell with this type script, with its data.
    pub fn cell_by_type(
        &self,
        type_: &Script,
    ) -> Result<Option<(LiveCell, Vec<u8>, Script)>, String> {
        self.wait_indexer()?;
        let key = json!({
            "script": json::Script::from(type_.clone()),
            "script_type": "type",
            "with_data": true,
        });
        let r = self
            .rpc
            .call("get_cells", json!([key, "asc", "0x1", Value::Null]))?;
        let Some(c) = r["objects"].as_array().and_then(|a| a.first()) else {
            return Ok(None);
        };
        let lock: json::Script =
            serde_json::from_value(c["output"]["lock"].clone()).map_err(|e| e.to_string())?;
        let data = hex::decode(
            c["output_data"]
                .as_str()
                .unwrap_or("0x")
                .trim_start_matches("0x"),
        )
        .map_err(|e| e.to_string())?;
        let cell = LiveCell {
            out_point: out_point(
                c["out_point"]["tx_hash"].as_str().unwrap_or(""),
                hex_u64(&c["out_point"]["index"])? as u32,
            )?,
            capacity: hex_u64(&c["output"]["capacity"])?,
        };
        Ok(Some((cell, data, lock.into())))
    }

    pub fn live_cell(
        &self,
        op: &OutPoint,
        with_data: bool,
    ) -> Result<Option<json::CellWithStatus>, String> {
        let jop = json::OutPoint::from(op.clone());
        let r = self.rpc.call("get_live_cell", json!([jop, with_data]))?;
        let c: json::CellWithStatus = serde_json::from_value(r).map_err(|e| e.to_string())?;
        Ok(if c.status == "live" { Some(c) } else { None })
    }

    /// Cell dep of the secp256k1_blake160 dep group (found in the genesis block).
    pub fn secp_dep(&self) -> Result<CellDep, String> {
        let block = self.rpc.call("get_block_by_number", json!(["0x0"]))?;
        let block: json::BlockView = serde_json::from_value(block).map_err(|e| e.to_string())?;
        for tx in &block.transactions {
            for (i, data) in tx.inner.outputs_data.iter().enumerate() {
                let Ok(vec) = packed::OutPointVec::from_slice(data.as_bytes()) else {
                    continue;
                };
                let mut is_secp = false;
                for member in vec.into_iter() {
                    let Some(info) = self.live_cell(&member, false)?.and_then(|c| c.cell) else {
                        continue;
                    };
                    let ty = info.output.type_.map(packed::Script::from);
                    is_secp |= ty.map(|t| t.calc_script_hash().unpack()) == Some(SIGHASH_CODE_HASH);
                }
                if is_secp {
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
        Err("secp256k1_blake160 dep group not found in the genesis block".into())
    }

    /// Spends the key's plain cells to pay `outs` plus the fee; change goes back to the key.
    pub fn build_funded(
        &self,
        key: &Key,
        mut outs: Vec<Out>,
        mut deps: Vec<CellDep>,
    ) -> Result<TransactionView, String> {
        deps.insert(0, self.secp_dep()?);
        let need: u64 = outs.iter().map(|o| o.capacity).sum();
        let lock = key.lock();
        let change_min = occupied(&lock, &None, 0);
        let cells = self.plain_cells(&lock)?;
        let mut picked = Vec::new();
        let mut have = 0u64;
        // Fee reserve: generous upper bound, the exact fee is taken from the change below.
        let reserve = 2 * CKB;
        for c in cells {
            if have >= need + reserve + change_min {
                break;
            }
            have += c.capacity;
            picked.push(c);
        }
        if have < need + reserve + change_min {
            return Err(format!(
                "insufficient funds: have {have} shannons, need {}",
                need + reserve + change_min
            ));
        }
        outs.push(Out {
            capacity: have - need,
            lock,
            type_: None,
            data: vec![],
            type_id: false,
        });
        let inputs: Vec<CellInput> = picked
            .iter()
            .map(|c| CellInput::new(c.out_point.clone(), 0))
            .collect();
        let build = |outs: &[Out]| -> TransactionView {
            let mut b = TransactionBuilder::default()
                .cell_deps(deps.clone())
                .inputs(inputs.clone());
            for (i, o) in outs.iter().enumerate() {
                let type_ = if o.type_id {
                    let mut hasher = new_blake2b();
                    hasher.update(inputs[0].as_slice());
                    hasher.update(&(i as u64).to_le_bytes());
                    let mut id = [0u8; 32];
                    hasher.finalize(&mut id);
                    Some(type_id_script(&id))
                } else {
                    o.type_.clone()
                };
                let out = CellOutput::new_builder()
                    .capacity(Pack::<packed::Uint64>::pack(&o.capacity))
                    .lock(o.lock.clone())
                    .type_(type_.pack())
                    .build();
                b = b
                    .output(out)
                    .output_data(Bytes::from(o.data.clone()).pack());
            }
            b.build()
        };
        let probe = sign_sighash(&build(&outs), &(0..inputs.len()).collect::<Vec<_>>(), key)?;
        let fee = fee_for(&probe);
        let last = outs.len() - 1;
        outs[last].capacity -= fee;
        sign_sighash(&build(&outs), &(0..inputs.len()).collect::<Vec<_>>(), key)
    }

    pub fn send(&self, tx: &TransactionView) -> Result<String, String> {
        let jtx = json::TransactionView::from(tx.clone()).inner;
        let r = self
            .rpc
            .call("send_transaction", json!([jtx, "passthrough"]))?;
        Ok(r.as_str().unwrap_or("").to_string())
    }

    /// Sends and waits until committed (devnet: mines blocks meanwhile).
    pub fn send_and_wait(&self, tx: &TransactionView) -> Result<String, String> {
        let hash = self.send(tx)?;
        for _ in 0..120 {
            let r = self.rpc.call("get_transaction", json!([hash]))?;
            match r["tx_status"]["status"].as_str() {
                Some("committed") => {
                    self.wait_indexer()?;
                    return Ok(hash);
                }
                Some("rejected") => {
                    return Err(format!("transaction rejected: {}", r["tx_status"]))
                }
                _ => {}
            }
            if self.devnet {
                self.rpc.call("generate_block", json!([]))?;
                std::thread::sleep(std::time::Duration::from_millis(100));
            } else {
                std::thread::sleep(std::time::Duration::from_secs(5));
            }
        }
        Err(format!("transaction {hash} was not committed in time"))
    }
}

/// 2 shannons per byte (the node's minimum is 1) plus a constant.
pub fn fee_for(tx: &TransactionView) -> u64 {
    tx.data().as_slice().len() as u64 * 2 + 1000
}

/// Signs the sighash group `group` (input indexes, all with the same lock) with `key`.
pub fn sign_sighash(
    tx: &TransactionView,
    group: &[usize],
    key: &Key,
) -> Result<TransactionView, String> {
    let n_inputs = tx.inputs().len();
    let mut witnesses: Vec<Bytes> = tx.witnesses().into_iter().map(|w| w.raw_data()).collect();
    while witnesses.len() < n_inputs {
        witnesses.push(Bytes::new());
    }
    let first = *group.first().ok_or("empty signing group")?;
    let args = if witnesses[first].is_empty() {
        WitnessArgs::default()
    } else {
        WitnessArgs::from_slice(&witnesses[first])
            .map_err(|e| format!("bad witness {first}: {e}"))?
    };
    let placeholder = args
        .clone()
        .as_builder()
        .lock(Some(Bytes::from(vec![0u8; 65])).pack())
        .build();
    let mut hasher = new_blake2b();
    hasher.update(tx.hash().as_slice());
    hasher.update(&(placeholder.as_bytes().len() as u64).to_le_bytes());
    hasher.update(&placeholder.as_bytes());
    for &i in &group[1..] {
        hasher.update(&(witnesses[i].len() as u64).to_le_bytes());
        hasher.update(&witnesses[i]);
    }
    for w in witnesses.iter().skip(n_inputs) {
        hasher.update(&(w.len() as u64).to_le_bytes());
        hasher.update(w);
    }
    let mut digest = [0u8; 32];
    hasher.finalize(&mut digest);
    let (rec, compact) = Secp256k1::new()
        .sign_ecdsa_recoverable(&Message::from_digest(digest), &key.sk)
        .serialize_compact();
    let mut sig = compact.to_vec();
    sig.push(rec.to_i32() as u8);
    witnesses[first] = args
        .as_builder()
        .lock(Some(Bytes::from(sig)).pack())
        .build()
        .as_bytes();
    Ok(tx
        .as_advanced_builder()
        .set_witnesses(witnesses.into_iter().map(|w| w.pack()).collect())
        .build())
}

pub fn lock_hash_hex(lock: &Script) -> String {
    h(script_hash(lock).as_slice())
}

pub fn lock_matches(lock: &Script, key: &Key) -> bool {
    let sighash = sighash_lock(&key.args);
    lock.code_hash() == sighash.code_hash()
        && lock.hash_type() == sighash.hash_type()
        && lock.args().raw_data() == sighash.args().raw_data()
}

pub fn data_hash(data: &[u8]) -> [u8; 32] {
    blake2b_256(data)
}

pub fn ckb_to_shannons(s: &str) -> Result<u64, String> {
    let bad = || format!("bad CKB amount '{s}'");
    let (int, frac) = s.split_once('.').unwrap_or((s, ""));
    if frac.len() > 8 || int.is_empty() && frac.is_empty() {
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
