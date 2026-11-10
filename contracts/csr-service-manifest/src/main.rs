//! `csr-service-manifest`: type script of a service manifest cell, args = `type_id (32)`.
//! See `spec/service-manifest.md`.
//!
//! - Type ID semantics (`ckb_std::type_id`), like `csr-config-type`.
//! - Every output of the group holds a valid `ServiceManifestV1` (molecule table, all caps, UTF-8, https URLs).
//! - Creation: the code cell (and every data cell) is a cell dep with the stated out point and data hash, and the
//!   code cell's lock is the manifest's lock (publisher binding).
//! - Update: binary, args and data cells are fixed; the lock is fixed; the text fields and `deprecated` change.
//! - Destruction is allowed (the lock authorises it). At most one input and one output of the group.
#![no_std]
#![no_main]

use alloc::vec::Vec;
use ckb_std::ckb_constants::{CellField, Source};
use ckb_std::error::SysError;
use ckb_std::syscalls;

ckb_std::entry!(program_entry);
ckb_std::default_alloc!();

const ERR_ARGS_LEN: i8 = 5;
const ERR_TYPE_ID: i8 = 6;
const ERR_GROUP_COUNT: i8 = 7;
const ERR_DATA: i8 = 8;
const ERR_VERSION_TAG: i8 = 9;
const ERR_CAP: i8 = 10;
const ERR_UTF8: i8 = 11;
const ERR_URL: i8 = 12;
const ERR_DATA_CELLS: i8 = 13;
const ERR_DEPRECATED: i8 = 14;
const ERR_CODE_DEP: i8 = 15;
const ERR_PUBLISHER: i8 = 16;
const ERR_DATA_DEP: i8 = 17;
const ERR_UPDATE_FIXED: i8 = 18;
const ERR_UPDATE_LOCK: i8 = 19;
const ERR_MIN_CSR: i8 = 20;

const MAX_DATA_CELLS: usize = 4;
const CELL_DEP_SIZE: usize = 37;
const DEP_TYPE_CODE: u8 = 0;

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

/// Loads a field of unknown length completely (the first call learns the size).
fn load_all(f: impl Fn(&mut [u8], usize) -> Result<usize, SysError>) -> Result<Vec<u8>, i8> {
    let mut buf = [0u8; 1024];
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

fn cell_hash(i: usize, src: Source, f: CellField) -> Result<[u8; 32], i8> {
    let mut h = [0u8; 32];
    syscalls::load_cell_by_field(&mut h, 0, i, src, f).map_err(sys)?;
    Ok(h)
}

/// Whether the group source has a cell at `i`; more than one cell is an error.
fn group_count(src: Source) -> Result<usize, i8> {
    let present = |i| match syscalls::load_cell_data(&mut [], 0, i, src) {
        Ok(_) | Err(SysError::LengthNotEnough(_)) => Ok(true),
        Err(SysError::IndexOutOfBound) => Ok(false),
        Err(e) => Err(sys(e)),
    };
    if present(1)? {
        return Err(ERR_GROUP_COUNT);
    }
    Ok(present(0)? as usize)
}

// --- molecule ---------------------------------------------------------------

const N_FIELDS: usize = 14;

struct Manifest<'a> {
    code_hash: &'a [u8],
    code_cell: &'a [u8],
    data_hashes: &'a [u8],
    data_cells: &'a [u8],
    args: &'a [u8],
    name: &'a [u8],
    version: &'a [u8],
    description: &'a [u8],
    url: &'a [u8],
    source: &'a [u8],
    language: &'a [u8],
    min_csr: &'a [u8],
    deprecated: u8,
}

/// Content of a molecule `Bytes` (u32 LE length ‖ bytes); the length must match exactly.
fn mol_bytes(b: &[u8]) -> Result<&[u8], i8> {
    let n = read_u32(b, 0).ok_or(ERR_DATA)?;
    if b.len() != 4 + n {
        return Err(ERR_DATA);
    }
    Ok(&b[4..])
}

