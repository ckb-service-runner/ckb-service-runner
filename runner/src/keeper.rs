//! Keeper job execution: the run lifecycle, sandbox execution, the run transaction, submission and
//! settlement tracking (`docs/spec/CSR-SPEC-v0.2.md` §10.3).
//!
//! Lifecycle of a run (one per job cell outpoint):
//! `Discovered → Queued → Scheduled → Running ⇄ Paused → Completed → Submitting → Settled | Lost`,
//! side exits `Failed` (execution or transaction building failed), `Rejected` (the node refused
//! the transaction while the job cell is still live) and `Expired` (the job reached its expiry
//! before it ran).

use crate::app::AppState;
use crate::chain::{Chain, RunJob};
use crate::config::Config;
use crate::db::{JobRun, Settlement};
use crate::jobs::{self, Job, Plan, Trigger};
use crate::runtime::host::{ChainAccess, HostState, MAX_QUERY_RESULTS};
use crate::runtime::vtx::{ckb_hash, VirtualTx};
use crate::runtime::{self, Limits, Status};
use crate::storage::Store;
use ckb_types::{packed, prelude::*};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub const DISCOVERED: &str = "Discovered";
pub const QUEUED: &str = "Queued";
pub const SCHEDULED: &str = "Scheduled";
pub const RUNNING: &str = "Running";
pub const PAUSED: &str = "Paused";
pub const COMPLETED: &str = "Completed";
pub const SUBMITTING: &str = "Submitting";
pub const SETTLED: &str = "Settled";
pub const LOST: &str = "Lost";
pub const FAILED: &str = "Failed";
pub const REJECTED: &str = "Rejected";
pub const EXPIRED: &str = "Expired";

/// Runs that wait for execution.
const PENDING: [&str; 4] = [DISCOVERED, QUEUED, SCHEDULED, PAUSED];
/// Runs that block a new run of the same outpoint.
const BLOCKING: [&str; 10] = [
    DISCOVERED, QUEUED, SCHEDULED, PAUSED, RUNNING, COMPLETED, SUBMITTING, SETTLED, LOST, EXPIRED,
];
/// A job cell whose runs failed or were rejected this often is not tried again.
const MAX_ATTEMPTS: usize = 3;
/// A submitted transaction the node no longer knows is given up after this many blocks.
const UNKNOWN_AFTER_BLOCKS: u64 = 30;
/// A transaction still pending after this many blocks is given up.
const PENDING_AFTER_BLOCKS: u64 = 500;

/// The `since` of the job input that proves the trigger (`job-cell-standard.md` §2).
pub fn since_for(trigger: &Trigger) -> u64 {
    match *trigger {
        Trigger::OnDemand => 0,
        Trigger::AtBlock(n) => n,
        Trigger::AtEpoch(e) => {
            // A length of 0 means the whole epoch number; CKB wants a well-formed fraction.
            let e = if (e >> 40) & 0xffff == 0 {
                (e & 0xff_ffff) | (1 << 40)
            } else {
                e
            };
            0x2000_0000_0000_0000 | (e & 0x00ff_ffff_ffff_ffff)
        }
        Trigger::AtTimestamp(ms) => 0x4000_0000_0000_0000 | ms.div_ceil(1000),
        Trigger::Every(k) => 0x8000_0000_0000_0000 | k,
    }
}

/// Node access for the `chain_read` / `cells_query` calls from the VM thread.
struct NodeAccess {
    chain: Arc<Chain>,
    handle: tokio::runtime::Handle,
}

impl ChainAccess for NodeAccess {
    fn read(&self, kind: u64, key: &[u8]) -> Result<Option<Vec<u8>>, String> {
        self.handle.block_on(self.chain.read_for_service(kind, key))
    }

    fn query(&self, script: &[u8], script_type: u64) -> Result<Option<Vec<[u8; 36]>>, String> {
        let Ok(s) = packed::Script::from_slice(script) else {
            return Ok(None);
        };
        let ops = self.handle.block_on(self.chain.query_for_service(
            &s,
            script_type,
            MAX_QUERY_RESULTS,
        ))?;
        Ok(Some(
            ops.iter()
                .map(|o| o.as_slice().try_into().expect("36-byte out point"))
                .collect(),
        ))
    }
}

