//! Tests for `csr-float-lock`.
use ckb_testtool::bytes::Bytes;
use ckb_testtool::ckb_error::Error as CkbError;
use ckb_testtool::ckb_types::packed::Script;
use csr_contract_tests::*;

const MAX_FEE: u64 = CKB; // 1 CKB per transaction
const FLOAT: u64 = 1000 * CKB;

/// Asserts that verification failed in the lock script of input `idx` with error code `code`.
fn assert_lock_error(res: Result<u64, CkbError>, idx: usize, code: i8) {
    let err = res.expect_err("transaction must be rejected");
    let s = format!("{err:?}");
    assert!(s.contains(&format!("Inputs[{idx}].Lock")), "not rejected by lock of input {idx}: {s}");
    assert!(s.contains(&format!("error code {code} ")), "expected error code {code}: {s}");
}

struct Setup {
    env: Env,
    owner: Key,
    runner: Key,
    float: Script,
}

fn setup() -> Setup {
    let mut env = Env::new();
    let owner = env.new_key();
    let runner = env.new_key();
    let float = env.float_lock_script(&owner.lock_hash(), &runner.lock_hash(), MAX_FEE);
    Setup { env, owner, runner, float }
}

#[test]
fn float_runner_tip_deposit_ok() {
    // A deposit does not spend the float cell: the float lock does not even run.
    let Setup { mut env, runner, float, .. } = setup();
    let key_cell = env.min_cell(100 * CKB, runner.lock.clone(), None, Bytes::new());
    let tx = tx_with(
        vec![env.input(&key_cell, 0)],
        vec![(output(FLOAT + 5 * CKB, float, None), Bytes::new()), (output(95 * CKB, runner.lock.clone(), None), Bytes::new())],
    );
    env.verify_signed(tx, &[&runner]).expect("deposit");
}

#[test]
fn float_runner_fee_within_max_ok() {
    let Setup { mut env, runner, float, .. } = setup();
    let f = env.cell(FLOAT, float.clone(), None, Bytes::new());
    let k = env.min_cell(10 * CKB, runner.lock.clone(), None, Bytes::new());
    // exactly max_fee leaves the float
    let tx = tx_with(
        vec![env.input(&f, 0), env.input(&k, 0)],
        vec![(output(FLOAT - MAX_FEE, float, None), Bytes::new()), (output(10 * CKB + MAX_FEE, runner.lock.clone(), None), Bytes::new())],
    );
    env.verify_signed(tx, &[&runner]).expect("fee within max");
}

#[test]
fn float_runner_merge_two_floats_ok() {
    let Setup { mut env, runner, float, .. } = setup();
    let a = env.cell(FLOAT, float.clone(), None, Bytes::new());
    let b = env.cell(FLOAT / 2, float.clone(), None, Bytes::new());
    let k = env.min_cell(10 * CKB, runner.lock.clone(), None, Bytes::new());
    let tx = tx_with(
        vec![env.input(&a, 0), env.input(&b, 0), env.input(&k, 0)],
        vec![(output(FLOAT + FLOAT / 2 - MAX_FEE, float, None), Bytes::new()), (output(10 * CKB, runner.lock.clone(), None), Bytes::new())],
    );
    env.verify_signed(tx, &[&runner]).expect("merge");
}

#[test]
fn float_runner_fee_above_max_rejected() {
    let Setup { mut env, runner, float, .. } = setup();
    let f = env.cell(FLOAT, float.clone(), None, Bytes::new());
    let k = env.min_cell(10 * CKB, runner.lock.clone(), None, Bytes::new());
    let tx = tx_with(
        vec![env.input(&f, 0), env.input(&k, 0)],
        vec![(output(FLOAT - MAX_FEE - 1, float, None), Bytes::new()), (output(10 * CKB + MAX_FEE + 1, runner.lock.clone(), None), Bytes::new())],
    );
    assert_lock_error(env.verify_signed(tx, &[&runner]), 0, 8);
}

