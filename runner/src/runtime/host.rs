//! Host interface: CKB syscalls answered from the virtual transaction plus the CSR service calls
//! (7001–7099, see `spec/host-interface.md`).

use super::vtx::VirtualTx;
use crate::storage::{Put, Store, MAX_KEY_BYTES};
use ckb_vm::registers::{A0, A1, A2, A3, A4, A5, A7};
use ckb_vm::{Error, Memory, Register, SupportMachine, Syscalls};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};

/// Domain prefix of service signatures (`sign`, 7020).
pub const SIGN_DOMAIN: &[u8] = b"CSR-SERVICE-SIGN-v1\n";

pub const MAX_RESULT_BYTES: usize = 65_536;
pub const MAX_DEBUG_LINES: usize = 64;
pub const MAX_DEBUG_LINE_BYTES: usize = 1024;
/// Cycles charged per syscall (CKB uses 500).
const SYSCALL_CYCLES: u64 = 500;
/// Cycles charged for a call that goes to the node (chain reads, cell queries).
#[allow(dead_code)]
pub const NODE_CALL_CYCLES: u64 = 100_000;
/// Longest string the `debug` call (2177) scans for its terminating zero byte.
#[allow(dead_code)]
pub const MAX_DEBUG_SCAN: usize = 1024;

/// Cycles charged for moving `bytes` of data through a host call (one cycle per 4 bytes).
#[allow(dead_code)]
pub fn data_cycles(bytes: u64) -> u64 {
    bytes.div_ceil(4)
}

// Return codes of CKB syscalls.
const SUCCESS: u64 = 0;
const INDEX_OUT_OF_BOUND: u64 = 1;
const ITEM_MISSING: u64 = 2;

// Return codes of service calls.
pub const SVC_OK: u64 = 0;
pub const SVC_MISSING: u64 = 1;
pub const SVC_NOT_SUPPORTED: u64 = 2;
pub const SVC_QUOTA_EXCEEDED: u64 = 10;
pub const SVC_TOO_LARGE: u64 = 11;
pub const SVC_INVALID_ARGUMENT: u64 = 12;

/// Tags carried in `Error::External` to stop a run with a specific status.
pub const STOP_RESULT_TOO_LARGE: &str = "csr:result_too_large";

const SRC_INPUT: u64 = 1;
const SRC_OUTPUT: u64 = 2;
const SRC_CELL_DEP: u64 = 3;
const SRC_HEADER_DEP: u64 = 4;
const SRC_GROUP_INPUT: u64 = 0x0100_0000_0000_0001;
const SRC_GROUP_OUTPUT: u64 = 0x0100_0000_0000_0002;

/// What a run collects; shared between the VM and the caller.
pub struct HostState {
    pub vtx: VirtualTx,
    pub block_info: [u64; 3],
    pub debug: Vec<String>,
    pub result: Option<Vec<u8>>,
    pub skeleton: Option<Vec<u8>>,
    /// Encrypted storage of the service; `None` makes the storage calls return 2.
    pub store: Option<Store>,
    /// Service key and code hash for `sign`; `None` makes it return 2.
    pub signer: Option<(secp256k1::SecretKey, [u8; 32])>,
    /// Node access for `chain_read` / `cells_query`; `None` (local runs) makes them return 2.
    pub chain: Option<Box<dyn ChainAccess>>,
}

/// Maximum number of out points `cells_query` returns.
pub const MAX_QUERY_RESULTS: usize = 64;

/// Blocking node access for the chain service calls (`spec/host-interface.md`).
pub trait ChainAccess: Send {
    /// `kind` 0 = live cell output (molecule `CellOutput`), 1 = live cell data (key = 36-byte out
    /// point), 2 = header (molecule `Header`), 3 = transaction (molecule `Transaction`) (key =
    /// 32-byte hash). `Ok(None)` = not found / not live.
    fn read(&self, kind: u64, key: &[u8]) -> Result<Option<Vec<u8>>, String>;
    /// Live cells whose lock (`script_type` 0) or type (1) matches `script` (molecule `Script`,
    /// args as prefix): 36-byte out points, at most `MAX_QUERY_RESULTS`.
    fn query(&self, script: &[u8], script_type: u64) -> Result<Option<Vec<[u8; 36]>>, String>;
}

