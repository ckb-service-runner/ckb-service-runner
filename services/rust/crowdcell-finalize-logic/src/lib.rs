//! Pure logic of the CrowdCell finalize service: payload, campaign data, decision, skeleton bytes.
#![cfg_attr(not(test), no_std)]
extern crate alloc;
use alloc::vec::Vec;

pub const MAX_SCRIPT_LEN: usize = 4096;
pub const STATUS_OFFSET: usize = 56;
pub const CAMPAIGN_MIN_SIZE: usize = 65;

#[derive(Debug, PartialEq, Eq)]
pub enum PayloadError {
    TooShort,
    ScriptTooLarge,
    LengthMismatch,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Payload {
    pub type_script: Vec<u8>,
    pub campaign_code: [u8; 36],
    pub lock_code: [u8; 36],
}

pub fn parse_payload(b: &[u8]) -> Result<Payload, PayloadError> {
    if b.len() < 4 + 72 {
        return Err(PayloadError::TooShort);
    }
    let n = u32::from_le_bytes(b[0..4].try_into().unwrap()) as usize;
    if n > MAX_SCRIPT_LEN {
        return Err(PayloadError::ScriptTooLarge);
    }
    if b.len() != 4 + n + 72 {
        return Err(PayloadError::LengthMismatch);
    }
    let mut campaign_code = [0u8; 36];
    let mut lock_code = [0u8; 36];
    campaign_code.copy_from_slice(&b[4 + n..4 + n + 36]);
    lock_code.copy_from_slice(&b[4 + n + 36..]);
    Ok(Payload { type_script: b[4..4 + n].to_vec(), campaign_code, lock_code })
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct Campaign {
    pub goal: u64,
    pub deadline: u64,
    pub total: u64,
    pub status: u8,
}

pub fn parse_campaign(d: &[u8]) -> Option<Campaign> {
    if d.len() < CAMPAIGN_MIN_SIZE {
        return None;
    }
    Some(Campaign {
        goal: u64::from_le_bytes(d[32..40].try_into().unwrap()),
        deadline: u64::from_le_bytes(d[40..48].try_into().unwrap()),
        total: u64::from_le_bytes(d[48..56].try_into().unwrap()),
        status: d[STATUS_OFFSET],
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Finalize { new_status: u8 },
    AlreadyFinal,
    NotYet,
    Malformed,
}

pub fn decide(c: &Campaign, tip: u64) -> Outcome {
    match c.status {
        0 => {}
        1 | 2 => return Outcome::AlreadyFinal,
        _ => return Outcome::Malformed,
    }
    if c.deadline == 0 {
        return Outcome::Malformed;
    }
    if tip < c.deadline {
        return Outcome::NotYet;
    }
    Outcome::Finalize { new_status: if c.total >= c.goal { 1 } else { 2 } }
}

pub fn finalized_data(old: &[u8], new_status: u8) -> Vec<u8> {
    let mut v = old.to_vec();
    v[STATUS_OFFSET] = new_status;
    v
}

#[derive(Debug, PartialEq, Eq)]
pub enum BuildError {
    BadOutput,
    BadDeadline,
}

fn fixvec(items: &[&[u8]]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&(items.len() as u32).to_le_bytes());
    for i in items {
        v.extend_from_slice(i);
    }
    v
}

fn dynvec(items: &[Vec<u8>]) -> Vec<u8> {
    let header = 4 + 4 * items.len();
    let total = header + items.iter().map(|i| i.len()).sum::<usize>();
    let mut v = Vec::new();
    v.extend_from_slice(&(total as u32).to_le_bytes());
    let mut off = header;
    for i in items {
        v.extend_from_slice(&(off as u32).to_le_bytes());
        off += i.len();
    }
    for i in items {
        v.extend_from_slice(i);
    }
    v
}

fn table(fields: &[Vec<u8>]) -> Vec<u8> {
    let header = 4 + 4 * fields.len();
    let total = header + fields.iter().map(|f| f.len()).sum::<usize>();
    let mut v = Vec::new();
    v.extend_from_slice(&(total as u32).to_le_bytes());
    let mut off = header;
    for f in fields {
        v.extend_from_slice(&(off as u32).to_le_bytes());
        off += f.len();
    }
    for f in fields {
        v.extend_from_slice(f);
    }
    v
}

fn bytes(b: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&(b.len() as u32).to_le_bytes());
    v.extend_from_slice(b);
    v
}

/// Molecule `Transaction`: the campaign cell as the only input (`since` = deadline), the same cell
/// output (capacity, lock, type) with `new_data`, the two code cells as deps, one empty witness.
pub fn build_skeleton(
    p: &Payload,
    cell_out_point: &[u8; 36],
    cell_output: &[u8],
    new_data: &[u8],
    deadline: u64,
) -> Result<Vec<u8>, BuildError> {
    if deadline >> 56 != 0 {
        return Err(BuildError::BadDeadline);
    }
    if cell_output.len() < 16
        || u32::from_le_bytes(cell_output[0..4].try_into().unwrap()) as usize != cell_output.len()
    {
        return Err(BuildError::BadOutput);
    }
    let mut dep_a = p.campaign_code.to_vec();
    dep_a.push(0);
    let mut dep_b = p.lock_code.to_vec();
    dep_b.push(0);
    let cell_deps = fixvec(&[&dep_a, &dep_b]);
    let header_deps = fixvec(&[]);
    let mut input = deadline.to_le_bytes().to_vec();
    input.extend_from_slice(cell_out_point);
    let inputs = fixvec(&[&input]);
    let outputs = dynvec(&[cell_output.to_vec()]);
    let outputs_data = dynvec(&[bytes(new_data)]);
    let raw = table(&[0u32.to_le_bytes().to_vec(), cell_deps, header_deps, inputs, outputs, outputs_data]);
    let witnesses = dynvec(&[bytes(&[])]);
    Ok(table(&[raw, witnesses]))
}

#[cfg(test)]
#[path = "../../../../acceptance/aiworker-csr-071-crowdcell/logic_tests.rs"]
mod aiworker_071_logic_tests;
