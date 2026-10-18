#![no_std]
#![no_main]

use ckb_std::ckb_constants::{CellField, Source};
use ckb_std::error::SysError;
use ckb_std::syscalls;

ckb_std::entry!(program_entry);
ckb_std::default_alloc!();

const ERR_ARGS_LEN: i8 = 5;
const ERR_NO_KEY_CELL: i8 = 6;
const ERR_OUTPUT_COUNT: i8 = 7;
const ERR_CAPACITY: i8 = 8;
const ERR_OVERFLOW: i8 = 9;
const ERR_OUTPUT_SHAPE: i8 = 10;

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

/// Loads into `buf`; a length larger than `buf` cannot be our (<= 4 + 12 + 33 + 1 + 4 + 72 byte) script.
fn load_raw<'a>(f: impl Fn(&mut [u8]) -> Result<usize, SysError>, buf: &'a mut [u8]) -> Result<&'a [u8], i8> {
    match f(buf) {
        Ok(n) => Ok(&buf[..n]),
        Err(SysError::LengthNotEnough(_)) => Ok(&buf[..0]),
        Err(e) => Err(sys(e)),
    }
}

fn read_u32(b: &[u8], at: usize) -> Option<usize> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?) as usize)
}

/// Extracts the `args` bytes of a molecule-encoded Script.
fn script_args(s: &[u8]) -> Option<&[u8]> {
    if read_u32(s, 0)? != s.len() {
        return None;
    }
    let start = read_u32(s, 12)?;
    let len = read_u32(s, start)?;
    s.get(start + 4..start + 4 + len)
}

fn load_cell_capacity(i: usize, src: Source) -> Result<u64, SysError> {
    let mut b = [0u8; 8];
    syscalls::load_cell_by_field(&mut b, 0, i, src, CellField::Capacity)?;
    Ok(u64::from_le_bytes(b))
}

fn has_input_with_lock_hash(hash: &[u8]) -> Result<bool, i8> {
    let mut h = [0u8; 32];
    for i in 0.. {
        match syscalls::load_cell_by_field(&mut h, 0, i, Source::Input, CellField::LockHash) {
            Ok(_) if h[..] == *hash => return Ok(true),
            Ok(_) => {}
            Err(SysError::IndexOutOfBound) => break,
            Err(e) => return Err(sys(e)),
        }
    }
    Ok(false)
}

fn run() -> Result<(), i8> {
    // Script molecule table: total_size, 3 offsets (code_hash, hash_type, args), fields.
    let mut sbuf = [0u8; 256];
    let script = load_raw(|b| syscalls::load_script(b, 0), &mut sbuf)?;
    let args = script_args(script).ok_or(ERR_ARGS_LEN)?;
    if args.len() != 72 {
        return Err(ERR_ARGS_LEN);
    }
    let owner = &args[0..32];
    let runner = &args[32..64];
    // args[64..72] (`max_fee_shannons`) is reserved: the runner path allows no capacity loss at all.

    // Owner path: anything allowed.
    if has_input_with_lock_hash(owner)? {
        return Ok(());
    }
    // Runner path.
    if !has_input_with_lock_hash(runner)? {
        return Err(ERR_NO_KEY_CELL);
    }

    let mut in_sum: u64 = 0;
    for i in 0.. {
        match load_cell_capacity(i, Source::GroupInput) {
            Ok(c) => in_sum = in_sum.checked_add(c).ok_or(ERR_OVERFLOW)?,
            Err(SysError::IndexOutOfBound) => break,
            Err(e) => return Err(sys(e)),
        }
    }

    let mut obuf = [0u8; 256];
    let mut count = 0u32;
    let mut out_idx = 0usize;
    for i in 0.. {
        match load_raw(|b| syscalls::load_cell_by_field(b, 0, i, Source::Output, CellField::Lock), &mut obuf) {
            Ok(lock) => {
                if lock == script {
                    count += 1;
                    out_idx = i;
                }
            }
            Err(1) => break, // IndexOutOfBound
            Err(e) => return Err(e),
        }
    }
    if count != 1 {
        return Err(ERR_OUTPUT_COUNT);
    }
    // Float cells never carry a type script or data.
    let mut tbuf = [0u8; 32];
    match syscalls::load_cell_by_field(&mut tbuf, 0, out_idx, Source::Output, CellField::Type) {
        Err(SysError::ItemMissing) => {}
        Ok(_) | Err(SysError::LengthNotEnough(_)) => return Err(ERR_OUTPUT_SHAPE),
        Err(e) => return Err(sys(e)),
    }
    match syscalls::load_cell_data(&mut tbuf, 0, out_idx, Source::Output) {
        Ok(0) => {}
        Ok(_) | Err(SysError::LengthNotEnough(_)) => return Err(ERR_OUTPUT_SHAPE),
        Err(e) => return Err(sys(e)),
    }
    // No capacity may leave the float on the runner path (fees are paid from the job cell).
    let out_cap = load_cell_capacity(out_idx, Source::Output).map_err(sys)?;
    if out_cap < in_sum {
        return Err(ERR_CAPACITY);
    }
    Ok(())
}