impl HostState {
    pub fn new(vtx: VirtualTx, block_info: [u64; 3]) -> Self {
        HostState {
            vtx,
            block_info,
            debug: Vec::new(),
            result: None,
            skeleton: None,
            store: None,
            signer: None,
            chain: None,
        }
    }

    pub fn with_store(mut self, store: Store) -> Self {
        self.store = Some(store);
        self
    }

    pub fn with_chain(mut self, chain: Box<dyn ChainAccess>) -> Self {
        self.chain = Some(chain);
        self
    }

    pub fn with_signer(mut self, key: secp256k1::SecretKey, code_hash: [u8; 32]) -> Self {
        self.signer = Some((key, code_hash));
        self
    }

    /// The result: `result_put` bytes, else the last `CSR-RESULT:0x<hex>` debug line, else empty.
    pub fn final_result(&self) -> Vec<u8> {
        if let Some(r) = &self.result {
            return r.clone();
        }
        self.debug
            .iter()
            .rev()
            .find_map(|l| l.trim_end().strip_prefix("CSR-RESULT:0x"))
            .and_then(|h| hex::decode(h).ok())
            .unwrap_or_default()
    }
}

pub struct Host {
    pub state: Arc<Mutex<HostState>>,
}

/// Writes `data[offset..]` to `addr` following CKB's partial-loading convention: `*len_ptr` holds
/// the buffer size on entry and the full data length on return.
fn write_partial<Mac: SupportMachine>(
    m: &mut Mac,
    data: &[u8],
    addr: u64,
    len_ptr: u64,
    offset: u64,
) -> Result<u64, Error> {
    let cap = m
        .memory_mut()
        .load64(&Mac::REG::from_u64(len_ptr))?
        .to_u64();
    let full = data.len() as u64;
    let start = offset.min(full) as usize;
    let n = (cap as usize).min(data.len() - start);
    if n > 0 {
        m.memory_mut().store_bytes(addr, &data[start..start + n])?;
    }
    m.memory_mut()
        .store64(&Mac::REG::from_u64(len_ptr), &Mac::REG::from_u64(full))?;
    Ok(SUCCESS)
}

fn storage_error(e: String) -> Error {
    Error::External(format!("csr:storage: {e}"))
}

/// Reads a storage key; `None` when its length is 0 or above the limit.
fn read_key<Mac: SupportMachine>(
    m: &mut Mac,
    addr: u64,
    len: u64,
) -> Result<Option<Vec<u8>>, Error> {
    if len == 0 || len as usize > MAX_KEY_BYTES {
        return Ok(None);
    }
    read_mem(m, addr, len).map(Some)
}

fn read_mem<Mac: SupportMachine>(m: &mut Mac, addr: u64, len: u64) -> Result<Vec<u8>, Error> {
    if len == 0 {
        return Ok(Vec::new());
    }
    Ok(m.memory_mut().load_bytes(addr, len)?.to_vec())
}

impl<Mac: SupportMachine> Syscalls<Mac> for Host {
    fn initialize(&mut self, _machine: &mut Mac) -> Result<(), Error> {
        Ok(())
    }

    fn ecall(&mut self, m: &mut Mac) -> Result<bool, Error> {
        let n = m.registers()[A7].to_u64();
        let arg = |m: &Mac, r: usize| m.registers()[r].to_u64();
        let (a0, a1, a2, a3, a4, a5) = (
            arg(m, A0),
            arg(m, A1),
            arg(m, A2),
            arg(m, A3),
            arg(m, A4),
            arg(m, A5),
        );
        let known = matches!(
            n,
            2041 | 2042
                | 2052
                | 2061
                | 2062
                | 2071..=2074
                | 2081..=2083
                | 2091
                | 2092
                | 2104
                | 2177
                | 7001..=7099
        );
        if !known {
            return Ok(false);
        }
        m.add_cycles_no_checking(SYSCALL_CYCLES)?;
        let ret = {
            let state = self.state.clone();
            let mut st = state.lock().unwrap();
            self.dispatch(m, &mut st, n, [a0, a1, a2, a3, a4, a5])?
        };
        m.set_register(A0, Mac::REG::from_u64(ret));
        Ok(true)
    }
}

