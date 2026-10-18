#![no_std]
#![no_main]

use ckb_std::ckb_constants::{CellField, Source};
use ckb_std::error::SysError;
use ckb_std::syscalls;

ckb_std::entry!(program_entry);
ckb_std::default_alloc!();

const ERR_ARGS_LEN: i8 = 5;
const ERR_NO_KEY_CELL: i8 = 6;

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

/// Extracts the `args` bytes of a molecule-encoded Script.
fn script_args(s: &[u8]) -> Option<&[u8]> {
    if read_u32(s, 0)? != s.len() {
        return None;
    }
    let start = read_u32(s, 12)?;
    let len = read_u32(s, start)?;
    s.get(start + 4..start + 4 + len)
}

fn run() -> Result<(), i8> {
    let mut sbuf = [0u8; 256];
    let script = match syscalls::load_script(&mut sbuf, 0) {
        Ok(n) => &sbuf[..n],
        Err(SysError::LengthNotEnough(_)) => return Err(ERR_ARGS_LEN),
        Err(e) => return Err(sys(e)),
    };
    let args = script_args(script).ok_or(ERR_ARGS_LEN)?;
    if args.len() != 64 {
        return Err(ERR_ARGS_LEN);
    }
    let (owner, runner) = args.split_at(32);
    let mut hash = [0u8; 32];
    for i in 0.. {
        match syscalls::load_cell_by_field(&mut hash, 0, i, Source::Input, CellField::LockHash) {
            Ok(_) if hash[..] == *owner || hash[..] == *runner => return Ok(()),
            Ok(_) => {}
            Err(SysError::IndexOutOfBound) => break,
            Err(e) => return Err(sys(e)),
        }
    }
    Err(ERR_NO_KEY_CELL)
}
