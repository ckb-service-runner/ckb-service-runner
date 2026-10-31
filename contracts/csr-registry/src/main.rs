//! `csr-registry` type script, args = `config_id (32)`: the args of the monitor config cell's `csr-config-type`
//! script. Data: `RegistryV3`. See `spec/registry.md`.
#![no_std]
#![no_main]

#[allow(dead_code)]
mod attest;
mod consent;
mod mol;

use alloc::vec::Vec;
use ckb_std::ckb_constants::{CellField, Source};
use ckb_std::error::SysError;
use ckb_std::syscalls;
use attest::{AttestError, RegistryV3, HASH_TYPE_DATA1, HASH_TYPE_DATA2};
use k256::ecdsa::Signature;

ckb_std::entry!(program_entry);
ckb_std::default_alloc!();

include!(concat!(env!("OUT_DIR"), "/code_hashes.rs"));

const ERR_ARGS_LEN: i8 = 5;
const ERR_GROUP_COUNT: i8 = 6;
const ERR_DATA: i8 = 7;
const ERR_CONFIG_DEP: i8 = 8;
const ERR_CONFIG_DATA: i8 = 9;
const ERR_SIGNATURE: i8 = 10;
// 11 (attestation copy does not match the entry) is unused since v3: the monitor signs the cell's own fields.
const ERR_EXPIRED: i8 = 12;
const ERR_LOCK: i8 = 13;
const ERR_RUNNER_FIELD: i8 = 14;
const ERR_RUNNER_CAPACITY: i8 = 15;
const ERR_DESTROY: i8 = 16;
const ERR_NO_RUNNER_KEY: i8 = 17;
const ERR_STAKE_DEP: i8 = 18;
const ERR_STAKE_NOT_DEPOSIT: i8 = 19;
const ERR_STAKE_CAPACITY: i8 = 20;
/// Stake rule 4: the stake owner is no input and the registry output's witness has no `output_type`.
const ERR_STAKE_NO_CONSENT: i8 = 21;
/// Stake rule 4: the consent signature is malformed (length, recovery byte, r/s).
const ERR_CONSENT_SIGNATURE: i8 = 22;
/// Stake rule 4: the consent signature was not made by the stake owner over this registration's text.
const ERR_CONSENT_MISMATCH: i8 = 23;
/// `expiry_ms − clock_ms` is over 8 days (decision D24 A).
const ERR_LIFETIME_CAP: i8 = 24;

/// Minimum capacity of the stake deposit: 10,000 CKB.
const MIN_STAKE: u64 = 1_000_000_000_000;
/// Type script code hash of the Nervos DAO (hash type `type`, empty args; the same on every chain).
const DAO_TYPE_CODE_HASH: [u8; 32] = hex32("82d76d1b75fe2fd9a27dfbaa65a039221a380d76c926f378d3f81cf3e7e13f2e");
/// Size of a molecule `CellDep`: `OutPoint (36) ‖ dep_type (1)`.
const CELL_DEP_SIZE: usize = 37;
const DEP_TYPE_CODE: u8 = 0;

/// `secp256k1_blake160_sighash_all` as deployed in the genesis block of every CKB chain (hash type `type`).
const SECP_TYPE_HASH: [u8; 32] = hex32("9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8");
/// Data hash of the same binary (ckb-system-scripts 0.5.4), referenced with hash type data/data1/data2.
const SECP_DATA_HASH: [u8; 32] = hex32("709f3fda12f561cfacf92273c57a98fede188a3f1a59b1f888d113f9cce08649");

const HASH_TYPE_DATA: u8 = 0;
const HASH_TYPE_TYPE: u8 = 1;

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

/// Whether `lock_hash` is one of the four accepted `secp256k1_blake160_sighash_all` locks with `args = blake160`.
fn is_sighash_lock_of(lock_hash: &[u8], blake160: &[u8]) -> bool {
    let candidates = [
        (&SECP_TYPE_HASH, HASH_TYPE_TYPE),
        (&SECP_DATA_HASH, HASH_TYPE_DATA),
        (&SECP_DATA_HASH, HASH_TYPE_DATA1),
        (&SECP_DATA_HASH, HASH_TYPE_DATA2),
    ];
    candidates.iter().any(|(c, t)| secp_lock_hash(c, *t, blake160)[..] == *lock_hash)
}

