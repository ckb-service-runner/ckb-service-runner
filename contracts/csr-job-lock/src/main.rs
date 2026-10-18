//! `csr-job-lock`, args = `registry_type_hash (32)`. See `spec/job-cell-standard.md`.
#![no_std]
#![no_main]

#[path = "../../csr-registry/src/mol.rs"]
#[allow(dead_code)]
mod mol;
#[path = "../../csr-job-type/src/job.rs"]
#[allow(dead_code)]
mod job;
#[path = "../../csr-job-type/src/util.rs"]
#[allow(dead_code)]
mod util;

use ckb_std::ckb_constants::{CellField, InputField, Source};
use ckb_std::syscalls;
use job::{Job, Trigger};
use util::*;

ckb_std::entry!(program_entry);
ckb_std::default_alloc!();

const ERR_ARGS_LEN: i8 = 5;
const ERR_DATA: i8 = 6;
const ERR_JOB_INPUTS: i8 = 7;
const ERR_SINCE_FORM: i8 = 8;
const ERR_TRIGGER: i8 = 9;
const ERR_REGISTRY_DEP: i8 = 10;
const ERR_TIP: i8 = 11;
const ERR_RECREATE: i8 = 12;
const ERR_REFUND: i8 = 13;
const ERR_NO_PATH: i8 = 14;
const ERR_NOT_REGISTERED_RUNNER: i8 = 15;
const ERR_CONFIG_DEP: i8 = 16;
const ERR_ATTESTATION_EXPIRED: i8 = 17;

/// `since` flags (top byte): bit 7 relative, bits 5–6 metric (0 block, 1 epoch, 2 timestamp).
const SINCE_ABS_BLOCK: u64 = 0x00;
const SINCE_ABS_EPOCH: u64 = 0x20;
const SINCE_ABS_TIMESTAMP: u64 = 0x40;
const SINCE_REL_BLOCK: u64 = 0x80;
const SINCE_VALUE_MASK: u64 = 0x00ff_ffff_ffff_ffff;

fn program_entry() -> i8 {
    match run() {
        Ok(()) => 0,
        Err(code) => code,
    }
}

fn since_parts(since: u64) -> (u64, u64) {
    (since >> 56, since & SINCE_VALUE_MASK)
}

/// `(number, index, length)` of a packed epoch; length 0 is read as a whole epoch (index 0, length 1).
fn epoch_parts(e: u64) -> (u128, u128, u128) {
    let number = (e & 0xff_ffff) as u128;
    let index = ((e >> 24) & 0xffff) as u128;
    let length = ((e >> 40) & 0xffff) as u128;
    if length == 0 {
        (number, 0, 1)
    } else {
        (number, index, length)
    }
}

/// `a ≥ b` for packed epochs with fraction (number + index / length).
fn epoch_ge(a: u64, b: u64) -> bool {
    let (an, ai, al) = epoch_parts(a);
    let (bn, bi, bl) = epoch_parts(b);
    (an * al + ai) * bl >= (bn * bl + bi) * al
}

/// Run path rule 1: the trigger is proven by the job input's `since`.
fn check_trigger(trigger: Trigger, since: u64) -> Result<(), i8> {
    let (flags, value) = since_parts(since);
    let (want_flags, reached) = match trigger {
        Trigger::OnDemand => return Ok(()),
        Trigger::AtBlock(n) => (SINCE_ABS_BLOCK, value >= n),
        Trigger::AtEpoch(e) => (SINCE_ABS_EPOCH, epoch_ge(value, e)),
        // since timestamps are seconds, the trigger is in milliseconds
        Trigger::AtTimestamp(t) => (SINCE_ABS_TIMESTAMP, (value as u128) * 1000 >= t as u128),
        Trigger::Every(k) => (SINCE_REL_BLOCK, value >= k),
    };
    if flags != want_flags {
        return Err(ERR_SINCE_FORM);
    }
    if !reached {
        return Err(ERR_TRIGGER);
    }
    Ok(())
}

/// Index of the registry cell dep: `Ok(None)` if there is none, error 10 if there are several.
fn registry_dep(registry_type_hash: &[u8]) -> Result<Option<usize>, i8> {
    let mut found = None;
    for_each_hash(Source::CellDep, CellField::TypeHash, |i, h| {
        if h.map_or(false, |h| h[..] == *registry_type_hash) {
            if found.is_some() {
                return Err(ERR_REGISTRY_DEP);
            }
            found = Some(i);
        }
        Ok(true)
    })?;
    Ok(found)
}

/// `float_lock_hash` (field 2) of `molecule(Registry)`.
fn float_lock_hash(registry: &[u8]) -> Option<[u8; 32]> {
    let [_, _, float, _, _, _] = mol::table::<6>(registry)?;
    Some(*mol::fixed::<32>(float)?)
}

/// `attestation.expiry_ms` (field 4, sub-field 3) of `molecule(Registry)`.
fn attestation_expiry(registry: &[u8]) -> Option<u64> {
    let [_, _, _, _, attestation, _] = mol::table::<6>(registry)?;
    let [_, _, _, expiry, _] = mol::table::<5>(attestation)?;
    Some(u64::from_le_bytes(*mol::fixed::<8>(expiry)?))
}

/// Run path rule 3: an input with the registry cell's runner lock hash (its dual lock's `args[32..64]`),
/// i.e. the registered node's runner key cell, proves that the registered node makes the transaction.
fn check_registered_runner(dep: usize) -> Result<(), i8> {
    let lock = cell_lock(dep, Source::CellDep)?;
    let (_, _, args) = mol::script(&lock).ok_or(ERR_REGISTRY_DEP)?;
    if args.len() != 64 {
        return Err(ERR_REGISTRY_DEP);
    }
    if input_with_lock_hash(&args[32..64])? {
        Ok(())
    } else {
        Err(ERR_NOT_REGISTERED_RUNNER)
    }
}

