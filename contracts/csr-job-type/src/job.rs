//! Strict reader of the molecule `Job` table (`contracts/schemas/csr.mol`), shared with `csr-job-lock`.

use crate::mol;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    OnDemand,
    AtBlock(u64),
    /// Packed epoch `number (24 bit) ‖ index (16 bit) ‖ length (16 bit)` as in CKB `since`.
    AtEpoch(u64),
    /// Milliseconds.
    AtTimestamp(u64),
    /// Blocks between runs.
    Every(u64),
}

pub struct Job {
    pub trigger: Trigger,
    pub tip_shannons: u64,
    pub max_fee_shannons: u64,
    pub expiry_block: u64,
    pub refund_lock_hash: [u8; 32],
    pub remaining_runs: u32,
}

fn u64_le(b: &[u8]) -> Option<u64> {
    Some(u64::from_le_bytes(*mol::fixed::<8>(b)?))
}

/// A table with one `Uint64` field.
fn single_u64(b: &[u8]) -> Option<u64> {
    let [v] = mol::table::<1>(b)?;
    u64_le(v)
}

fn trigger(b: &[u8]) -> Option<Trigger> {
    let id = u32::from_le_bytes(b.get(0..4)?.try_into().ok()?);
    let item = &b[4..];
    Some(match id {
        // empty table: just its total size
        0 if item == 4u32.to_le_bytes() => Trigger::OnDemand,
        1 => Trigger::AtBlock(single_u64(item)?),
        2 => Trigger::AtEpoch(single_u64(item)?),
        3 => Trigger::AtTimestamp(single_u64(item)?),
        4 => Trigger::Every(single_u64(item)?),
        _ => return None,
    })
}

/// Parses `molecule(Job)`; `None` if the data is not exactly a `Job` table.
/// `remaining_runs` is the last field and fixed-size, so it is always the last 4 bytes of the data.
pub fn parse(d: &[u8]) -> Option<Job> {
    let [trig, tip, max_fee, max_cycles, payload, expiry, refund, runs] = mol::table::<8>(d)?;
    u64_le(max_cycles)?;
    mol::bytes(payload)?;
    Some(Job {
        trigger: trigger(trig)?,
        tip_shannons: u64_le(tip)?,
        max_fee_shannons: u64_le(max_fee)?,
        expiry_block: u64_le(expiry)?,
        refund_lock_hash: *mol::fixed::<32>(refund)?,
        remaining_runs: u32::from_le_bytes(*mol::fixed::<4>(runs)?),
    })
}

/// Whether `out` is `inp` with only `remaining_runs` decremented by one (both must already parse).
pub fn is_next_run(inp: &[u8], in_runs: u32, out: &[u8], out_runs: u32) -> bool {
    inp.len() == out.len()
        && inp.len() >= 4
        && inp[..inp.len() - 4] == out[..out.len() - 4]
        && in_runs.checked_sub(1) == Some(out_runs)
}
