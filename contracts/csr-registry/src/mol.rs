//! Minimal molecule readers (no `bytes` crate): tables, `Bytes` vectors and the Script table.

fn read_u32(b: &[u8], at: usize) -> Option<usize> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?) as usize)
}

/// Splits a molecule table into its `N` field slices. The table must be complete (total size = slice length)
/// and have exactly `N` fields with monotonic offsets.
pub fn table<const N: usize>(b: &[u8]) -> Option<[&[u8]; N]> {
    if read_u32(b, 0)? != b.len() {
        return None;
    }
    let first = read_u32(b, 4)?;
    if first != 4 * (N + 1) {
        return None;
    }
    let mut offs = [0usize; N];
    for (i, o) in offs.iter_mut().enumerate() {
        *o = read_u32(b, 4 + 4 * i)?;
    }
    let mut out: [&[u8]; N] = [&[]; N];
    for i in 0..N {
        let end = if i + 1 < N { offs[i + 1] } else { b.len() };
        out[i] = b.get(offs[i]..end)?;
    }
    Some(out)
}

/// Content of a molecule `Bytes` (u32 LE length ‖ bytes); the length must match the slice exactly.
pub fn bytes(b: &[u8]) -> Option<&[u8]> {
    let n = read_u32(b, 0)?;
    if b.len() != 4 + n {
        return None;
    }
    Some(&b[4..])
}

/// A fixed-size field of exactly `N` bytes.
pub fn fixed<const N: usize>(b: &[u8]) -> Option<&[u8; N]> {
    b.try_into().ok()
}

/// `(code_hash, hash_type, args)` of a molecule Script.
pub fn script(b: &[u8]) -> Option<(&[u8; 32], u8, &[u8])> {
    let [code_hash, hash_type, args] = table::<3>(b)?;
    Some((fixed::<32>(code_hash)?, *fixed::<1>(hash_type)?.first()?, bytes(args)?))
}
