// Acceptance tests for csr-keeper-robustness, written by the spec author.
// Included from runner/src/keeper.rs with #[cfg(test)] #[path = ...] mod aiworker_keeper_robustness_tests;
use super::*;
use crate::config::Config;
use crate::db::{Db, JobCellRow, JobRun};
use std::collections::HashSet;

fn run(outpoint: &str, state: &str) -> JobRun {
    JobRun {
        id: 0,
        outpoint: outpoint.into(),
        code_hash: format!("0x{}", "ab".repeat(32)),
        trigger: "OnDemand".into(),
        tip_shannons: 1,
        max_cycles: 1000,
        tip_rate: 0.0,
        state: state.into(),
        cycles: None,
        result_tx: None,
        result_hex: None,
        error: None,
        logs: vec![],
        submit_block: None,
    }
}

fn add(db: &Db, outpoint: &str, state: &str) -> i64 {
    db.add_run(&run(outpoint, state)).unwrap()
}

fn state_of(db: &Db, id: i64) -> String {
    db.run(id).unwrap().unwrap().state
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

#[test]
fn abort_batch_fails_interrupted_runs() {
    let db = Db::open_memory();
    let running = add(&db, "0xa:0", "Running");
    let completed = add(&db, "0xb:0", "Completed");
    let scheduled = add(&db, "0xc:0", "Scheduled");
    let queued = add(&db, "0xd:0", "Queued");
    let submitting = add(&db, "0xe:0", "Submitting");
    let n = db.abort_batch("batch aborted: disk full").unwrap();
    assert_eq!(n, 3);
    for id in [running, completed, scheduled] {
        assert_eq!(state_of(&db, id), "Failed");
        let r = db.run(id).unwrap().unwrap();
        assert!(r.error.unwrap_or_default().contains("disk full"));
    }
    assert_eq!(state_of(&db, queued), "Queued");
    assert_eq!(state_of(&db, submitting), "Submitting");
    // The batch gate is open again.
    assert!(db.runs_in(&["Running", "Completed"]).unwrap().is_empty());
}

#[test]
fn runs_in_filters_and_orders_oldest_first() {
    let db = Db::open_memory();
    let a = add(&db, "0xa:0", "Queued");
    add(&db, "0xb:0", "Settled");
    let c = add(&db, "0xc:0", "Paused");
    let got: Vec<i64> = db.runs_in(&["Queued", "Paused"]).unwrap().iter().map(|r| r.id).collect();
    assert_eq!(got, vec![a, c]);
    assert!(db.runs_in(&[]).unwrap().is_empty());
}

#[test]
fn prune_removes_only_old_finished_runs_of_vanished_cells() {
    let db = Db::open_memory();
    let old_settled = add(&db, "0xa:0", "Settled");
    let old_failed = add(&db, "0xb:0", "Failed");
    let still_listed = add(&db, "0xc:0", "Failed");
    let waiting = add(&db, "0xd:0", "Queued");
    db.replace_job_cells(
        &[JobCellRow {
            outpoint: "0xc:0".into(),
            code_hash: format!("0x{}", "ab".repeat(32)),
            tip_shannons: 1,
            max_cycles: 1000,
            trigger: "OnDemand".into(),
            state: "discovered".into(),
            reason: None,
        }],
        10,
    )
    .unwrap();
    // Nothing is old yet.
    assert_eq!(db.prune_runs(now(), 30 * 86400).unwrap(), 0);
    // 40 days later.
    let later = now() + 40 * 86400;
    assert_eq!(db.prune_runs(later, 30 * 86400).unwrap(), 2);
    assert!(db.run(old_settled).unwrap().is_none());
    assert!(db.run(old_failed).unwrap().is_none());
    assert!(db.events(old_failed).unwrap().is_empty());
    assert!(db.run(still_listed).unwrap().is_some(), "runs of a listed job cell keep counting as attempts");
    assert!(db.run(waiting).unwrap().is_some(), "waiting runs are never pruned");
}

#[test]
fn prune_keeps_settlements() {
    let db = Db::open_memory();
    let id = add(&db, "0xa:0", "Submitting");
    db.settle(&crate::db::Settlement {
        job_outpoint: "0xa:0".into(),
        run_id: id,
        tx_hash: format!("0x{}", "cd".repeat(32)),
        tip_shannons: 5,
        block: 7,
        t: now(),
    })
    .unwrap();
    assert_eq!(db.prune_runs(now() + 40 * 86400, 30 * 86400).unwrap(), 1);
    assert_eq!(db.settlements().unwrap().len(), 1);
}

#[test]
fn attempt_number_counts_runs_of_the_same_cell() {
    let db = Db::open_memory();
    let first = add(&db, "0xa:0", "Failed");
    add(&db, "0xb:0", "Failed");
    let second = add(&db, "0xa:0", "Rejected");
    let third = add(&db, "0xa:0", "Queued");
    assert_eq!(db.attempt_of(first).unwrap(), Some(1));
    assert_eq!(db.attempt_of(second).unwrap(), Some(2));
    assert_eq!(db.attempt_of(third).unwrap(), Some(3));
    assert_eq!(db.attempt_of(9999).unwrap(), None);
}

#[test]
fn lost_only_when_the_list_is_complete() {
    let pending = vec![
        JobRun { id: 1, ..run("0xa:0", "Queued") },
        JobRun { id: 2, ..run("0xb:0", "Paused") },
    ];
    let live: HashSet<String> = ["0xa:0".to_string()].into_iter().collect();
    assert_eq!(lost_runs(&pending, &live, true), vec![2]);
    assert!(lost_runs(&pending, &live, false).is_empty());
}

#[test]
fn indexer_lag_matches_r2() {
    assert!(crate::chain::indexer_ready(100, 100));
    assert!(crate::chain::indexer_ready(100, 95));
    assert!(!crate::chain::indexer_ready(100, 94));
    assert!(crate::chain::indexer_ready(100, 101));
}

#[test]
fn pending_sweep_is_given_up() {
    assert!(!sweep_given_up(1000, 1000));
    assert!(!sweep_given_up(1000, 1000 + SWEEP_PENDING_AFTER_BLOCKS - 1));
    assert!(sweep_given_up(1000, 1000 + SWEEP_PENDING_AFTER_BLOCKS));
    assert!(!sweep_given_up(1000, 10), "a tip below the send block is not a give-up");
}

#[test]
fn expired_attestation_blocks_runs() {
    assert!(crate::chain::attestation_usable(2_000, 1_000).is_ok());
    let e = crate::chain::attestation_usable(1_000, 1_000).unwrap_err();
    assert!(e.contains("attestation expired"), "{e}");
    assert!(crate::chain::attestation_usable(1_000, 5_000).is_err());
}

#[test]
fn config_bounds_are_checked() {
    assert!(Config::parse("[payout]\nsweep_amount_ckb = 2000000000\nsweep_threshold_ckb = 2000000100").is_err());
    assert!(Config::parse("[payout]\nsweep_amount_ckb = 100000\nsweep_threshold_ckb = 100100").is_ok());
    assert!(Config::parse("[storage]\ntotal_quota_mb = 0\nper_service_quota_mb = 0").is_err());
    assert!(Config::parse("[storage]\ntotal_quota_mb = 2000000\nper_service_quota_mb = 64").is_err());
    assert!(Config::parse("[storage]\ntotal_quota_mb = 256\nper_service_quota_mb = 0").is_err());
    assert!(Config::parse("[limits]\nmin_free_ram_mb = 2000000").is_err());
    assert!(Config::parse("[limits]\nmin_free_disk_gb = 200000").is_err());
}

#[test]
fn allowed_services_must_be_code_hashes() {
    let ok = format!("[runtime]\nallowed_services = [\"0x{}\"]", "Ab".repeat(32));
    assert!(Config::parse(&ok).is_ok());
    for bad in ["ab", "0x1234", &format!("{}", "ab".repeat(32)), &format!("0x{}", "zz".repeat(32))] {
        let t = format!("[runtime]\nallowed_services = [\"{bad}\"]");
        let e = Config::parse(&t).unwrap_err();
        assert!(e.contains("runtime.allowed_services"), "{e}");
    }
}