/// What the sandbox produced for one run.
struct Executed {
    run_id: i64,
    job: Job,
    outcome: Result<(runtime::Outcome, Option<Vec<u8>>), String>,
}

pub struct Keeper {
    chain: Arc<Chain>,
    /// Service code by code hash (verified against the hash).
    code: HashMap<String, Vec<u8>>,
}

fn hex_of(b: &[u8]) -> String {
    format!("0x{}", hex::encode(b))
}

fn new_run(j: &Job) -> JobRun {
    JobRun {
        id: 0,
        outpoint: j.outpoint.clone(),
        code_hash: j.code_hash.clone(),
        trigger: j.trigger.label(),
        tip_shannons: j.tip_shannons,
        max_cycles: j.max_cycles,
        tip_rate: j.tip_rate(),
        state: DISCOVERED.into(),
        cycles: None,
        result_tx: None,
        result_hex: None,
        error: None,
        logs: vec![],
        submit_block: None,
    }
}

impl Keeper {
    pub fn new(chain: Chain) -> Keeper {
        Keeper {
            chain: Arc::new(chain),
            code: HashMap::new(),
        }
    }

    pub async fn tip(&self) -> Result<u64, String> {
        Ok(self.chain.tip_info().await?.number)
    }

    /// One scheduler step at a new tip. `runtime_on` = runtime state `on`; `hold` = why new runs
    /// must not start (R2, "node always wins").
    pub async fn tick(
        &mut self,
        st: &Arc<AppState>,
        cfg: &Config,
        runtime_on: bool,
        hold: Option<String>,
    ) -> Result<String, String> {
        let tip = self.chain.tip_info().await?;
        self.track(st, tip.number).await?;
        if !runtime_on {
            return Ok(format!("block {}: runtime not on", tip.number));
        }
        let raws = self.chain.job_cells().await?;
        let data: HashMap<String, Vec<u8>> = raws
            .iter()
            .map(|r| (r.outpoint.clone(), r.data.clone()))
            .collect();
        let decoded: Vec<Job> = raws.iter().filter_map(jobs::decode).collect();
        let plan = jobs::select(cfg, &jobs::allowlist(&st.db, cfg)?, &tip, decoded);
        jobs::store(&st.db, &plan)?;
        self.reconcile(st, &plan, &data)?;
        let summary = format!(
            "block {}: {} eligible job(s), {} excluded",
            tip.number,
            plan.selected.len(),
            plan.excluded.len()
        );
        // One batch at a time: its transactions chain on each other's runner wallet output.
        if !st.db.runs_in(&[RUNNING, COMPLETED, SUBMITTING])?.is_empty() {
            return Ok(summary);
        }
        let batch = self.queue(st, &plan)?;
        if batch.is_empty() {
            return Ok(summary);
        }
        if let Some(why) = hold {
            for (r, _) in &batch {
                if r.state != PAUSED {
                    st.db.set_run_state(r.id, PAUSED, Some(&why))?;
                }
            }
            return Ok(format!("{summary}; held: {why}"));
        }
        // Without a registration paying into the runner wallet, or a wallet cell, nothing can be
        // settled: do not run.
        let ctx = self.chain.run_context().await?;
        let executed = self.execute(st, cfg, batch, &tip).await?;
        self.submit(st, ctx, executed, &data, tip.number).await?;
        Ok(summary)
    }