impl Host {
    fn dispatch<Mac: SupportMachine>(
        &mut self,
        m: &mut Mac,
        st: &mut HostState,
        n: u64,
        [a0, a1, a2, a3, a4, a5]: [u64; 6],
    ) -> Result<u64, Error> {
        Ok(match n {
            2041 => 2, // vm_version
            2042 => m.cycles(),
            2052 => {
                let d = st.vtx.script().to_molecule();
                write_partial(m, &d, a0, a1, a2)?
            }
            2061 => {
                let d = st.vtx.tx_hash;
                write_partial(m, &d, a0, a1, a2)?
            }
            2062 => {
                let d = st.vtx.script().hash();
                write_partial(m, &d, a0, a1, a2)?
            }
            // load_cell(addr, len, offset, index, source)
            2071 => match cell_at(&st.vtx, a3, a4) {
                Lookup::Cell(c) => write_partial(m, &c.output_molecule(), a0, a1, a2)?,
                Lookup::Missing(code) => code,
            },
            // load_input(addr, len, offset, index, source)
            2073 => match input_at(&st.vtx, a3, a4) {
                Lookup::Cell(_) => {
                    let mut d = Vec::with_capacity(44);
                    d.extend_from_slice(&0u64.to_le_bytes()); // since
                    d.extend_from_slice(&out_point(&st.vtx));
                    write_partial(m, &d, a0, a1, a2)?
                }
                Lookup::Missing(code) => code,
            },
            // load_witness(addr, len, offset, index, source)
            2074 => match witness_at(&st.vtx, a3, a4) {
                Ok(i) => {
                    let d = st.vtx.witnesses[i].clone();
                    write_partial(m, &d, a0, a1, a2)?
                }
                Err(code) => code,
            },
            // load_cell_by_field(addr, len, offset, index, source, field)
            2081 => match cell_at(&st.vtx, a3, a4) {
                Lookup::Cell(c) => {
                    let d: Option<Vec<u8>> = match a5 {
                        0 => Some(c.capacity.to_le_bytes().to_vec()),
                        1 => Some(c.data_hash().to_vec()),
                        2 => Some(c.lock.to_molecule()),
                        3 => Some(c.lock.hash().to_vec()),
                        4 => c.type_.as_ref().map(|s| s.to_molecule()),
                        5 => c.type_.as_ref().map(|s| s.hash().to_vec()),
                        6 => Some(c.occupied_capacity().to_le_bytes().to_vec()),
                        _ => return Ok(ITEM_MISSING),
                    };
                    match d {
                        Some(d) => write_partial(m, &d, a0, a1, a2)?,
                        None => ITEM_MISSING,
                    }
                }
                Lookup::Missing(code) => code,
            },
            // load_header / load_header_by_field: the virtual transaction has no header deps.
            2072 | 2082 => INDEX_OUT_OF_BOUND,
            // load_input_by_field(addr, len, offset, index, source, field)
            2083 => match input_at(&st.vtx, a3, a4) {
                Lookup::Cell(_) => match a5 {
                    0 => write_partial(m, &out_point(&st.vtx), a0, a1, a2)?,
                    1 => write_partial(m, &0u64.to_le_bytes(), a0, a1, a2)?,
                    _ => ITEM_MISSING,
                },
                Lookup::Missing(code) => code,
            },
            // load_cell_data_as_code(addr, memory_size, content_offset, content_size, index, source)
            2091 => match cell_at(&st.vtx, a4, a5) {
                Lookup::Cell(c) => {
                    let data = &c.data;
                    let (off, size) = (a2 as usize, a3 as usize);
                    if off > data.len() || size > data.len() - off || size as u64 > a1 {
                        return Ok(SVC_INVALID_ARGUMENT);
                    }
                    m.memory_mut().store_bytes(a0, &data[off..off + size])?;
                    SUCCESS
                }
                Lookup::Missing(code) => code,
            },
            // load_cell_data(addr, len, offset, index, source)
            2092 => match cell_at(&st.vtx, a3, a4) {
                Lookup::Cell(c) => write_partial(m, &c.data, a0, a1, a2)?,
                Lookup::Missing(code) => code,
            },
            2104 => INDEX_OUT_OF_BOUND, // load_block_extension: no headers
            2177 => {
                let mut bytes = Vec::new();
                let mut p = a0;
                loop {
                    let b = m.memory_mut().load8(&Mac::REG::from_u64(p))?.to_u8();
                    if b == 0 {
                        break;
                    }
                    if bytes.len() < MAX_DEBUG_LINE_BYTES {
                        bytes.push(b);
                    }
                    p += 1;
                }
                if st.debug.len() < MAX_DEBUG_LINES {
                    st.debug.push(String::from_utf8_lossy(&bytes).into_owned());
                }
                return Ok(a0); // debug has no return value; keep a0 unchanged
            }
            // job_payload(addr, len_ptr, offset)
            7001 => {
                let d = st.vtx.witnesses[0].clone();
                write_partial(m, &d, a0, a1, a2)?;
                SVC_OK
            }
            // block_info(addr): tip number, packed epoch, median time (ms), u64 LE each
            7002 => {
                let mut d = Vec::with_capacity(24);
                for v in st.block_info {
                    d.extend_from_slice(&v.to_le_bytes());
                }
                m.memory_mut().store_bytes(a0, &d)?;
                SVC_OK
            }
            // tx_skeleton_put(addr, len)
            7030 => {
                let d = read_mem(m, a0, a1)?;
                st.skeleton = Some(d);
                SVC_OK
            }
            // result_put(addr, len)
            7031 => {
                if a1 as usize > MAX_RESULT_BYTES {
                    return Err(Error::External(STOP_RESULT_TOO_LARGE.into()));
                }
                st.result = Some(read_mem(m, a0, a1)?);
                SVC_OK
            }
            // kv_get(key, key_len, addr, len_ptr, offset)
            7010 => {
                let Some(store) = &st.store else {
                    return Ok(SVC_NOT_SUPPORTED);
                };
                let Some(key) = read_key(m, a0, a1)? else {
                    return Ok(SVC_INVALID_ARGUMENT);
                };
                match store.get(&key).map_err(storage_error)? {
                    Some(v) => {
                        write_partial(m, &v, a2, a3, a4)?;
                        SVC_OK
                    }
                    None => SVC_MISSING,
                }
            }
            // kv_put(key, key_len, val, val_len)
            7011 => {
                let Some(store) = &st.store else {
                    return Ok(SVC_NOT_SUPPORTED);
                };
                if a1 as usize > MAX_KEY_BYTES || a3 as usize > crate::storage::MAX_VALUE_BYTES {
                    return Ok(SVC_TOO_LARGE);
                }
                let key = read_mem(m, a0, a1)?;
                let val = read_mem(m, a2, a3)?;
                match store.put(&key, &val).map_err(storage_error)? {
                    Put::Ok => SVC_OK,
                    Put::QuotaExceeded => SVC_QUOTA_EXCEEDED,
                    Put::TooLarge => SVC_TOO_LARGE,
                    Put::InvalidArgument => SVC_INVALID_ARGUMENT,
                }
            }
            // kv_delete(key, key_len)
            7012 | 7013 => {
                let Some(store) = &st.store else {
                    return Ok(SVC_NOT_SUPPORTED);
                };
                let Some(key) = read_key(m, a0, a1)? else {
                    return Ok(SVC_INVALID_ARGUMENT);
                };
                let found = if n == 7012 {
                    store.delete(&key)
                } else {
                    store.mark_secret(&key)
                };
                if found.map_err(storage_error)? {
                    SVC_OK
                } else {
                    SVC_MISSING
                }
            }
            // delete_on_consume(key, key_len, outpoint)
            7014 => {
                let Some(store) = &st.store else {
                    return Ok(SVC_NOT_SUPPORTED);
                };
                let Some(key) = read_key(m, a0, a1)? else {
                    return Ok(SVC_INVALID_ARGUMENT);
                };
                let op: [u8; 36] = read_mem(m, a2, 36)?.try_into().expect("36 bytes");
                match store.delete_on_consume(&key, &op).map_err(storage_error)? {
                    Put::Ok => SVC_OK,
                    _ => SVC_QUOTA_EXCEEDED,
                }
            }
            // sign(msg32, sig_out64)
            7020 => {
                let Some((key, code_hash)) = &st.signer else {
                    return Ok(SVC_NOT_SUPPORTED);
                };
                let msg = read_mem(m, a0, 32)?;
                let mut h = Sha256::new();
                h.update(SIGN_DOMAIN);
                h.update(code_hash);
                h.update(&msg);
                let digest = secp256k1::Message::from_digest(h.finalize().into());
                let sig = secp256k1::Secp256k1::signing_only().sign_ecdsa(&digest, key);
                m.memory_mut().store_bytes(a1, &sig.serialize_compact())?;
                SVC_OK
            }
            // chain_read(kind, key, addr, len_ptr, offset)
            7003 => {
                let Some(chain) = &st.chain else {
                    return Ok(SVC_NOT_SUPPORTED);
                };
                let key_len = match a0 {
                    0 | 1 => 36,
                    2 | 3 => 32,
                    _ => return Ok(SVC_INVALID_ARGUMENT),
                };
                let key = read_mem(m, a1, key_len)?;
                match chain.read(a0, &key).map_err(chain_error)? {
                    Some(d) => {
                        write_partial(m, &d, a2, a3, a4)?;
                        SVC_OK
                    }
                    None => SVC_MISSING,
                }
            }
            // cells_query(script, script_len, script_type, addr, len_ptr, offset)
            7004 => {
                let Some(chain) = &st.chain else {
                    return Ok(SVC_NOT_SUPPORTED);
                };
                if a1 == 0 || a1 > 4096 || a2 > 1 {
                    return Ok(SVC_INVALID_ARGUMENT);
                }
                let script = read_mem(m, a0, a1)?;
                match chain.query(&script, a2).map_err(chain_error)? {
                    Some(ops) => {
                        write_partial(m, &out_point_vec(&ops), a3, a4, a5)?;
                        SVC_OK
                    }
                    None => SVC_INVALID_ARGUMENT,
                }
            }
            // CRT calls and unassigned numbers: not available.
            _ => SVC_NOT_SUPPORTED,
        })
    }
}

