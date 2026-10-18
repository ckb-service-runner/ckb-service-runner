//! `csr-registry` type script, args = `config_type_hash (32)`. See `spec/registry.md`.
#![no_std]
#![no_main]


mod mol;

use alloc::vec::Vec;
use ckb_std::ckb_constants::{CellField, Source};
use ckb_std::error::SysError;
use ckb_std::syscalls;
use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};

ckb_std::entry!(program_entry);
ckb_std::default_alloc!();

include!(concat!(env!("OUT_DIR"), "/dual_lock_code_hash.rs"));

const ERR_ARGS_LEN: i8 = 5;
const ERR_GROUP_COUNT: i8 = 6;
const ERR_DATA: i8 = 7;
const ERR_CONFIG_DEP: i8 = 8;
const ERR_CONFIG_DATA: i8 = 9;
const ERR_SIGNATURE: i8 = 10;
const ERR_ATTEST_MISMATCH: i8 = 11;
const ERR_EXPIRED: i8 = 12;
const ERR_LOCK: i8 = 13;
const ERR_RUNNER_FIELD: i8 = 14;
const ERR_RUNNER_CAPACITY: i8 = 15;
const ERR_DESTROY: i8 = 16;

/// Domain separator of the attestation message.
const ATTEST_DOMAIN: &[u8] = b"CSR-ATTEST-v1";

/// `secp256k1_blake160_sighash_all` as deployed in the genesis block of every CKB chain (hash type `type`).
const SECP_TYPE_HASH: [u8; 32] = hex32("9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8");
/// Data hash of the same binary (ckb-system-scripts 0.5.4), referenced with hash type data/data1/data2.
const SECP_DATA_HASH: [u8; 32] = hex32("709f3fda12f561cfacf92273c57a98fede188a3f1a59b1f888d113f9cce08649");

const HASH_TYPE_DATA: u8 = 0;
const HASH_TYPE_TYPE: u8 = 1;
const HASH_TYPE_DATA1: u8 = 2;
const HASH_TYPE_DATA2: u8 = 4;

const fn hex32(s: &str) -> [u8; 32] {
    const fn nib(c: u8) -> u8 {
        match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            _ => panic!("hex"),
        }
    }
    let b = s.as_bytes();
    let mut out = [0u8; 32];
    let mut i = 0;
    while i < 32 {
        out[i] = nib(b[2 * i]) << 4 | nib(b[2 * i + 1]);
        i += 1;
    }
    out
}

fn program_entry() -> i8 {
    match run() {
        Ok(()) => 0,
        Err(code) => code,
    }
}

fn sys(e: SysError) -> i8 {
    match e {
        SysError::IndexOutOfBound => 1,
        SysError::ItemMissing => 2,
        SysError::LengthNotEnough(_) => 3,
        _ => 4,
    }
}

fn blake2b_256(parts: &[&[u8]]) -> [u8; 32] {
    let mut h = blake2b_ref::Blake2bBuilder::new(32).personal(b"ckb-default-hash").build();
    for p in parts {
        h.update(p);
    }
    let mut out = [0u8; 32];
    h.finalize(&mut out);
    out
}

/// Loads a field of unknown length completely (the first call learns the size).
fn load_all(f: impl Fn(&mut [u8], usize) -> Result<usize, SysError>) -> Result<Vec<u8>, i8> {
    let mut buf = [0u8; 256];
    match f(&mut buf, 0) {
        Ok(n) => Ok(buf[..n].to_vec()),
        Err(SysError::LengthNotEnough(total)) => {
            let mut v = alloc::vec![0u8; total];
            let n = f(&mut v, 0).map_err(sys)?;
            v.truncate(n);
            Ok(v)
        }
        Err(e) => Err(sys(e)),
    }
}

fn cell_data(i: usize, src: Source) -> Result<Vec<u8>, i8> {
    load_all(|b, o| syscalls::load_cell_data(b, o, i, src))
}

fn cell_lock(i: usize, src: Source) -> Result<Vec<u8>, i8> {
    load_all(|b, o| syscalls::load_cell_by_field(b, o, i, src, CellField::Lock))
}

fn load_cell_capacity(i: usize, src: Source) -> Result<u64, SysError> {
    let mut b = [0u8; 8];
    syscalls::load_cell_by_field(&mut b, 0, i, src, CellField::Capacity)?;
    Ok(u64::from_le_bytes(b))
}

/// `Some(hash)` of field `f` (LockHash/TypeHash) of each cell of `src` until the end; `None` for a missing type.
fn for_each_hash(src: Source, f: CellField, mut cb: impl FnMut(usize, Option<&[u8; 32]>) -> Result<bool, i8>) -> Result<(), i8> {
    let mut hash = [0u8; 32];
    for i in 0.. {
        let r = match syscalls::load_cell_by_field(&mut hash, 0, i, src, f) {
            Ok(_) => cb(i, Some(&hash))?,
            Err(SysError::ItemMissing) => cb(i, None)?,
            Err(SysError::IndexOutOfBound) => break,
            Err(e) => return Err(sys(e)),
        };
        if !r {
            break;
        }
    }
    Ok(())
}

