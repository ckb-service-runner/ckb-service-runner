//! Keeper jobs: decoding of job cells, discovery, selection by tip rate and `csr jobs plan`.

use crate::chain::{Chain, RawJob, TipInfo};
use crate::config::Config;
use crate::db::{Db, JobCellRow};
use crate::registry::ENABLED;
use serde_json::{json, Value};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq)]
pub enum Trigger {
    OnDemand,
    AtBlock(u64),
    AtEpoch(u64),
    AtTimestamp(u64),
    Every(u64),
}

impl Trigger {
    /// `ondemand`, `at-block:N`, ... as shown in the API.
    pub fn label(&self) -> String {
        match self {
            Trigger::OnDemand => "ondemand".into(),
            Trigger::AtBlock(n) => format!("at-block:{n}"),
            Trigger::AtEpoch(n) => format!("at-epoch:{n}"),
            Trigger::AtTimestamp(n) => format!("at-timestamp:{n}"),
            Trigger::Every(n) => format!("every:{n}"),
        }
    }
}

/// A decoded job cell.
#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub outpoint: String,
    pub code_hash: String,
    pub trigger: Trigger,
    pub tip_shannons: u64,
    pub max_fee_shannons: u64,
    pub max_cycles: u64,
    pub payload: Vec<u8>,
    pub expiry_block: u64,
    pub refund_lock_hash: Vec<u8>,
    pub remaining_runs: u32,
    pub capacity: u64,
    /// Block that created the cell (the base of `Every`).
    pub block: u64,
}

/// Watch header: "CSRW" ‖ version 1 (one byte) ‖ watched out point (36 bytes: tx hash ‖ index u32 LE).
pub const WATCH_MAGIC: &[u8; 4] = b"CSRW";
pub const WATCH_HEADER_LEN: usize = 41;

/// A payload that starts with the watch header for `outpoint`, followed by `inner`.
#[allow(dead_code)] // used by tests and tools that build watched jobs
pub fn watch_payload(outpoint: &[u8; 36], inner: &[u8]) -> Vec<u8> {
    let mut p = WATCH_MAGIC.to_vec();
    p.push(1);
    p.extend_from_slice(outpoint);
    p.extend_from_slice(inner);
    p
}

/// The watched out point; `None` unless magic, version 1 and length >= 41.
pub fn watch_outpoint(payload: &[u8]) -> Option<[u8; 36]> {
    if payload.len() < WATCH_HEADER_LEN || &payload[..4] != WATCH_MAGIC || payload[4] != 1 {
        return None;
    }
    payload[5..WATCH_HEADER_LEN].try_into().ok()
}

/// `0x<tx hash>:<index>`, the form the node and the database use.
pub fn outpoint_str(op: &[u8; 36]) -> String {
    let index = u32::from_le_bytes(op[32..].try_into().expect("4 bytes"));
    format!("0x{}:{index}", hex::encode(&op[..32]))
}

/// Parses `0x<tx hash>:<index>`.
pub fn outpoint_bytes(s: &str) -> Option<[u8; 36]> {
    let (h, i) = s.split_once(':')?;
    let hash = hex::decode(h.trim_start_matches("0x")).ok()?;
    let index: u32 = i.parse().ok()?;
    let mut out = [0u8; 36];
    out[..32].copy_from_slice(hash.as_slice().try_into().ok()?);
    out[32..].copy_from_slice(&index.to_le_bytes());
    Some(out)
}

/// True if the payload has no watch, or the watched cell is known to be consumed.
pub fn watch_due(payload: &[u8], dead: &HashSet<[u8; 36]>) -> bool {
    watch_outpoint(payload).is_none_or(|op| dead.contains(&op))
}

/// `on-cell-consumed:0x<tx hash>:<index>`.
pub fn watch_label(outpoint: &[u8; 36]) -> String {
    format!("on-cell-consumed:{}", outpoint_str(outpoint))
}

impl Job {
    /// The watched out point of an `OnDemand` job (watch headers on other triggers are ignored).
    pub fn watched(&self) -> Option<[u8; 36]> {
        if self.trigger == Trigger::OnDemand {
            watch_outpoint(&self.payload)
        } else {
            None
        }
    }

    /// The trigger label shown in the API: the watch label for watched jobs.
    pub fn trigger_label(&self) -> String {
        self.watched()
            .map_or_else(|| self.trigger.label(), |op| watch_label(&op))
    }

    /// `(tip / 1e8) / (max_cycles / 1e6)`: CKB per million cycles.
    pub fn tip_rate(&self) -> f64 {
        (self.tip_shannons as f64 / 1e8) / (self.max_cycles.max(1) as f64 / 1e6)
    }
}

fn u32_at(d: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(off..off + 4)?.try_into().ok()?))
}