    /// Creates runs for newly selected jobs and returns the next batch (best tip rate first).
    fn queue(&self, st: &AppState, plan: &Plan) -> Result<Vec<(JobRun, Job)>, String> {
        let cfg_cap = plan.capacity as usize;
        let runs = st.db.runs()?;
        let mut batch = Vec::new();
        for j in &plan.selected {
            let mine: Vec<&JobRun> = runs.iter().filter(|r| r.outpoint == j.outpoint).collect();
            let pending = mine.iter().find(|r| PENDING.contains(&r.state.as_str()));
            let run = match pending {
                Some(r) => (*r).clone(),
                None => {
                    if mine.iter().any(|r| BLOCKING.contains(&r.state.as_str()))
                        || mine.len() >= MAX_ATTEMPTS
                    {
                        continue;
                    }
                    let mut r = new_run(j);
                    r.id = st.db.add_run(&r)?;
                    st.db.set_run_state(r.id, QUEUED, None)?;
                    r.state = QUEUED.into();
                    r
                }
            };
            if batch.len() < cfg_cap {
                batch.push((run, j.clone()));
            }
        }
        Ok(batch)
    }

    /// Waiting runs whose job cell is gone become `Lost`; expired ones `Expired`.
    fn reconcile(
        &self,
        st: &AppState,
        plan: &Plan,
        live: &HashMap<String, Vec<u8>>,
    ) -> Result<(), String> {
        let expired: HashSet<&str> = plan
            .excluded
            .iter()
            .filter(|(_, r)| *r == jobs::Exclusion::Expired)
            .map(|(j, _)| j.outpoint.as_str())
            .collect();
        for r in st.db.runs_in(&PENDING)? {
            if !live.contains_key(&r.outpoint) {
                st.db
                    .set_run_state(r.id, LOST, Some("the job cell was spent by someone else"))?;
            } else if expired.contains(r.outpoint.as_str()) {
                st.db
                    .set_run_state(r.id, EXPIRED, Some("the job reached its expiry block"))?;
            }
        }
        Ok(())
    }

    /// The service code from its code cell (cached, checked against the code hash).
    async fn code(&mut self, st: &AppState, code_hash: &str) -> Result<Vec<u8>, String> {
        if let Some(c) = self.code.get(code_hash) {
            return Ok(c.clone());
        }
        let op =
            st.db.service(code_hash)?.and_then(|s| s.outpoint).ok_or(
                "the service's code cell is unknown (add it with `csr service add-onchain`)",
            )?;
        let (_, data) = self
            .chain
            .live_cell(&op)
            .await?
            .ok_or_else(|| format!("the code cell {op} is not live"))?;
        if hex_of(&ckb_hash(&data)) != code_hash {
            return Err(format!("the code cell {op} does not hold {code_hash}"));
        }
        self.code.insert(code_hash.to_string(), data.clone());
        Ok(data)
    }