/// Run path rule 4: exactly one cell dep with the registry's `config_type_hash` (its type script args), and
/// the registry's `attestation.expiry_ms > clock_ms` of that config cell. The signature is not re-verified.
fn check_attestation_live(dep: usize, registry: &[u8]) -> Result<(), i8> {
    let reg_type = cell_type(dep, Source::CellDep)?.ok_or(ERR_REGISTRY_DEP)?;
    let (_, _, config_type_hash) = mol::script(&reg_type).ok_or(ERR_REGISTRY_DEP)?;
    if config_type_hash.len() != 32 {
        return Err(ERR_REGISTRY_DEP);
    }
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
    // config data: monitor_pubkey (33) ‖ clock_ms (u64 LE) ‖ reserved
    let clock_ms = u64::from_le_bytes(config.get(33..41).ok_or(ERR_CONFIG_DEP)?.try_into().unwrap());
    let expiry_ms = attestation_expiry(registry).ok_or(ERR_REGISTRY_DEP)?;
    if expiry_ms > clock_ms {
        Ok(())
    } else {
        Err(ERR_ATTESTATION_EXPIRED)
    }
}

/// Σ outputs − Σ inputs (capacity) of cells with lock hash `h`, ≥ `need`.
fn net_inflow_at_least(h: &[u8], need: u128) -> Result<bool, i8> {
    let out = capacity_with_lock_hash(Source::Output, h)?;
    let inp = capacity_with_lock_hash(Source::Input, h)?;
    Ok(out >= inp + need)
}

/// Run path rule 5, `remaining_runs > 1`: exactly one recreated job cell with runs − 1 and enough capacity.
fn check_recreated(lock: &[u8], job: &Job, in_data: &[u8], min_cap: u64) -> Result<(), i8> {
    let in_type = cell_type(0, Source::GroupInput)?;
    let mut found = None;
    for i in 0.. {
        let out_lock = match cell_lock(i, Source::Output) {
            Ok(l) => l,
            Err(1) => break, // IndexOutOfBound
            Err(e) => return Err(e),
        };
        if out_lock == lock && cell_type(i, Source::Output)? == in_type {
            if found.is_some() {
                return Err(ERR_RECREATE);
            }
            found = Some(i);
        }
    }
    let i = found.ok_or(ERR_RECREATE)?;
    let out_data = cell_data(i, Source::Output)?;
    let out = job::parse(&out_data).ok_or(ERR_RECREATE)?;
    if !job::is_next_run(in_data, job.remaining_runs, &out_data, out.remaining_runs) {
        return Err(ERR_RECREATE);
    }
    if capacity(i, Source::Output).map_err(sys)? < min_cap {
        return Err(ERR_RECREATE);
    }
    Ok(())
}

fn run() -> Result<(), i8> {
    let script = script()?;
    let (code_hash, _, args) = mol::script(&script).ok_or(ERR_ARGS_LEN)?;
    if args.len() != 32 {
        return Err(ERR_ARGS_LEN);
    }
    let registry_type_hash = args;

    // One job input per transaction: no other input is locked by this code (any args, any data hash type).
    let mut job_inputs = 0;
    for i in 0.. {
        match cell_lock(i, Source::Input) {
            Ok(l) => {
                if mol::script(&l).map_or(false, |(c, _, _)| c == code_hash) {
                    job_inputs += 1;
                }
            }
            Err(1) => break,
            Err(e) => return Err(e),
        }
    }
    if job_inputs != 1 {
        return Err(ERR_JOB_INPUTS);
    }

    let in_data = cell_data(0, Source::GroupInput)?;
    let job = job::parse(&in_data).ok_or(ERR_DATA)?;
    let mut sb = [0u8; 8];
    syscalls::load_input_by_field(&mut sb, 0, 0, Source::GroupInput, InputField::Since).map_err(sys)?;
    let since = u64::from_le_bytes(sb);

    // Refund path: absolute block ≥ expiry_block and a refund-lock input → anything allowed.
    let (flags, value) = since_parts(since);
    if flags == SINCE_ABS_BLOCK && value >= job.expiry_block && input_with_lock_hash(&job.refund_lock_hash)? {
        return Ok(());
    }

    // Run path. Without a registry cell dep the transaction can only be a (failed) refund.
    let dep = registry_dep(registry_type_hash)?.ok_or(ERR_NO_PATH)?;

    // 1. trigger
    check_trigger(job.trigger, since)?;

    // 2. tip to the registered float
    let registry = cell_data(dep, Source::CellDep)?;
    let float = float_lock_hash(&registry).ok_or(ERR_REGISTRY_DEP)?;
    if !net_inflow_at_least(&float, job.tip_shannons as u128)? {
        return Err(ERR_TIP);
    }

    // 3. made by the registered node (its runner key cell is an input)
    check_registered_runner(dep)?;

    // 4. the registry's attestation has not expired by the monitor clock
    check_attestation_live(dep, &registry)?;

    // 5. no siphoning: the job keeps (or refunds) everything but tip and fee
    let in_cap = capacity(0, Source::GroupInput).map_err(sys)?;
    let min_cap = in_cap.saturating_sub(job.tip_shannons).saturating_sub(job.max_fee_shannons);
    if job.remaining_runs > 1 {
        check_recreated(&script, &job, &in_data, min_cap)
    } else if net_inflow_at_least(&job.refund_lock_hash, min_cap as u128)? {
        Ok(())
    } else {
        Err(ERR_REFUND)
    }
}