fn chain_error(e: String) -> Error {
    Error::External(format!("csr:chain: {e}"))
}

/// Molecule `OutPointVec` (fixvec: item count u32 LE, then 36-byte items).
pub fn out_point_vec(ops: &[[u8; 36]]) -> Vec<u8> {
    let mut d = (ops.len() as u32).to_le_bytes().to_vec();
    for op in ops {
        d.extend_from_slice(op);
    }
    d
}

fn out_point(vtx: &VirtualTx) -> Vec<u8> {
    let mut d = vtx.input_out_point.0.to_vec();
    d.extend_from_slice(&vtx.input_out_point.1.to_le_bytes());
    d
}

enum Lookup<'a> {
    Cell(&'a super::vtx::Cell),
    Missing(u64),
}

fn input_at(vtx: &VirtualTx, index: u64, source: u64) -> Lookup<'_> {
    match source {
        SRC_INPUT | SRC_GROUP_INPUT if index == 0 => Lookup::Cell(&vtx.input),
        SRC_INPUT | SRC_GROUP_INPUT => Lookup::Missing(INDEX_OUT_OF_BOUND),
        _ => Lookup::Missing(INDEX_OUT_OF_BOUND),
    }
}

fn cell_at(vtx: &VirtualTx, index: u64, source: u64) -> Lookup<'_> {
    match source {
        SRC_INPUT | SRC_GROUP_INPUT => input_at(vtx, index, source),
        SRC_CELL_DEP => match vtx.cell_deps.get(index as usize) {
            Some(c) => Lookup::Cell(c),
            None => Lookup::Missing(INDEX_OUT_OF_BOUND),
        },
        SRC_OUTPUT | SRC_GROUP_OUTPUT | SRC_HEADER_DEP => Lookup::Missing(INDEX_OUT_OF_BOUND),
        _ => Lookup::Missing(ITEM_MISSING),
    }
}

/// Witness index for `source`; the group contains only input 0 so group index 0 maps to witness 0.
fn witness_at(vtx: &VirtualTx, index: u64, source: u64) -> Result<usize, u64> {
    match source {
        SRC_INPUT | SRC_GROUP_INPUT if (index as usize) < vtx.witnesses.len() => Ok(index as usize),
        SRC_OUTPUT | SRC_GROUP_OUTPUT if (index as usize) < vtx.witnesses.len() => {
            Ok(index as usize)
        }
        SRC_INPUT | SRC_GROUP_INPUT | SRC_OUTPUT | SRC_GROUP_OUTPUT => Err(INDEX_OUT_OF_BOUND),
        _ => Err(ITEM_MISSING),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn out_point_vec_is_a_molecule_fixvec() {
        let v = out_point_vec(&[[1; 36], [2; 36]]);
        assert_eq!(v.len(), 4 + 72);
        assert_eq!(&v[..4], &2u32.to_le_bytes());
        assert_eq!(v[4 + 36], 2);
    }
}