    /// Runs the batch in the sandbox: different services in parallel, runs of one service one
    /// after another (they share its storage).
    async fn execute(
        &mut self,
        st: &Arc<AppState>,
        cfg: &Config,
        batch: Vec<(JobRun, Job)>,
        tip: &crate::chain::TipInfo,
    ) -> Result<Vec<Executed>, String> {
        let mut groups: Vec<(String, Vec<(i64, Job)>)> = Vec::new();
        for (r, j) in batch {
            if r.state == QUEUED {
                st.db.set_run_state(r.id, SCHEDULED, None)?;
            }
            match groups.iter_mut().find(|g| g.0 == j.code_hash) {
                Some(g) => g.1.push((r.id, j)),
                None => groups.push((j.code_hash.clone(), vec![(r.id, j)])),
            }
        }
        let mut out = Vec::new();
        let mut handles = Vec::new();
        let storage_root = st.net_dir.join(&cfg.storage.path);
        let block_info = [tip.number, tip.epoch, tip.median_ms];
        for (code_hash, runs) in groups {
            let elf = match self.code(st, &code_hash).await {
                Ok(e) => e,
                Err(e) => {
                    for (id, job) in runs {
                        st.db.set_run_state(id, RUNNING, None)?;
                        out.push(Executed {
                            run_id: id,
                            job,
                            outcome: Err(e.clone()),
                        });
                    }
                    continue;
                }
            };
            for (id, _) in &runs {
                st.db.set_run_state(*id, RUNNING, None)?;
            }
            let hash: [u8; 32] = hex::decode(code_hash.trim_start_matches("0x"))
                .ok()
                .and_then(|h| h.try_into().ok())
                .ok_or("bad code hash")?;
            let store_args = (
                storage_root.clone(),
                st.storage_key,
                cfg.storage.per_service_quota_mb,
                cfg.storage.total_quota_mb,
            );
            let key = st.service_key;
            let chain = self.chain.clone();
            let handle = tokio::runtime::Handle::current();
            let limits = cfg.limits.clone();
            handles.push(tokio::task::spawn_blocking(move || {
                runs.into_iter()
                    .map(|(id, job)| {
                        let mut vtx =
                            VirtualTx::new(elf.clone(), vec![], vec![], job.payload.clone());
                        vtx.input.capacity = job.capacity;
                        if let Ok((h, i)) = crate::chain::parse_out_point(&job.outpoint) {
                            if let Ok(Ok(h)) =
                                hex::decode(h.trim_start_matches("0x")).map(<[u8; 32]>::try_from)
                            {
                                vtx.input_out_point = (h, i);
                            }
                        }
                        let store = Store::new(
                            &store_args.0,
                            &hex::encode(hash),
                            store_args.1,
                            store_args.2,
                            store_args.3,
                        );
                        let state = HostState::new(vtx, block_info)
                            .with_store(store)
                            .with_signer(key, hash)
                            .with_chain(Box::new(NodeAccess {
                                chain: chain.clone(),
                                handle: handle.clone(),
                            }));
                        let lim = Limits {
                            max_cycles: job.max_cycles.min(limits.max_cycles_per_job),
                            max_memory_mb: limits.max_vm_memory_mb,
                            max_wall_seconds: limits.max_job_wall_seconds,
                        };
                        let outcome = runtime::run(&elf, state, lim).map(|(o, s)| (o, s.skeleton));
                        Executed {
                            run_id: id,
                            job,
                            outcome,
                        }
                    })
                    .collect::<Vec<_>>()
            }));
        }
        for h in handles {
            out.extend(h.await.map_err(|e| format!("sandbox thread failed: {e}"))?);
        }
        Ok(out)
    }

    /// The refund lock script of a job (cached in the settings table by its hash).
    async fn refund_lock(
        &self,
        st: &AppState,
        job: &Job,
    ) -> Result<Option<packed::Script>, String> {
        let key = format!("refund_lock:{}", hex::encode(&job.refund_lock_hash));
        if let Some(h) = st.db.get_setting(&key)? {
            if let Some(s) = hex::decode(h)
                .ok()
                .and_then(|b| packed::Script::from_slice(&b).ok())
            {
                return Ok(Some(s));
            }
        }
        let found = self
            .chain
            .find_lock(&job.outpoint, &job.refund_lock_hash)
            .await?;
        if let Some(s) = &found {
            st.db.put_setting(&key, &hex::encode(s.as_slice()))?;
        }
        Ok(found)
    }

