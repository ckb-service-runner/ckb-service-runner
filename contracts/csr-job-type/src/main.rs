//! `csr-job-type` type script, args = `service_code_hash (32)`. See `spec/job-cell-standard.md`.
#![no_std]
#![no_main]

#[path = "../../csr-registry/src/mol.rs"]
#[allow(dead_code)]
mod mol;
#[allow(dead_code)]
mod job;
#[allow(dead_code)]
mod util;

use ckb_std::ckb_constants::Source;
use job::Trigger;
use util::*;

ckb_std::entry!(program_entry);
ckb_std::default_alloc!();

const ERR_ARGS_LEN: i8 = 5;
const ERR_DATA: i8 = 6;
const ERR_RUNS: i8 = 7;
const ERR_CAPACITY: i8 = 8;
const ERR_UPDATE: i8 = 9;
const ERR_UPDATE_FIELDS: i8 = 10;

const SHANNONS_PER_BYTE: u64 = 100_000_000;

fn program_entry() -> i8 {
    match run() {
        Ok(()) => 0,
        Err(code) => code,
    }
}

/// Bytes a script occupies on chain: code hash (32) + hash type (1) + args.
fn script_bytes(raw: &[u8]) -> Result<u64, i8> {
    let (_, _, args) = mol::script(raw).ok_or(ERR_CAPACITY)?;
    Ok(33 + args.len() as u64)
}

/// Occupied capacity (shannons) of group output 0: capacity field (8) + lock + type + data.
fn occupied(data_len: usize) -> Result<u64, i8> {
    let lock = cell_lock(0, Source::GroupOutput)?;
    let type_ = cell_type(0, Source::GroupOutput)?.ok_or(ERR_CAPACITY)?;
    let bytes = 8 + script_bytes(&lock)? + script_bytes(&type_)? + data_len as u64;
    bytes.checked_mul(SHANNONS_PER_BYTE).ok_or(ERR_CAPACITY)
}

fn create() -> Result<(), i8> {
    let data = cell_data(0, Source::GroupOutput)?;
    let job = job::parse(&data).ok_or(ERR_DATA)?;
    let runs = job.remaining_runs;
    let every = matches!(job.trigger, Trigger::Every(_));
    if runs == 0 || (!every && runs != 1) {
        return Err(ERR_RUNS);
    }
    let per_run = (job.tip_shannons as u128) + (job.max_fee_shannons as u128);
    let need = occupied(data.len())? as u128 + (runs as u128) * per_run;
    let cap = capacity(0, Source::GroupOutput).map_err(sys)?;
    if (cap as u128) < need {
        return Err(ERR_CAPACITY);
    }
    Ok(())
}

fn update() -> Result<(), i8> {
    let in_data = cell_data(0, Source::GroupInput)?;
    let out_data = cell_data(0, Source::GroupOutput)?;
    let inp = job::parse(&in_data).ok_or(ERR_DATA)?;
    let out = job::parse(&out_data).ok_or(ERR_DATA)?;
    if !matches!(inp.trigger, Trigger::Every(_)) {
        return Err(ERR_UPDATE);
    }
    if out.remaining_runs == 0 || !job::is_next_run(&in_data, inp.remaining_runs, &out_data, out.remaining_runs) {
        return Err(ERR_UPDATE_FIELDS);
    }
    Ok(())
}

fn run() -> Result<(), i8> {
    let script = script()?;
    let (_, _, args) = mol::script(&script).ok_or(ERR_ARGS_LEN)?;
    if args.len() != 32 {
        return Err(ERR_ARGS_LEN);
    }
    let n_in = group_count(Source::GroupInput, ERR_UPDATE)?;
    let n_out = group_count(Source::GroupOutput, ERR_UPDATE)?;
    match (n_in, n_out) {
        (0, 1) => create(),
        // consuming a job cell: the job lock decides
        (1, 0) => Ok(()),
        (1, 1) => update(),
        _ => Err(ERR_UPDATE),
    }
}