/// Items of a fixvec (u32 LE count ‖ items of `size` bytes).
fn fixvec(b: &[u8], size: usize) -> Result<&[u8], i8> {
    let n = read_u32(b, 0).ok_or(ERR_DATA)?;
    if n > 1024 || b.len() != 4 + n * size {
        return Err(ERR_DATA);
    }
    Ok(&b[4..])
}

fn parse(b: &[u8]) -> Result<Manifest<'_>, i8> {
    if read_u32(b, 0).ok_or(ERR_DATA)? != b.len() || read_u32(b, 4).ok_or(ERR_DATA)? != 4 * (N_FIELDS + 1) {
        return Err(ERR_DATA);
    }
    let mut f: [&[u8]; N_FIELDS] = [&[]; N_FIELDS];
    let mut prev = 4 * (N_FIELDS + 1);
    for i in 0..N_FIELDS {
        let end = if i + 1 < N_FIELDS { read_u32(b, 8 + 4 * i).ok_or(ERR_DATA)? } else { b.len() };
        if end < prev || end > b.len() {
            return Err(ERR_DATA);
        }
        f[i] = &b[prev..end];
        prev = end;
    }
    if f[0].len() != 1 || f[1].len() != 32 || f[2].len() != 36 || f[13].len() != 1 {
        return Err(ERR_DATA);
    }
    if f[0][0] != 1 {
        return Err(ERR_VERSION_TAG);
    }
    Ok(Manifest {
        code_hash: f[1],
        code_cell: f[2],
        data_hashes: fixvec(f[3], 32)?,
        data_cells: fixvec(f[4], 36)?,
        args: mol_bytes(f[5])?,
        name: mol_bytes(f[6])?,
        version: mol_bytes(f[7])?,
        description: mol_bytes(f[8])?,
        url: mol_bytes(f[9])?,
        source: mol_bytes(f[10])?,
        language: mol_bytes(f[11])?,
        min_csr: mol_bytes(f[12])?,
        deprecated: f[13][0],
    })
}

fn check_url(u: &[u8]) -> Result<(), i8> {
    if u.len() > 128 {
        return Err(ERR_CAP);
    }
    if !u.is_empty() && !u.starts_with(b"https://") {
        return Err(ERR_URL);
    }
    Ok(())
}

/// Empty or `X.Y.Z` (digits, at least one per part).
fn check_min_csr(v: &[u8]) -> Result<(), i8> {
    if v.is_empty() {
        return Ok(());
    }
    let mut parts = 0;
    for p in v.split(|c| *c == b'.') {
        if p.is_empty() || !p.iter().all(u8::is_ascii_digit) {
            return Err(ERR_MIN_CSR);
        }
        parts += 1;
    }
    if parts != 3 {
        return Err(ERR_MIN_CSR);
    }
    Ok(())
}

fn check_fields(m: &Manifest) -> Result<(), i8> {
    let n = m.data_hashes.len() / 32;
    if n != m.data_cells.len() / 36 || n > MAX_DATA_CELLS {
        return Err(ERR_DATA_CELLS);
    }
    if m.args.len() > 64
        || m.name.is_empty()
        || m.name.len() > 48
        || m.version.is_empty()
        || m.version.len() > 16
        || m.description.len() > 512
        || m.language.len() > 16
        || m.min_csr.len() > 16
    {
        return Err(ERR_CAP);
    }
    for t in [m.name, m.version, m.description, m.url, m.source, m.language, m.min_csr] {
        if core::str::from_utf8(t).is_err() {
            return Err(ERR_UTF8);
        }
    }
    check_url(m.url)?;
    check_url(m.source)?;
    check_min_csr(m.min_csr)?;
    if m.deprecated > 1 {
        return Err(ERR_DEPRECATED);
    }
    Ok(())
}

// --- cell deps ----------------------------------------------------------------