    /// Completes, signs and submits the run transactions one after another.
    async fn submit(
        &self,
        st: &AppState,
        mut ctx: crate::chain::RunContext,
        executed: Vec<Executed>,
        data: &HashMap<String, Vec<u8>>,
        tip: u64,
    ) -> Result<(), String> {
        for e in executed {
            let (o, skeleton) = match e.outcome {
                Err(err) => {
                    st.db.set_run_state(e.run_id, FAILED, Some(&err))?;
                    continue;
                }
                Ok(x) => x,
            };
            let result_hex = hex_of(&o.result);
            st.db
                .set_run_result(e.run_id, o.cycles, &result_hex, &o.debug)?;
            if o.status != Status::Ok {
                let why = match &o.error {
                    Some(m) => format!("{} (exit {}): {m}", o.status.as_str(), o.exit_code),
                    None => format!("{} (exit {})", o.status.as_str(), o.exit_code),
                };
                st.db.set_run_state(e.run_id, FAILED, Some(&why))?;
                continue;
            }
            st.db.set_run_state(
                e.run_id,
                COMPLETED,
                Some(&format!("{} cycles, result {result_hex}", o.cycles)),
            )?;
            let built = async {
                let skeleton =
                    match skeleton.filter(|s| !s.is_empty()) {
                        None => None,
                        Some(b) => Some(packed::Transaction::from_slice(&b).map_err(|_| {
                            "the transaction skeleton is not a molecule Transaction"
                        })?),
                    };
                let skeleton_input_capacity = match &skeleton {
                    Some(t) => self.chain.inputs_capacity(t).await?,
                    None => 0,
                };
                let refund_lock = match self.refund_lock(st, &e.job).await {
                    Ok(l) => l,
                    Err(err) if e.job.remaining_runs <= 1 => return Err(err),
                    Err(_) => None,
                };
                let run = RunJob {
                    outpoint: e.job.outpoint.clone(),
                    service_code_hash: hex::decode(e.job.code_hash.trim_start_matches("0x"))
                        .map_err(|_| "bad code hash")?,
                    capacity: e.job.capacity,
                    data: data
                        .get(&e.job.outpoint)
                        .cloned()
                        .ok_or("the job cell is gone")?,
                    since: since_for(&e.job.trigger),
                    tip_shannons: e.job.tip_shannons,
                    max_fee_shannons: e.job.max_fee_shannons,
                    remaining_runs: e.job.remaining_runs,
                    refund_lock,
                    skeleton,
                    skeleton_input_capacity,
                };
                self.chain.build_run(&ctx, &run)
            }
            .await;
            let built = match built {
                Ok(b) => b,
                Err(err) => {
                    st.db.set_run_state(e.run_id, FAILED, Some(&err))?;
                    continue;
                }
            };
            st.db.set_run_tx(e.run_id, &built.tx_hash, tip)?;
            st.db.set_run_state(
                e.run_id,
                SUBMITTING,
                Some(&format!("{} (fee {} shannons)", built.tx_hash, built.fee)),
            )?;
            match self.chain.send(&built.tx).await {
                Ok(_) => {
                    tracing::info!(
                        "job {}: submitted {} (tip {} shannons)",
                        e.job.outpoint,
                        built.tx_hash,
                        e.job.tip_shannons
                    );
                    ctx = built.next;
                }
                Err(msg) => {
                    // The winner's transaction may only be in the pool: the cell is still live
                    // on chain, but the node reports its input as spent.
                    let live = !is_conflict(&msg)
                        && self.chain.is_live(&e.job.outpoint).await.unwrap_or(true);
                    let (state, note) = if live {
                        (
                            REJECTED,
                            format!("the node rejected the transaction: {msg}"),
                        )
                    } else {
                        (LOST, "the job cell was spent by someone else".to_string())
                    };
                    tracing::warn!("job {}: {note}", e.job.outpoint);
                    st.db.set_run_state(e.run_id, state, Some(&note))?;
                }
            }
        }
        Ok(())
    }

    /// Follows submitted transactions: committed → `Settled` (with the block), dropped while the
    /// job cell is gone → `Lost`, dropped while it is live → `Rejected`.
    async fn track(&self, st: &AppState, tip: u64) -> Result<(), String> {
        for r in st.db.runs_in(&[SUBMITTING])? {
            let Some(tx) = r.result_tx.clone() else {
                st.db
                    .set_run_state(r.id, FAILED, Some("no transaction recorded"))?;
                continue;
            };
            let (status, block) = self.chain.tx_status(&tx).await?;
            let age = tip.saturating_sub(r.submit_block.unwrap_or(tip));
            let give_up = match status.as_str() {
                "committed" => {
                    let Some(block) = block else { continue };
                    st.db.settle(&Settlement {
                        job_outpoint: r.outpoint.clone(),
                        run_id: r.id,
                        tx_hash: tx.clone(),
                        tip_shannons: r.tip_shannons,
                        block,
                        t: crate::run::now(),
                    })?;
                    tracing::info!("job {}: settled in block {block}", r.outpoint);
                    false
                }
                "rejected" => true,
                "pending" | "proposed" => age > PENDING_AFTER_BLOCKS,
                _ => age > UNKNOWN_AFTER_BLOCKS,
            };
            if give_up {
                if self.chain.is_live(&r.outpoint).await? {
                    st.db.set_run_state(
                        r.id,
                        REJECTED,
                        Some(&format!("transaction {tx} was not committed ({status})")),
                    )?;
                } else {
                    st.db.set_run_state(
                        r.id,
                        LOST,
                        Some("the job cell was spent by someone else"),
                    )?;
                }
            }
        }
        Ok(())
    }
}