fn u64_at(d: &[u8], off: usize) -> Option<u64> {
    Some(u64::from_le_bytes(d.get(off..off + 8)?.try_into().ok()?))
}

/// Splits a molecule table into its fields (strict: offsets ascending, no trailing bytes).
fn table_fields(d: &[u8], count: usize) -> Option<Vec<&[u8]>> {
    if u32_at(d, 0)? as usize != d.len() {
        return None;
    }
    let first = u32_at(d, 4)? as usize;
    if first != 4 + 4 * count {
        return None;
    }
    let mut offs = Vec::new();
    for i in 0..count {
        offs.push(u32_at(d, 4 + 4 * i)? as usize);
    }
    offs.push(d.len());
    let mut out = Vec::new();
    for w in offs.windows(2) {
        out.push(d.get(w[0]..w[1])?);
    }
    Some(out)
}

fn decode_trigger(d: &[u8]) -> Option<Trigger> {
    let id = u32_at(d, 0)?;
    let inner = d.get(4..)?;
    if id == 0 {
        return (inner == [4, 0, 0, 0]).then_some(Trigger::OnDemand);
    }
    let f = table_fields(inner, 1)?;
    let v = u64_at(f[0], 0).filter(|_| f[0].len() == 8)?;
    match id {
        1 => Some(Trigger::AtBlock(v)),
        2 => Some(Trigger::AtEpoch(v)),
        3 => Some(Trigger::AtTimestamp(v)),
        4 => Some(Trigger::Every(v)),
        _ => None,
    }
}

/// Decodes the molecule `Job` of a job cell; `None` if it is malformed.
pub fn decode(raw: &RawJob) -> Option<Job> {
    let f = table_fields(&raw.data, 8)?;
    let payload_len = u32_at(f[4], 0)? as usize;
    if f[4].len() != 4 + payload_len || f[1].len() != 8 || f[6].len() != 32 || f[7].len() != 4 {
        return None;
    }
    Some(Job {
        outpoint: raw.outpoint.clone(),
        code_hash: raw.service_code_hash.clone(),
        trigger: decode_trigger(f[0])?,
        tip_shannons: u64_at(f[1], 0)?,
        max_fee_shannons: u64_at(f[2], 0).filter(|_| f[2].len() == 8)?,
        max_cycles: u64_at(f[3], 0).filter(|_| f[3].len() == 8)?,
        payload: f[4][4..].to_vec(),
        expiry_block: u64_at(f[5], 0).filter(|_| f[5].len() == 8)?,
        refund_lock_hash: f[6].to_vec(),
        remaining_runs: u32_at(f[7], 0)?,
        capacity: raw.capacity,
        block: raw.block,
    })
}

/// `number + index / length` of a packed epoch as `(num, den)` parts, cross-multiplied by the caller.
fn epoch_ge(tip: u64, want: u64) -> bool {
    let parts = |e: u64| {
        let (n, i, l) = (e & 0xff_ffff, (e >> 24) & 0xffff, (e >> 40) & 0xffff);
        if l == 0 {
            (n as u128, 0u128, 1u128)
        } else {
            (n as u128, i as u128, l as u128)
        }
    };
    let (tn, ti, tl) = parts(tip);
    let (wn, wi, wl) = parts(want);
    // tn + ti/tl >= wn + wi/wl
    (tn * tl + ti) * wl >= (wn * wl + wi) * tl
}

