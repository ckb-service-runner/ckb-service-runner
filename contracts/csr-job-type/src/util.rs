//! Syscall helpers shared by `csr-job-type` and `csr-job-lock` (`#[path]`-included by the lock).
//! Only `ckb_std::syscalls` is used (no `high_level`, no `ckb-types`; see `docs/decisions.md`).

use alloc::vec::Vec;
use ckb_std::ckb_constants::{CellField, Source};
use ckb_std::error::SysError;
use ckb_std::syscalls;

pub fn sys(e: SysError) -> i8 {
    match e {
        SysError::IndexOutOfBound => 1,
        SysError::ItemMissing => 2,
        SysError::LengthNotEnough(_) => 3,
        _ => 4,
    }
}

/// Loads a field of unknown length completely (the first call learns the size).
pub fn load_all(f: impl Fn(&mut [u8], usize) -> Result<usize, SysError>) -> Result<Vec<u8>, i8> {
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

pub fn script() -> Result<Vec<u8>, i8> {
    load_all(|b, o| syscalls::load_script(b, o))
}

pub fn cell_data(i: usize, src: Source) -> Result<Vec<u8>, i8> {
    load_all(|b, o| syscalls::load_cell_data(b, o, i, src))
}

pub fn cell_lock(i: usize, src: Source) -> Result<Vec<u8>, i8> {
    load_all(|b, o| syscalls::load_cell_by_field(b, o, i, src, CellField::Lock))
}

/// Raw type script of a cell, `None` if the cell has no type script.
pub fn cell_type(i: usize, src: Source) -> Result<Option<Vec<u8>>, i8> {
    match load_all(|b, o| syscalls::load_cell_by_field(b, o, i, src, CellField::Type)) {
        Ok(v) => Ok(Some(v)),
        Err(2) => Ok(None), // ItemMissing
        Err(e) => Err(e),
    }
}

pub fn capacity(i: usize, src: Source) -> Result<u64, SysError> {
    let mut b = [0u8; 8];
    syscalls::load_cell_by_field(&mut b, 0, i, src, CellField::Capacity)?;
    Ok(u64::from_le_bytes(b))
}

/// Calls `cb(index, Some(hash))` for each cell of `src` (field `f` = LockHash/TypeHash; `None` for a missing
/// type) until the end or until `cb` returns `false`.
pub fn for_each_hash(
    src: Source,
    f: CellField,
    mut cb: impl FnMut(usize, Option<&[u8; 32]>) -> Result<bool, i8>,
) -> Result<(), i8> {
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
pub fn input_with_lock_hash(want: &[u8]) -> Result<bool, i8> {
    let mut found = false;
    for_each_hash(Source::Input, CellField::LockHash, |_, h| {
        found = h.map_or(false, |h| h[..] == *want);
        Ok(!found)
    })?;
    Ok(found)
}

/// Σ capacity of the cells of `src` whose lock hash is `want`.
pub fn capacity_with_lock_hash(src: Source, want: &[u8]) -> Result<u128, i8> {
    let mut sum = 0u128;
    let mut err = None;
    for_each_hash(src, CellField::LockHash, |i, h| {
        if h.map_or(false, |h| h[..] == *want) {
            match capacity(i, src) {
                Ok(c) => sum += c as u128,
                Err(e) => {
                    err = Some(sys(e));
                    return Ok(false);
                }
            }
        }
        Ok(true)
    })?;
    match err {
        Some(e) => Err(e),
        None => Ok(sum),
    }
}

/// Number of cells (0 or 1) in a group source; `too_many` if there are more.
pub fn group_count(src: Source, too_many: i8) -> Result<usize, i8> {
    match capacity(1, src) {
        Ok(_) => return Err(too_many),
        Err(SysError::IndexOutOfBound) => {}
        Err(e) => return Err(sys(e)),
    }
    match capacity(0, src) {
        Ok(_) => Ok(1),
        Err(SysError::IndexOutOfBound) => Ok(0),
        Err(e) => Err(sys(e)),
    }
}