/// Rule 5: the lock is `csr-dual-lock` (args 32 or 64 bytes) and its runner hash is the system lock of
/// `service_pubkey`.
fn check_lock(lock: &[u8], service_pubkey: &[u8]) -> Result<(), i8> {
    let (code_hash, hash_type, _) = mol::script(lock).ok_or(ERR_LOCK)?;
    if *code_hash != DUAL_LOCK_CODE_HASH || !(hash_type == HASH_TYPE_DATA1 || hash_type == HASH_TYPE_DATA2) {
        return Err(ERR_LOCK);
    }
    let b160 = &blake2b_256(&[service_pubkey])[..20];
    if is_sighash_lock_of(runner_hash(lock)?, b160) {
        Ok(())
    } else {
        Err(ERR_LOCK)
    }
}

/// Rules 1–5 for a registry output.
fn check_output(config_id: &[u8; 32], r: &RegistryV3, lock: &[u8]) -> Result<(), i8> {
    // 1. exactly one config cell dep: type `csr-config-type` (data2) with args `config_id`
    let config_type_hash = attest::config_type_hash(&CONFIG_TYPE_CODE_HASH, config_id);
    let mut config = None;
    for_each_hash(Source::CellDep, CellField::TypeHash, |i, h| {
        if h.map_or(false, |h| *h == config_type_hash) {
            if config.is_some() {
                return Err(ERR_CONFIG_DEP);
            }
            config = Some(i);
        }
        Ok(true)
    })?;
    let config = cell_data(config.ok_or(ERR_CONFIG_DEP)?, Source::CellDep)?;
    let (monitor_pubkey, clock_ms) = attest::config_fields(&config).ok_or(ERR_CONFIG_DATA)?;

    // 2.–4. signed by the monitor over this entry and this registry deployment, not expired, within the cap
    let mut own_hash = [0u8; 32];
    syscalls::load_script_hash(&mut own_hash, 0).map_err(sys)?;
    attest::check_attestation(r, &own_hash, monitor_pubkey, clock_ms).map_err(|e| match e {
        AttestError::Signature => ERR_SIGNATURE,
        AttestError::Expired => ERR_EXPIRED,
        AttestError::LifetimeOverCap => ERR_LIFETIME_CAP,
    })?;
    // 5. lock
    check_lock(lock, r.service_pubkey)
}

/// Owner hash from a registry cell's dual-lock args, and whether an input with it is present.
fn owner_present(lock: &[u8]) -> Result<bool, i8> {
    input_with_lock_hash(owner_hash(lock)?)
}

/// `(owner, runner)` hashes of a registry cell's dual lock (args 32 bytes: owner = runner; or 64 bytes).
fn lock_hashes(lock: &[u8]) -> Result<(&[u8], &[u8]), i8> {
    let (_, _, args) = mol::script(lock).ok_or(ERR_LOCK)?;
    attest::dual_lock_hashes(args).ok_or(ERR_LOCK)
}

/// Runner hash of a registry cell's dual lock.
fn runner_hash(lock: &[u8]) -> Result<&[u8], i8> {
    Ok(lock_hashes(lock)?.1)
}

/// Owner hash of a registry cell's dual lock.
fn owner_hash(lock: &[u8]) -> Result<&[u8], i8> {
    Ok(lock_hashes(lock)?.0)
}

/// Rule 6: the runner key cell of the output's runner lock is an input (proves possession of the service key).
fn check_runner_key(out_lock: &[u8]) -> Result<(), i8> {
    if input_with_lock_hash(runner_hash(out_lock)?)? {
        Ok(())
    } else {
        Err(ERR_NO_RUNNER_KEY)
    }
}

/// Fills `buf` from the transaction at `offset` (a longer remainder is fine).
fn load_tx_at(buf: &mut [u8], offset: usize) -> Result<(), i8> {
    match syscalls::load_transaction(buf, offset) {
        Ok(n) if n >= buf.len() => Ok(()),
        Err(SysError::LengthNotEnough(n)) if n >= buf.len() => Ok(()),
        Ok(_) => Err(3),
        Err(e) => Err(sys(e)),
    }
}

