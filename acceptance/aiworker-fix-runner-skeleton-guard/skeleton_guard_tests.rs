// Acceptance tests for the skeleton input guard (F-SEC-001), written by the spec author.
// Included from runner/src/chain.rs with #[cfg(test)] #[path = ...] mod aiworker_skeleton_guard_tests;
use super::*;

const RUNNER_HASH: [u8; 32] = [0x11; 32];
const DUAL: [u8; 32] = [0xd1; 32];
const JOB_LOCK: [u8; 32] = [0x10; 32];
const REGISTRY: [u8; 32] = [0x4e; 32];
const SIGHASH: [u8; 32] = [0x9b; 32];

fn s(code_hash: [u8; 32], hash_type: u8, args: Vec<u8>) -> ScriptRef {
    ScriptRef { code_hash, hash_type, args }
}

fn policy() -> SkeletonPolicy {
    SkeletonPolicy {
        runner_lock: s(SIGHASH, 1, vec![0x22; 20]),
        runner_lock_hash: RUNNER_HASH,
        dual_lock: (DUAL, 4),
        job_lock: (JOB_LOCK, 4),
        registry_type: (REGISTRY, 4),
    }
}

fn halves(a: [u8; 32], b: [u8; 32]) -> Vec<u8> {
    let mut v = a.to_vec();
    v.extend_from_slice(&b);
    v
}

#[test]
fn skeleton_guard_allows_foreign_cells() {
    let p = policy();
    let other = s(SIGHASH, 1, vec![0x33; 20]);
    assert!(check_skeleton_input(&p, 0, &other, None).is_ok());
    assert!(check_skeleton_input(&p, 1, &other, Some(&s([0x77; 32], 1, vec![]))).is_ok());
    assert!(check_skeleton_input(&p, 2, &s(DUAL, 4, halves([0x44; 32], [0x55; 32])), None).is_ok());
}

#[test]
fn skeleton_guard_rejects_runner_wallet_cell() {
    let p = policy();
    let e = check_skeleton_input(&p, 2, &s(SIGHASH, 1, vec![0x22; 20]), None).unwrap_err();
    assert!(e.contains("skeleton input 2"), "{e}");
}

#[test]
fn skeleton_guard_rejects_runner_owned_dual_lock() {
    let p = policy();
    assert!(check_skeleton_input(&p, 0, &s(DUAL, 4, halves(RUNNER_HASH, RUNNER_HASH)), None).is_err());
    assert!(check_skeleton_input(&p, 0, &s(DUAL, 4, halves([0x44; 32], RUNNER_HASH)), None).is_err());
    assert!(check_skeleton_input(&p, 0, &s(DUAL, 4, halves(RUNNER_HASH, [0x55; 32])), None).is_err());
}

#[test]
fn skeleton_guard_rejects_job_and_registry_cells() {
    let p = policy();
    assert!(check_skeleton_input(&p, 0, &s(JOB_LOCK, 4, vec![1, 2, 3]), None).is_err());
    let other = s(SIGHASH, 1, vec![0x33; 20]);
    let e = check_skeleton_input(&p, 5, &other, Some(&s(REGISTRY, 4, vec![0xab; 32]))).unwrap_err();
    assert!(e.contains("skeleton input 5"), "{e}");
}