/// Whether an input with this lock hash exists.
fn input_with_lock_hash(want: &[u8]) -> Result<bool, i8> {
    let mut found = false;
    for_each_hash(Source::Input, CellField::LockHash, |_, h| {
        found = h.map_or(false, |h| h[..] == *want);
        Ok(!found)
    })?;
    Ok(found)
}

/// Number of cells in a group source, or an error if there are more than one.
fn group_count(src: Source) -> Result<usize, i8> {
    match load_cell_capacity(1, src) {
        Ok(_) => return Err(ERR_GROUP_COUNT),
        Err(SysError::IndexOutOfBound) => {}
        Err(e) => return Err(sys(e)),
    }
    match load_cell_capacity(0, src) {
        Ok(_) => Ok(1),
        Err(SysError::IndexOutOfBound) => Ok(0),
        Err(e) => Err(sys(e)),
    }
}

/// Parsed `Registry` data (field slices are raw molecule encodings).
struct Registry<'a> {
    peer_id: &'a [u8],
    service_pubkey: &'a [u8],
    float_lock_hash: &'a [u8],
    network: &'a [u8],
    att_peer_id: &'a [u8],
    att_service_pubkey: &'a [u8],
    att_network: &'a [u8],
    att_expiry: &'a [u8],
    att_signature: &'a [u8; 65],
}

fn parse_registry(d: &[u8]) -> Option<Registry<'_>> {
    let [peer_id, service_pubkey, float_lock_hash, network, attestation, prekeys] = mol::table::<6>(d)?;
    mol::bytes(peer_id)?;
    mol::fixed::<33>(service_pubkey)?;
    mol::fixed::<32>(float_lock_hash)?;
    if mol::fixed::<1>(network)?[0] > 2 {
        return None;
    }
    mol::bytes(prekeys)?;
    let [att_peer_id, att_service_pubkey, att_network, att_expiry, att_signature] = mol::table::<5>(attestation)?;
    mol::bytes(att_peer_id)?;
    mol::fixed::<33>(att_service_pubkey)?;
    mol::fixed::<1>(att_network)?;
    mol::fixed::<8>(att_expiry)?;
    Some(Registry {
        peer_id,
        service_pubkey,
        float_lock_hash,
        network,
        att_peer_id,
        att_service_pubkey,
        att_network,
        att_expiry,
        att_signature: mol::fixed::<65>(att_signature)?,
    })
}

/// Message signed by the monitor: blake2b-256("CSR-ATTEST-v1" ‖ molecule(AttestationBody)).
fn attestation_message(r: &Registry) -> [u8; 32] {
    let fields = [r.att_peer_id, r.att_service_pubkey, r.att_network, r.att_expiry];
    let mut header = [0u8; 20];
    let mut off = header.len();
    for (i, f) in fields.iter().enumerate() {
        header[4 + 4 * i..8 + 4 * i].copy_from_slice(&(off as u32).to_le_bytes());
        off += f.len();
    }
    header[0..4].copy_from_slice(&(off as u32).to_le_bytes());
    blake2b_256(&[ATTEST_DOMAIN, &header, fields[0], fields[1], fields[2], fields[3]])
}

/// Recovers the compressed public key from a 65-byte `r ‖ s ‖ recovery_id` signature (low-S required).
fn recover(msg: &[u8; 32], sig: &[u8; 65]) -> Option<[u8; 33]> {
    let recid = RecoveryId::from_byte(sig[64])?;
    let signature = Signature::from_slice(&sig[..64]).ok()?;
    let vk = VerifyingKey::recover_from_prehash(msg, &signature, recid).ok()?;
    vk.to_encoded_point(true).as_bytes().try_into().ok()
}

/// Lock hash of a system lock script referencing the sighash code with `args = blake160`.
fn secp_lock_hash(code_hash: &[u8; 32], hash_type: u8, blake160: &[u8]) -> [u8; 32] {
    // Script table: total 73, offsets 16/48/49, code_hash, hash_type, args (Bytes of 20).
    let mut s = [0u8; 73];
    s[0..4].copy_from_slice(&73u32.to_le_bytes());
    s[4..8].copy_from_slice(&16u32.to_le_bytes());
    s[8..12].copy_from_slice(&48u32.to_le_bytes());
    s[12..16].copy_from_slice(&49u32.to_le_bytes());
    s[16..48].copy_from_slice(code_hash);
    s[48] = hash_type;
    s[49..53].copy_from_slice(&20u32.to_le_bytes());
    s[53..73].copy_from_slice(blake160);
    blake2b_256(&[&s])
}

