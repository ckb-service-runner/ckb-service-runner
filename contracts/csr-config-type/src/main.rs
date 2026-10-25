//! `csr-config-type`: type script of the monitor config cell, args = `type_id (32)`. See `spec/registry.md` §1.
//!
//! - Type ID semantics (`ckb_std::type_id`): at most one cell with this type script per transaction side, and a
//!   new cell's args are the Type ID of its first input and output index.
//! - Data `monitor_pubkey (33) ‖ clock_ms (u64 LE) ‖ reserved`, at least 41 bytes.
//! - Update: `monitor_pubkey` unchanged, `clock_ms` not smaller (decision D24 A).
//! - Destroying the cell is refused.
#![no_std]
#![no_main]

use ckb_std::ckb_constants::Source;
use ckb_std::error::SysError;
use ckb_std::syscalls;

ckb_std::entry!(program_entry);
ckb_std::default_alloc!();

const ERR_ARGS_LEN: i8 = 5;
const ERR_TYPE_ID: i8 = 6;
const ERR_DATA: i8 = 7;
const ERR_MONITOR_KEY: i8 = 8;
const ERR_CLOCK_BACKWARD: i8 = 9;
const ERR_DESTROY: i8 = 10;

const MIN_DATA: usize = 41;

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

fn read_u32(b: &[u8], at: usize) -> Option<usize> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?) as usize)
}

/// Length of the `args` of the current (molecule) script.
fn args_len() -> Result<usize, i8> {
    let mut buf = [0u8; 256];
    let n = match syscalls::load_script(&mut buf, 0) {
        Ok(n) => n,
        Err(SysError::LengthNotEnough(_)) => return Err(ERR_ARGS_LEN),
        Err(e) => return Err(sys(e)),
    };
    let s = &buf[..n];
    let start = read_u32(s, 12).ok_or(ERR_ARGS_LEN)?;
    read_u32(s, start).ok_or(ERR_ARGS_LEN)
}

/// Whether the group source has a cell at `i`.
fn present(i: usize, src: Source) -> Result<bool, i8> {
    match syscalls::load_cell_data(&mut [], 0, i, src) {
        Ok(_) | Err(SysError::LengthNotEnough(_)) => Ok(true),
        Err(SysError::IndexOutOfBound) => Ok(false),
        Err(e) => Err(sys(e)),
    }
}

/// The first 41 bytes of the data of cell 0 of `src` (error 7 if shorter).
fn head(src: Source) -> Result<[u8; MIN_DATA], i8> {
    let mut b = [0u8; MIN_DATA];
    match syscalls::load_cell_data(&mut b, 0, 0, src) {
        Ok(n) if n >= MIN_DATA => Ok(b),
        Err(SysError::LengthNotEnough(n)) if n >= MIN_DATA => Ok(b),
        Ok(_) => Err(ERR_DATA),
        Err(e) => Err(sys(e)),
    }
}

fn run() -> Result<(), i8> {
    if args_len()? != 32 {
        return Err(ERR_ARGS_LEN);
    }
    ckb_std::type_id::check_type_id(0, 32).map_err(|_| ERR_TYPE_ID)?;

    let has_in = present(0, Source::GroupInput)?;
    let has_out = present(0, Source::GroupOutput)?;
    if !has_out {
        return Err(ERR_DESTROY);
    }
    let out = head(Source::GroupOutput)?;
    if has_in {
        let inp = head(Source::GroupInput)?;
        if inp[..33] != out[..33] {
            return Err(ERR_MONITOR_KEY);
        }
        let clock = |d: &[u8; MIN_DATA]| u64::from_le_bytes(d[33..41].try_into().unwrap());
        if clock(&out) < clock(&inp) {
            return Err(ERR_CLOCK_BACKWARD);
        }
    }
    Ok(())
}