fn read_u32(b: &[u8], at: usize) -> usize {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap()) as usize
}

/// `RawTransaction.cell_deps` of the current transaction (fixvec items, 37 bytes each).
fn raw_cell_deps() -> Result<Vec<u8>, i8> {
    // Transaction: total ‖ off(raw) ‖ off(witnesses); RawTransaction: total ‖ 6 field offsets.
    let mut tx_header = [0u8; 12];
    load_tx_at(&mut tx_header, 0)?;
    let raw = read_u32(&tx_header, 4);
    let mut raw_header = [0u8; 28];
    load_tx_at(&mut raw_header, raw)?;
    let (start, end) = (read_u32(&raw_header, 8), read_u32(&raw_header, 12));
    if end < start + 4 {
        return Err(ERR_STAKE_DEP);
    }
    let mut deps = alloc::vec![0u8; end - start];
    load_tx_at(&mut deps, raw + start)?;
    let items = &deps[4..];
    if items.len() != read_u32(&deps, 0) * CELL_DEP_SIZE {
        return Err(ERR_STAKE_DEP);
    }
    Ok(items.to_vec())
}

/// Stake rules 1–4 (`spec/registry.md` §4.1): the registry's `stake` is a code cell dep at a position where its
/// raw index equals its `Source::CellDep` index, it is a Nervos DAO deposit of at least `MIN_STAKE`, and its
/// owner (the deposit's lock hash, not necessarily the registry owner) consents.
fn check_stake(r: &RegistryV3) -> Result<(), i8> {
    let stake = r.stake;
    // 1. position j: the stake entry is code and so is every entry before it
    let deps = raw_cell_deps()?;
    let mut found = None;
    for (j, dep) in deps.chunks_exact(CELL_DEP_SIZE).enumerate() {
        if dep[36] != DEP_TYPE_CODE {
            break;
        }
        if dep[..36] == stake[..] {
            found = Some(j);
            break;
        }
    }
    let j = found.ok_or(ERR_STAKE_DEP)?;

    // 2. a Nervos DAO deposit: DAO type script with empty args, data = 8 zero bytes
    let type_script = load_all(|b, o| syscalls::load_cell_by_field(b, o, j, Source::CellDep, CellField::Type))
        .map_err(|_| ERR_STAKE_NOT_DEPOSIT)?;
    let (code_hash, hash_type, args) = mol::script(&type_script).ok_or(ERR_STAKE_NOT_DEPOSIT)?;
    if *code_hash != DAO_TYPE_CODE_HASH || hash_type != HASH_TYPE_TYPE || !args.is_empty() {
        return Err(ERR_STAKE_NOT_DEPOSIT);
    }
    if cell_data(j, Source::CellDep)? != [0u8; 8] {
        return Err(ERR_STAKE_NOT_DEPOSIT);
    }

    // 3. capacity
    if load_cell_capacity(j, Source::CellDep).map_err(sys)? < MIN_STAKE {
        return Err(ERR_STAKE_CAPACITY);
    }

    // 4. the stake owner consents
    let mut stake_owner = [0u8; 32];
    syscalls::load_cell_by_field(&mut stake_owner, 0, j, Source::CellDep, CellField::LockHash).map_err(sys)?;
    check_consent(r, &stake_owner)
}

/// `Some(content)` of a molecule `BytesOpt` (empty = `None`).
fn bytes_opt(b: &[u8]) -> Option<Option<&[u8]>> {
    if b.is_empty() {
        Some(None)
    } else {
        mol::bytes(b).map(Some)
    }
}