/// Whether a node error says that an input of our transaction was spent by another one.
fn is_conflict(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    ["dead(", "double", "conflict", "rbf", "unknown(outpoint"]
        .iter()
        .any(|k| m.contains(k))
}

/// `GET /api/jobs` entry of a run.
pub fn run_json(r: &JobRun) -> Value {
    json!({
        "id": r.id,
        "outpoint": r.outpoint,
        "code_hash": r.code_hash,
        "kind": "keeper",
        "trigger": r.trigger,
        "tip_shannons": r.tip_shannons,
        "tip_rate": r.tip_rate,
        "cycles": r.cycles,
        "state": r.state,
        "result_tx": r.result_tx,
    })
}

/// `GET /api/jobs/{id}`: the entry plus timeline, capped logs, result and limits.
pub fn run_detail(st: &AppState, id: i64) -> Result<Option<Value>, String> {
    let Some(r) = st.db.run(id)? else {
        return Ok(None);
    };
    let cfg = st.config.read().unwrap().clone();
    let mut v = run_json(&r);
    v["timeline"] = Value::Array(
        st.db
            .events(id)?
            .into_iter()
            .map(|e| json!({"t": e.t, "state": e.state, "note": e.note}))
            .collect(),
    );
    v["logs"] = json!(r.logs);
    v["result_hex"] = json!(r.result_hex);
    v["error"] = json!(r.error);
    v["limits"] = json!({
        "max_cycles": r.max_cycles.min(cfg.limits.max_cycles_per_job),
        "max_memory_mb": cfg.limits.max_vm_memory_mb,
        "max_wall_seconds": cfg.limits.max_job_wall_seconds,
    });
    Ok(Some(v))
}

/// `GET /api/earnings` without the runner wallet balance (added by the caller from the chain).
pub fn earnings_json(st: &AppState) -> Result<Value, String> {
    let s = st.db.settlements()?;
    Ok(json!({
        "tips_shannons": s.iter().map(|x| x.tip_shannons).sum::<u64>(),
        // Job transactions pay their fee from the job cell; the runner wallet never pays one.
        "fees_shannons": 0,
        "wallet": {"capacity_shannons": null},
        "settlements": s.iter().map(|x| json!({
            "job_outpoint": x.job_outpoint,
            "tx_hash": x.tx_hash,
            "tip_shannons": x.tip_shannons,
            "block": x.block,
            "t": x.t,
        })).collect::<Vec<_>>(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_conflicts_are_recognised() {
        assert!(is_conflict("Resolve failed Dead(OutPoint(0xab00000000))"));
        assert!(is_conflict("PoolRejectedRBF: conflict"));
        assert!(!is_conflict(
            "Verification failed Script(ValidationFailure)"
        ));
    }

    #[test]
    fn since_matches_the_trigger_table() {
        assert_eq!(since_for(&Trigger::OnDemand), 0);
        assert_eq!(since_for(&Trigger::AtBlock(42)), 42);
        assert_eq!(since_for(&Trigger::Every(2)), 0x8000_0000_0000_0002);
        assert_eq!(
            since_for(&Trigger::AtTimestamp(10_001)),
            0x4000_0000_0000_000b
        );
        assert_eq!(
            since_for(&Trigger::AtTimestamp(10_000)),
            0x4000_0000_0000_000a
        );
        let e = 5 | (1 << 24) | (4 << 40);
        assert_eq!(since_for(&Trigger::AtEpoch(e)), 0x2000_0000_0000_0000 | e);
        assert_eq!(
            since_for(&Trigger::AtEpoch(7)),
            0x2000_0000_0000_0000 | 7 | (1 << 40)
        );
    }
}