/// Rule 5: the lock is `csr-dual-lock` and its runner hash is the system lock of `service_pubkey`.
fn check_lock(lock: &[u8], service_pubkey: &[u8]) -> Result<(), i8> {
    let (code_hash, hash_type, args) = mol::script(lock).ok_or(ERR_LOCK)?;
    if *code_hash != DUAL_LOCK_CODE_HASH || !(hash_type == HASH_TYPE_DATA1 || hash_type == HASH_TYPE_DATA2) {
        return Err(ERR_LOCK);
    }
    if args.len() != 64 {
        return Err(ERR_LOCK);
    }
    let runner = &args[32..64];
    let b160 = &blake2b_256(&[service_pubkey])[..20];
    let candidates = [
        (&SECP_TYPE_HASH, HASH_TYPE_TYPE),
        (&SECP_DATA_HASH, HASH_TYPE_DATA),
        (&SECP_DATA_HASH, HASH_TYPE_DATA1),
        (&SECP_DATA_HASH, HASH_TYPE_DATA2),
    ];
    if candidates.iter().any(|(c, t)| secp_lock_hash(c, *t, b160)[..] == *runner) {
        Ok(())
    } else {
        Err(ERR_LOCK)
    }
}

/// Rules 1–5 for a registry output.
fn check_output(config_type_hash: &[u8], r: &Registry, lock: &[u8]) -> Result<(), i8> {
    // 1. exactly one config cell dep
    let mut config = None;
    for_each_hash(Source::CellDep, CellField::TypeHash, |i, h| {
        if h.map_or(false, |h| h[..] == *config_type_hash) {
            if config.is_some() {
                return Err(ERR_CONFIG_DEP);
            }
            config = Some(i);
        }
        Ok(true)
    })?;
    let config = cell_data(config.ok_or(ERR_CONFIG_DEP)?, Source::CellDep)?;
    if config.len() < 41 {
        return Err(ERR_CONFIG_DATA);
    }
    let monitor_pubkey = &config[0..33];
    let clock_ms = u64::from_le_bytes(config[33..41].try_into().unwrap());

    // 2. signed by the monitor
    let msg = attestation_message(r);
    match recover(&msg, r.att_signature) {
        Some(pk) if pk[..] == *monitor_pubkey => {}
        _ => return Err(ERR_SIGNATURE),
    }
    // 3. attestation is about this registry entry
    if r.att_peer_id != r.peer_id || r.att_service_pubkey != r.service_pubkey || r.att_network != r.network {
        return Err(ERR_ATTEST_MISMATCH);
    }
    // 4. not expired
    let expiry = u64::from_le_bytes(r.att_expiry.try_into().unwrap());
    if expiry <= clock_ms {
        return Err(ERR_EXPIRED);
    }
    // 5. lock
    check_lock(lock, r.service_pubkey)
}

/// Owner hash from a registry cell's dual-lock args, and whether an input with it is present.
fn owner_present(lock: &[u8]) -> Result<bool, i8> {
    let (_, _, args) = mol::script(lock).ok_or(ERR_LOCK)?;
    let owner = args.get(0..32).ok_or(ERR_LOCK)?;
    input_with_lock_hash(owner)
}

fn run() -> Result<(), i8> {
    let script = load_all(|b, o| syscalls::load_script(b, o))?;
    let (_, _, config_type_hash) = mol::script(&script).ok_or(ERR_ARGS_LEN)?;
    if config_type_hash.len() != 32 {
        return Err(ERR_ARGS_LEN);
    }
    let n_in = group_count(Source::GroupInput)?;
    let n_out = group_count(Source::GroupOutput)?;

    if n_out == 0 {
        // destroy (n_in == 1; a group always has at least one cell)
        let lock = cell_lock(0, Source::GroupInput)?;
        return if owner_present(&lock)? { Ok(()) } else { Err(ERR_DESTROY) };
    }

    let out_data = cell_data(0, Source::GroupOutput)?;
    let out = parse_registry(&out_data).ok_or(ERR_DATA)?;
    let out_lock = cell_lock(0, Source::GroupOutput)?;

    if n_in == 1 {
        let in_lock = cell_lock(0, Source::GroupInput)?;
        if !owner_present(&in_lock)? {
            // runner path: only attestation and prekeys may change
            let in_data = cell_data(0, Source::GroupInput)?;
            let inp = parse_registry(&in_data).ok_or(ERR_DATA)?;
            if inp.peer_id != out.peer_id
                || inp.service_pubkey != out.service_pubkey
                || inp.float_lock_hash != out.float_lock_hash
                || inp.network != out.network
                || in_lock != out_lock
            {
                return Err(ERR_RUNNER_FIELD);
            }
            let in_cap = load_cell_capacity(0, Source::GroupInput).map_err(sys)?;
            let out_cap = load_cell_capacity(0, Source::GroupOutput).map_err(sys)?;
            if out_cap < in_cap {
                return Err(ERR_RUNNER_CAPACITY);
            }
        }
    }
    check_output(config_type_hash, &out, &out_lock)
}