fn load_tx_at(buf: &mut [u8], offset: usize) -> Result<(), i8> {
    match syscalls::load_transaction(buf, offset) {
        Ok(n) if n >= buf.len() => Ok(()),
        Err(SysError::LengthNotEnough(n)) if n >= buf.len() => Ok(()),
        Ok(_) => Err(3),
        Err(e) => Err(sys(e)),
    }
}

/// `RawTransaction.cell_deps` of the current transaction (fixvec items, 37 bytes each: out point ‖ dep type).
fn raw_cell_deps() -> Result<Vec<u8>, i8> {
    let mut tx_header = [0u8; 12];
    load_tx_at(&mut tx_header, 0)?;
    let raw = read_u32(&tx_header, 4).ok_or(ERR_DATA)?;
    let mut raw_header = [0u8; 28];
    load_tx_at(&mut raw_header, raw)?;
    let (start, end) = (read_u32(&raw_header, 8).ok_or(ERR_DATA)?, read_u32(&raw_header, 12).ok_or(ERR_DATA)?);
    if end < start + 4 {
        return Err(ERR_DATA);
    }
    let mut deps = alloc::vec![0u8; end - start];
    load_tx_at(&mut deps, raw + start)?;
    let items = &deps[4..];
    if items.len() != read_u32(&deps, 0).ok_or(ERR_DATA)? * CELL_DEP_SIZE {
        return Err(ERR_DATA);
    }
    Ok(items.to_vec())
}

/// The index of a cell dep with this out point, whose data hash is `data_hash`. Only code-type deps count, and no
/// dep group may precede it (a dep group expands to several `Source::CellDep` cells and shifts the indices).
fn find_dep(deps: &[u8], out_point: &[u8], data_hash: &[u8]) -> Result<Option<usize>, i8> {
    for (i, d) in deps.chunks(CELL_DEP_SIZE).enumerate() {
        if d[36] != DEP_TYPE_CODE {
            return Ok(None);
        }
        if &d[..36] == out_point {
            return Ok((cell_hash(i, Source::CellDep, CellField::DataHash)?[..] == *data_hash).then_some(i));
        }
    }
    Ok(None)
}

fn check_creation(m: &Manifest, out_lock_hash: &[u8; 32]) -> Result<(), i8> {
    let deps = raw_cell_deps()?;
    let code = find_dep(&deps, m.code_cell, m.code_hash)?.ok_or(ERR_CODE_DEP)?;
    if cell_hash(code, Source::CellDep, CellField::LockHash)? != *out_lock_hash {
        return Err(ERR_PUBLISHER);
    }
    for (op, h) in m.data_cells.chunks(36).zip(m.data_hashes.chunks(32)) {
        find_dep(&deps, op, h)?.ok_or(ERR_DATA_DEP)?;
    }
    Ok(())
}

fn run() -> Result<(), i8> {
    if args_len()? != 32 {
        return Err(ERR_ARGS_LEN);
    }
    ckb_std::type_id::check_type_id(0, 32).map_err(|_| ERR_TYPE_ID)?;

    let has_in = group_count(Source::GroupInput)? == 1;
    let has_out = group_count(Source::GroupOutput)? == 1;
    if !has_out {
        return Ok(()); // destruction: the lock authorises it
    }
    let out_data = cell_data(0, Source::GroupOutput)?;
    let out = parse(&out_data)?;
    check_fields(&out)?;
    if has_in {
        let in_data = cell_data(0, Source::GroupInput)?;
        let inp = parse(&in_data)?;
        if inp.code_hash != out.code_hash
            || inp.code_cell != out.code_cell
            || inp.data_hashes != out.data_hashes
            || inp.data_cells != out.data_cells
            || inp.args != out.args
        {
            return Err(ERR_UPDATE_FIXED);
        }
        if cell_hash(0, Source::GroupInput, CellField::LockHash)? != cell_hash(0, Source::GroupOutput, CellField::LockHash)? {
            return Err(ERR_UPDATE_LOCK);
        }
    } else {
        check_creation(&out, &cell_hash(0, Source::GroupOutput, CellField::LockHash)?)?;
    }
    Ok(())
}