#[test]
fn float_runner_redirect_to_other_lock_rejected() {
    let Setup { mut env, runner, float, .. } = setup();
    let f = env.cell(FLOAT, float, None, Bytes::new());
    let k = env.min_cell(10 * CKB, runner.lock.clone(), None, Bytes::new());
    let tx = tx_with(
        vec![env.input(&f, 0), env.input(&k, 0)],
        vec![(output(FLOAT, runner.lock.clone(), None), Bytes::new()), (output(10 * CKB, runner.lock.clone(), None), Bytes::new())],
    );
    assert_lock_error(env.verify_signed(tx, &[&runner]), 0, 7);
}

#[test]
fn float_runner_two_float_outputs_rejected() {
    let Setup { mut env, runner, float, .. } = setup();
    let f = env.cell(FLOAT, float.clone(), None, Bytes::new());
    let k = env.min_cell(10 * CKB, runner.lock.clone(), None, Bytes::new());
    let tx = tx_with(
        vec![env.input(&f, 0), env.input(&k, 0)],
        vec![
            (output(FLOAT / 2, float.clone(), None), Bytes::new()),
            (output(FLOAT / 2, float, None), Bytes::new()),
            (output(10 * CKB, runner.lock.clone(), None), Bytes::new()),
        ],
    );
    assert_lock_error(env.verify_signed(tx, &[&runner]), 0, 7);
}

#[test]
fn float_runner_changed_args_rejected() {
    let Setup { mut env, runner, float, .. } = setup();
    let other = env.new_key();
    // same code, different owner in args: a float the attacker controls
    let evil = env.float_lock_script(&other.lock_hash(), &runner.lock_hash(), MAX_FEE);
    let f = env.cell(FLOAT, float, None, Bytes::new());
    let k = env.min_cell(10 * CKB, runner.lock.clone(), None, Bytes::new());
    let tx = tx_with(
        vec![env.input(&f, 0), env.input(&k, 0)],
        vec![(output(FLOAT, evil, None), Bytes::new()), (output(10 * CKB, runner.lock.clone(), None), Bytes::new())],
    );
    assert_lock_error(env.verify_signed(tx, &[&runner]), 0, 7);
}

#[test]
fn float_runner_without_runner_input_rejected() {
    // Another key cell is present, but it is neither owner nor runner.
    let Setup { mut env, float, .. } = setup();
    let stranger = env.new_key();
    let f = env.cell(FLOAT, float.clone(), None, Bytes::new());
    let k = env.min_cell(10 * CKB, stranger.lock.clone(), None, Bytes::new());
    let tx = tx_with(
        vec![env.input(&f, 0), env.input(&k, 0)],
        vec![(output(FLOAT, float, None), Bytes::new()), (output(10 * CKB, stranger.lock.clone(), None), Bytes::new())],
    );
    assert_lock_error(env.verify_signed(tx, &[&stranger]), 0, 6);
}

#[test]
fn float_owner_withdraw_all_ok() {
    let Setup { mut env, owner, float, .. } = setup();
    let f = env.cell(FLOAT, float, None, Bytes::new());
    let k = env.min_cell(10 * CKB, owner.lock.clone(), None, Bytes::new());
    let tx = tx_with(
        vec![env.input(&f, 0), env.input(&k, 0)],
        vec![(output(FLOAT + 10 * CKB, owner.lock.clone(), None), Bytes::new())],
    );
    env.verify_signed(tx, &[&owner]).expect("owner withdraw");
}

#[test]
fn float_without_any_key_input_rejected() {
    let Setup { mut env, float, .. } = setup();
    let f = env.cell(FLOAT, float, None, Bytes::new());
    let anyone = env.always_success_script(b"x");
    let tx = tx_with(vec![env.input(&f, 0)], vec![(output(FLOAT, anyone, None), Bytes::new())]);
    assert_lock_error(env.verify(tx), 0, 6);
}