/// Does the trigger's `since` condition hold at the current tip?
pub fn trigger_due(job: &Job, tip: &TipInfo) -> bool {
    match job.trigger {
        Trigger::OnDemand => true,
        Trigger::AtBlock(n) => tip.number >= n,
        Trigger::AtEpoch(e) => epoch_ge(tip.epoch, e),
        Trigger::AtTimestamp(t) => tip.median_ms / 1000 * 1000 >= t,
        Trigger::Every(k) => tip.number >= job.block.saturating_add(k),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Exclusion {
    NotAllowlisted,
    Expired,
    TriggerNotDue,
    AboveMaxCycles,
    BelowMinRate,
    WaitingForCell,
}

impl Exclusion {
    pub fn as_str(self) -> &'static str {
        match self {
            Exclusion::NotAllowlisted => "not_allowlisted",
            Exclusion::Expired => "expired",
            Exclusion::TriggerNotDue => "trigger_not_due",
            Exclusion::AboveMaxCycles => "above_max_cycles",
            Exclusion::BelowMinRate => "below_min_rate",
            Exclusion::WaitingForCell => "waiting_for_cell",
        }
    }
}

pub struct Plan {
    pub tip: u64,
    pub capacity: u32,
    pub selected: Vec<Job>,
    pub excluded: Vec<(Job, Exclusion)>,
}

impl Plan {
    pub fn to_json(&self) -> Value {
        json!({
            "tip": self.tip,
            "capacity": self.capacity,
            "selected": self.selected.iter().map(|j| json!({
                "outpoint": j.outpoint,
                "code_hash": j.code_hash,
                "tip_shannons": j.tip_shannons,
                "max_cycles": j.max_cycles,
                "tip_rate": j.tip_rate(),
            })).collect::<Vec<_>>(),
            "excluded": self.excluded.iter().map(|(j, r)| json!({
                "outpoint": j.outpoint,
                "code_hash": j.code_hash,
                "reason": r.as_str(),
            })).collect::<Vec<_>>(),
        })
    }
}

/// Services that are `approved_enabled` in the database **and** in `runtime.allowed_services`.
pub fn allowlist(db: &Db, cfg: &Config) -> Result<Vec<String>, String> {
    let allowed: Vec<String> = cfg
        .runtime
        .allowed_services
        .iter()
        .map(|a| a.trim_start_matches("0x").to_ascii_lowercase())
        .collect();
    Ok(db
        .services()?
        .into_iter()
        .filter(|s| s.status == ENABLED)
        .map(|s| s.code_hash.to_ascii_lowercase())
        .filter(|h| allowed.contains(&h.trim_start_matches("0x").to_string()))
        .collect())
}

/// Applies the exclusion rules (first match wins) and orders the rest by tip rate.
pub fn select(
    cfg: &Config,
    allow: &[String],
    tip: &TipInfo,
    dead: &HashSet<[u8; 36]>,
    jobs: Vec<Job>,
) -> Plan {
    let mut selected = Vec::new();
    let mut excluded = Vec::new();
    for j in jobs {
        let reason = if !allow.contains(&j.code_hash.to_ascii_lowercase()) {
            Some(Exclusion::NotAllowlisted)
        } else if tip.number >= j.expiry_block {
            Some(Exclusion::Expired)
        } else if !trigger_due(&j, tip) {
            Some(Exclusion::TriggerNotDue)
        } else if j.watched().is_some_and(|_| !watch_due(&j.payload, dead)) {
            Some(Exclusion::WaitingForCell)
        } else if j.max_cycles > cfg.limits.max_cycles_per_job {
            Some(Exclusion::AboveMaxCycles)
        } else if j.tip_rate() < cfg.runtime.min_ckb_per_mcycle {
            Some(Exclusion::BelowMinRate)
        } else {
            None
        };
        match reason {
            Some(r) => excluded.push((j, r)),
            None => selected.push(j),
        }
    }
    selected.sort_by(|a, b| {
        b.tip_rate()
            .total_cmp(&a.tip_rate())
            .then_with(|| a.outpoint.cmp(&b.outpoint))
    });
    excluded.sort_by(|a, b| a.0.outpoint.cmp(&b.0.outpoint));
    Plan {
        tip: tip.number,
        capacity: cfg.limits.max_concurrent_jobs,
        selected,
        excluded,
    }
}

/// Reads the live job cells and the tip, and computes the plan.
pub async fn plan_now(chain: &Chain, db: &Db, cfg: &Config) -> Result<Plan, String> {
    let tip = chain.tip_info().await?;
    let jobs: Vec<Job> = chain.job_cells().await?.iter().filter_map(decode).collect();
    Ok(select(cfg, &allowlist(db, cfg)?, &tip, &db.watched_dead()?, jobs))
}

/// Stores a plan as the set of discovered jobs (eligible ones as `discovered`).
pub fn store(db: &Db, plan: &Plan) -> Result<(), String> {
    let row = |j: &Job, reason: Option<Exclusion>| JobCellRow {
        outpoint: j.outpoint.clone(),
        code_hash: j.code_hash.clone(),
        tip_shannons: j.tip_shannons,
        max_cycles: j.max_cycles,
        trigger: j.trigger_label(),
        state: if reason.is_some() {
            "excluded"
        } else {
            "discovered"
        }
        .into(),
        reason: reason.map(|r| r.as_str().to_string()),
    };
    let rows: Vec<JobCellRow> = plan
        .selected
        .iter()
        .map(|j| row(j, None))
        .chain(plan.excluded.iter().map(|(j, r)| row(j, Some(*r))))
        .collect();
    db.replace_job_cells(&rows, plan.tip)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tip(n: u64) -> TipInfo {
        TipInfo {
            number: n,
            epoch: 5,
            median_ms: 10_500,
        }
    }

    fn job(op: &str, tip_ckb: u64, cycles: u64, trigger: Trigger) -> Job {
        Job {
            outpoint: op.into(),
            code_hash: "0xaa".into(),
            trigger,
            tip_shannons: tip_ckb * 100_000_000,
            max_fee_shannons: 0,
            max_cycles: cycles,
            payload: vec![],
            expiry_block: 1000,
            refund_lock_hash: vec![0; 32],
            remaining_runs: 1,
            capacity: 0,
            block: 10,
        }
    }

    fn table(fields: &[Vec<u8>]) -> Vec<u8> {
        let header = 4 + 4 * fields.len();
        let mut out = ((header + fields.iter().map(Vec::len).sum::<usize>()) as u32)
            .to_le_bytes()
            .to_vec();
        let mut off = header;
        for f in fields {
            out.extend_from_slice(&(off as u32).to_le_bytes());
            off += f.len();
        }
        fields.iter().for_each(|f| out.extend_from_slice(f));
        out
    }

    #[test]
    fn decodes_job_data() {
        let mut trig = 4u32.to_le_bytes().to_vec();
        trig.extend(table(&[7u64.to_le_bytes().to_vec()]));
        let data = table(&[
            trig,
            5u64.to_le_bytes().to_vec(),
            6u64.to_le_bytes().to_vec(),
            7u64.to_le_bytes().to_vec(),
            [2u32.to_le_bytes().to_vec(), vec![1, 2]].concat(),
            9u64.to_le_bytes().to_vec(),
            vec![3; 32],
            4u32.to_le_bytes().to_vec(),
        ]);
        let raw = RawJob {
            outpoint: "0x01:0".into(),
            service_code_hash: "0xaa".into(),
            capacity: 1,
            block: 2,
            data: data.clone(),
        };
        let j = decode(&raw).unwrap();
        assert_eq!(j.trigger, Trigger::Every(7));
        assert_eq!(
            (j.tip_shannons, j.max_fee_shannons, j.max_cycles),
            (5, 6, 7)
        );
        assert_eq!(j.payload, vec![1, 2]);
        assert_eq!((j.expiry_block, j.remaining_runs), (9, 4));
        let mut bad = data;
        bad.push(0);
        assert!(decode(&RawJob { data: bad, ..raw }).is_none());
    }

    #[test]
    fn triggers() {
        let t = tip(100);
        assert!(trigger_due(&job("a", 1, 1, Trigger::OnDemand), &t));
        assert!(trigger_due(&job("a", 1, 1, Trigger::AtBlock(100)), &t));
        assert!(!trigger_due(&job("a", 1, 1, Trigger::AtBlock(101)), &t));
        assert!(trigger_due(&job("a", 1, 1, Trigger::Every(90)), &t));
        assert!(!trigger_due(&job("a", 1, 1, Trigger::Every(91)), &t));
        assert!(trigger_due(
            &job("a", 1, 1, Trigger::AtTimestamp(10_000)),
            &t
        ));
        assert!(!trigger_due(
            &job("a", 1, 1, Trigger::AtTimestamp(11_000)),
            &t
        ));
        assert!(trigger_due(&job("a", 1, 1, Trigger::AtEpoch(5)), &t));
        assert!(!trigger_due(&job("a", 1, 1, Trigger::AtEpoch(6)), &t));
    }

    #[test]
    fn orders_by_rate_then_outpoint_and_excludes_in_order() {
        let mut cfg = Config::default();
        cfg.runtime.min_ckb_per_mcycle = 0.5;
        cfg.limits.max_cycles_per_job = 70_000_000;
        let allow = vec!["0xaa".to_string()];
        let mut foreign = job("f", 100, 1_000_000, Trigger::OnDemand);
        foreign.code_hash = "0xbb".into();
        let mut expired = job("e", 100, 1_000_000, Trigger::AtBlock(5000));
        expired.expiry_block = 50;
        let jobs = vec![
            job("b", 10, 10_000_000, Trigger::OnDemand),
            job("a", 10, 10_000_000, Trigger::OnDemand),
            job("c", 50, 10_000_000, Trigger::OnDemand),
            job("low", 1, 10_000_000, Trigger::OnDemand),
            job("big", 100, 100_000_000, Trigger::OnDemand),
            job("nd", 10, 1_000_000, Trigger::AtBlock(5000)),
            foreign,
            expired,
        ];
        let p = select(&cfg, &allow, &tip(100), &HashSet::new(), jobs);
        let sel: Vec<_> = p.selected.iter().map(|j| j.outpoint.as_str()).collect();
        assert_eq!(sel, ["c", "a", "b"]);
        let why: std::collections::HashMap<_, _> = p
            .excluded
            .iter()
            .map(|(j, r)| (j.outpoint.as_str(), r.as_str()))
            .collect();
        assert_eq!(why["low"], "below_min_rate");
        assert_eq!(why["big"], "above_max_cycles");
        assert_eq!(why["nd"], "trigger_not_due");
        assert_eq!(why["f"], "not_allowlisted");
        assert_eq!(why["e"], "expired");
    }
}

#[cfg(test)]
#[path = "../../acceptance/aiworker-csr-cell-consumed-trigger/watch_tests.rs"]
mod aiworker_watch_tests;
