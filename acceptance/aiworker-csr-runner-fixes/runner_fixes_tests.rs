// Acceptance tests for csr-runner-fixes, written by the spec author.
// Included from runner/src/db.rs with #[cfg(test)] #[path = ...] mod aiworker_runner_fixes_tests;
use super::*;
use crate::config::Config;
use std::io::Write;

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn run(outpoint: &str, code_hash: &str) -> JobRun {
    JobRun {
        id: 0,
        outpoint: outpoint.into(),
        code_hash: code_hash.into(),
        trigger: "OnDemand".into(),
        tip_shannons: 0,
        max_cycles: 1000,
        tip_rate: 0.0,
        state: "Submitting".into(),
        cycles: None,
        result_tx: None,
        result_hex: None,
        error: None,
        logs: vec![],
        submit_block: None,
    }
}

#[test]
fn ranking_changes_need_a_restart() {
    let old = Config::default();
    let mut new = old.clone();
    assert!(!old.needs_restart(&new));
    new.ranking.join = !old.ranking.join;
    assert!(old.needs_restart(&new));
    let mut new = old.clone();
    new.ranking.ping_mode = "outbound".into();
    assert!(old.needs_restart(&new));
    let mut new = old.clone();
    new.logging.level = "debug".into();
    assert!(!old.needs_restart(&new), "the log level changes at once");
}

#[test]
fn samples_keep_free_disk() {
    let db = Db::open_memory();
    let t = now();
    db.add_sample(&Sample { t, cpu_percent: 12.5, ram_free_mb: 2048, tip_lag: Some(0), disk_free_gb: Some(321) }).unwrap();
    let got = db.samples_since(t - 10).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].disk_free_gb, Some(321));
}

#[test]
fn service_totals_count_settled_jobs_and_tips() {
    let db = Db::open_memory();
    let a = format!("0x{}", "aa".repeat(32));
    let b = format!("0x{}", "bb".repeat(32));
    for (i, (op, ch, tip)) in [("0x1:0", &a, 5u64), ("0x2:0", &a, 7), ("0x3:0", &b, 11)].iter().enumerate() {
        let id = db.add_run(&run(op, ch)).unwrap();
        db.settle(&Settlement {
            job_outpoint: op.to_string(),
            run_id: id,
            tx_hash: format!("0x{:064x}", i),
            tip_shannons: *tip,
            block: 10,
            t: now(),
        })
        .unwrap();
    }
    db.add_run(&run("0x4:0", &a)).unwrap(); // not settled
    let t = db.service_totals().unwrap();
    assert_eq!(t.get(&a), Some(&(2, 12)));
    assert_eq!(t.get(&b), Some(&(1, 11)));
}

#[test]
fn mainnet_has_no_placeholder_monitor_key() {
    let d = crate::network::NetworkDefaults::for_network("mainnet");
    assert!(d.monitor_pubkey.is_empty());
    assert!(!crate::network::monitor_key_usable(""));
    assert!(!crate::network::monitor_key_usable("0x0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"));
    let t = crate::network::NetworkDefaults::for_network("testnet");
    assert!(crate::network::monitor_key_usable(&t.monitor_pubkey));
}

#[test]
fn logs_rotate_while_running() {
    let tmp = crate::test_tmp();
    let p = tmp.path().join("csr.log");
    let mut f = crate::logs::RotatingFile::open(&p, 1000).unwrap();
    for _ in 0..30 {
        f.write_all(&[b'x'; 99]).unwrap();
        f.write_all(b"\n").unwrap();
    }
    f.flush().unwrap();
    let cur = std::fs::metadata(&p).unwrap().len();
    let old = std::fs::metadata(tmp.path().join("csr.log.1")).unwrap().len();
    assert!(cur <= 1000, "current log {cur} bytes");
    assert!(old > 0 && old <= 1000, "rotated log {old} bytes");
}

#[test]
fn wallet_actions_wait_for_pending_runner_transactions() {
    assert!(crate::wallet::wallet_busy(0, false).is_none());
    assert!(crate::wallet::wallet_busy(1, false).is_some());
    assert!(crate::wallet::wallet_busy(0, true).unwrap().contains("sweep"));
}