/// Stake rule 4: an input with the stake owner's lock hash, or a signature of the stake owner over the consent
/// text in `WitnessArgs.output_type` of the registry output's witness.
fn check_consent(r: &RegistryV3, stake_owner: &[u8; 32]) -> Result<(), i8> {
    if input_with_lock_hash(stake_owner)? {
        return Ok(());
    }
    let witness = load_all(|b, o| syscalls::load_witness(b, o, 0, Source::GroupOutput)).map_err(|_| ERR_STAKE_NO_CONSENT)?;
    let [_, _, output_type] = mol::table::<3>(&witness).ok_or(ERR_STAKE_NO_CONSENT)?;
    let sig = bytes_opt(output_type).ok_or(ERR_STAKE_NO_CONSENT)?.ok_or(ERR_STAKE_NO_CONSENT)?;
    let (rs, recid) = consent::parse_signature(sig).ok_or(ERR_CONSENT_SIGNATURE)?;
    Signature::from_slice(&rs).map_err(|_| ERR_CONSENT_SIGNATURE)?;

    let peer_id = mol::bytes(r.peer_id).ok_or(ERR_DATA)?;
    let digest = consent::consent_digest(r.network, r.stake, r.service_pubkey, peer_id).ok_or(ERR_DATA)?;
    let pubkey = attest::recover_parts(&digest, &rs, recid).ok_or(ERR_CONSENT_MISMATCH)?;
    if is_sighash_lock_of(stake_owner, &blake2b_256(&[&pubkey])[..20]) {
        Ok(())
    } else {
        Err(ERR_CONSENT_MISMATCH)
    }
}

fn run() -> Result<(), i8> {
    let script = load_all(|b, o| syscalls::load_script(b, o))?;
    let (_, _, config_id) = mol::script(&script).ok_or(ERR_ARGS_LEN)?;
    let config_id: &[u8; 32] = config_id.try_into().map_err(|_| ERR_ARGS_LEN)?;
    let n_in = group_count(Source::GroupInput)?;
    let n_out = group_count(Source::GroupOutput)?;

    if n_out == 0 {
        // destroy (n_in == 1; a group always has at least one cell)
        let lock = cell_lock(0, Source::GroupInput)?;
        return if owner_present(&lock)? { Ok(()) } else { Err(ERR_DESTROY) };
    }

    let out_data = cell_data(0, Source::GroupOutput)?;
    let out = attest::parse_v3(&out_data).ok_or(ERR_DATA)?;
    let out_lock = cell_lock(0, Source::GroupOutput)?;

    // Rule 6 applies on creation and whenever the owner changes the service key / runner lock;
    // the stake rules (with the consent) on creation and whenever the owner changes the stake, the owner,
    // `peer_id`, `network` or `service_pubkey` (decision D23 A).
    let mut need_runner_key = n_in == 0;
    let mut need_stake = n_in == 0;
    if n_in == 1 {
        let in_lock = cell_lock(0, Source::GroupInput)?;
        let in_data = cell_data(0, Source::GroupInput)?;
        if owner_present(&in_lock)? {
            let (new_key, new_identity, new_stake) = match attest::parse_v3(&in_data) {
                Some(inp) => (
                    inp.service_pubkey != out.service_pubkey,
                    inp.peer_id != out.peer_id || inp.network != out.network,
                    inp.stake != out.stake,
                ),
                None => (true, true, true),
            };
            let new_runner = match (runner_hash(&in_lock), runner_hash(&out_lock)) {
                (Ok(a), Ok(b)) => a != b,
                _ => true,
            };
            let new_owner = match (owner_hash(&in_lock), owner_hash(&out_lock)) {
                (Ok(a), Ok(b)) => a != b,
                _ => true,
            };
            need_runner_key = new_key || new_runner;
            need_stake = new_stake || new_owner || new_key || new_identity;
        } else {
            // runner path: only expiry_ms, attestation_sig, endpoint and prekeys may change (no stake check:
            // `stake` stays fixed)
            let inp = attest::parse_v3(&in_data).ok_or(ERR_DATA)?;
            if inp.peer_id != out.peer_id
                || inp.service_pubkey != out.service_pubkey
                || inp.payout_lock_hash != out.payout_lock_hash
                || inp.network != out.network
                || inp.stake != out.stake
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
    check_output(config_id, &out, &out_lock)?;
    if need_runner_key {
        check_runner_key(&out_lock)?;
    }
    if need_stake {
        check_stake(&out)?;
    }
    Ok(())
}
