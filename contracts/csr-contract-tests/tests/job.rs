//! Tests for `csr-job-type` and `csr-job-lock`.
use ckb_testtool::bytes::Bytes;
use ckb_testtool::ckb_error::Error as CkbError;
use ckb_testtool::ckb_types::core::TransactionView;
use ckb_testtool::ckb_types::packed::{CellOutput, OutPoint, Script};
use ckb_testtool::ckb_types::prelude::*;
use csr_contract_tests::*;

const CLOCK_MS: u64 = 1_760_000_000_000;
const EXPIRY_MS: u64 = CLOCK_MS + 7 * 24 * 3600 * 1000;
const TIP: u64 = 2 * CKB;
const MAX_FEE: u64 = 1_000_000;
const FLOAT: u64 = 1000 * CKB;
const FLOAT_MAX_FEE: u64 = CKB;
const KEY_CELL: u64 = 100 * CKB;
const WALLET: u64 = 1000 * CKB;
const FEE: u64 = 100_000;
const EXPIRY_BLOCK: u64 = 50_000;
const SERVICE_CODE_HASH: [u8; 32] = [0x5e; 32];

/// Asserts that verification failed in script `script` (e.g. `Inputs[0].Lock`) with error code `code`.
fn assert_script_error(res: Result<u64, CkbError>, script: &str, code: i8) {
    let err = res.expect_err("transaction must be rejected");
    let s = format!("{err:?}");
    assert!(s.contains(script), "not rejected by {script}: {s}");
    assert!(s.contains(&format!("error code {code} ")), "expected error code {code}: {s}");
}

fn assert_lock_error(res: Result<u64, CkbError>, code: i8) {
    assert_script_error(res, "Inputs[0].Lock", code);
}

struct S {
    env: Env,
    /// The runner's service key: its system lock is the runner lock of the float and the registry.
    service: Key,
    /// The job's creator; its system lock is `refund_lock_hash`.
    user: Key,
    /// Live registry cell (cell dep of every run).
    registry: OutPoint,
    reg_type: Script,
    float: Script,
    job_lock: Script,
    job_type: Script,
}

fn setup() -> S {
    let mut env = Env::new();
    let owner = env.new_key();
    let service = env.new_key();
    let monitor = env.new_key();
    let user = env.new_key();
    let (_, config_hash) = env.config_cell(&monitor.pubkey, CLOCK_MS, monitor.lock.clone());
    let reg_type = env.registry_type_script(&config_hash);
    let dual = env.dual_lock_script(&owner.lock_hash(), &service.lock_hash());
    let float = env.float_lock_script(&owner.lock_hash(), &service.lock_hash(), FLOAT_MAX_FEE);
    let fields = RegistryFields {
        peer_id: b"12D3KooWExamplePeerIdForTheJobTests".to_vec(),
        service_pubkey: service.pubkey,
        float_lock_hash: float.calc_script_hash().unpack(),
        network: 2,
        prekeys: Vec::new(),
    };
    let data = fields.signed_data(&monitor, EXPIRY_MS);
    let registry = env.min_cell(10 * CKB, dual, Some(reg_type.clone()), data);
    let reg_hash: [u8; 32] = reg_type.calc_script_hash().unpack();
    let job_lock = env.job_lock_script(&reg_hash);
    let job_type = env.job_type_script(&SERVICE_CODE_HASH);
    S { env, service, user, registry, reg_type, float, job_lock, job_type }
}

impl S {
    fn job(&self, trigger: Trigger, runs: u32) -> JobFields {
        JobFields {
            trigger,
            tip_shannons: TIP,
            max_fee_shannons: MAX_FEE,
            max_cycles: 70_000_000,
            payload: b"hello job".to_vec(),
            expiry_block: EXPIRY_BLOCK,
            refund_lock_hash: self.user.lock_hash(),
            remaining_runs: runs,
        }
    }

    fn required(&self, j: &JobFields) -> u64 {
        j.required_capacity(&self.job_lock, &self.job_type)
    }

    /// Creation: user wallet in, job cell with `capacity` + change out.
    fn create(&mut self, j: &JobFields, capacity: u64) -> Result<u64, CkbError> {
        let wallet = self.env.cell(WALLET, self.user.lock.clone(), None, Bytes::new());
        let tx = tx_with(
            vec![self.env.input(&wallet, 0)],
            vec![
                (output(capacity, self.job_lock.clone(), Some(self.job_type.clone())), j.data()),
                (output(WALLET - capacity - FEE, self.user.lock.clone(), None), Bytes::new()),
            ],
        );
        let user = &self.user;
        self.env.verify_signed(tx, &[user])
    }

    /// A live job cell with exactly the required capacity. Returns (out point, capacity).
    fn live_job(&mut self, j: &JobFields) -> (OutPoint, u64) {
        let cap = self.required(j);
        let op = self.env.cell(cap, self.job_lock.clone(), Some(self.job_type.clone()), j.data());
        (op, cap)
    }

    fn job_output(&self, cap: u64, j: &JobFields) -> (CellOutput, Bytes) {
        (output(cap, self.job_lock.clone(), Some(self.job_type.clone())), j.data())
    }

    fn refund_output(&self, cap: u64) -> (CellOutput, Bytes) {
        (output(cap, self.user.lock.clone(), None), Bytes::new())
    }

    /// Run transaction of the registered runner: inputs `[job (since), float, runner key cell]`, outputs
    /// `[float + tip_paid, runner key cell + key_gain] ++ outs`, cell dep: the registry cell.
    fn run_tx(&mut self, job: &OutPoint, since: u64, tip_paid: u64, key_gain: u64, outs: Vec<(CellOutput, Bytes)>) -> TransactionView {
        let float = self.float.clone();
        let key_lock = self.service.lock.clone();
        let f = self.env.cell(FLOAT, float.clone(), None, Bytes::new());
        let k = self.env.cell(KEY_CELL, key_lock.clone(), None, Bytes::new());
        let mut outputs = vec![
            (output(FLOAT + tip_paid, float, None), Bytes::new()),
            (output(KEY_CELL + key_gain, key_lock, None), Bytes::new()),
        ];
        outputs.extend(outs);
        tx_with(vec![self.env.input(job, since), self.env.input(&f, 0), self.env.input(&k, 0)], outputs)
            .as_advanced_builder()
            .cell_dep(cell_dep(&self.registry))
            .build()
    }

    fn run(&mut self, job: &OutPoint, since: u64, tip_paid: u64, key_gain: u64, outs: Vec<(CellOutput, Bytes)>) -> Result<u64, CkbError> {
        let tx = self.run_tx(job, since, tip_paid, key_gain, outs);
        let service = &self.service;
        self.env.verify_signed(tx, &[service])
    }

    /// Single run that refunds the remainder (capacity − tip − max_fee) to the user.
    fn run_single(&mut self, trigger: Trigger, since: u64) -> Result<u64, CkbError> {
        let j = self.job(trigger, 1);
        let (op, cap) = self.live_job(&j);
        let refund = self.refund_output(cap - TIP - MAX_FEE);
        self.run(&op, since, TIP, 0, vec![refund])
    }

    /// Refund transaction: inputs `[job (since), key cell of `key`]`, everything goes to `key`; no registry dep.
    fn refund(&mut self, since: u64, user_signs: bool) -> Result<u64, CkbError> {
        let j = self.job(Trigger::OnDemand, 1);
        let (op, cap) = self.live_job(&j);
        let key = if user_signs { &self.user } else { &self.service };
        let lock = key.lock.clone();
        let k = self.env.cell(KEY_CELL, lock.clone(), None, Bytes::new());
        let tx = tx_with(
            vec![self.env.input(&op, since), self.env.input(&k, 0)],
            vec![(output(KEY_CELL + cap - FEE, lock, None), Bytes::new())],
        );
        let key = if user_signs { &self.user } else { &self.service };
        self.env.verify_signed(tx, &[key])
    }
}

// --- creation (csr-job-type) --------------------------------------------------------------------

#[test]
fn job_create_ondemand_ok() {
    let mut s = setup();
    let j = s.job(Trigger::OnDemand, 1);
    let cap = s.required(&j);
    s.create(&j, cap).expect("create");
}

#[test]
fn job_create_every_multi_run_ok() {
    let mut s = setup();
    let j = s.job(Trigger::Every(10), 5);
    let cap = s.required(&j);
    s.create(&j, cap).expect("create");
}

#[test]
fn job_create_underfunded_rejected() {
    let mut s = setup();
    let j = s.job(Trigger::Every(10), 5);
    let cap = s.required(&j) - 1;
    assert_script_error(s.create(&j, cap), "Outputs[0].Type", 8);
}

#[test]
fn job_create_multi_run_non_every_rejected() {
    let mut s = setup();
    let j = s.job(Trigger::OnDemand, 2);
    let cap = s.required(&j);
    assert_script_error(s.create(&j, cap), "Outputs[0].Type", 7);
}

#[test]
fn job_create_zero_runs_rejected() {
    let mut s = setup();
    let j = s.job(Trigger::Every(10), 0);
    let cap = s.required(&j) + 10 * CKB;
    assert_script_error(s.create(&j, cap), "Outputs[0].Type", 7);
}

#[test]
fn job_create_malformed_data_rejected() {
    let mut s = setup();
    let wallet = s.env.cell(WALLET, s.user.lock.clone(), None, Bytes::new());
    let data = Bytes::from_static(b"not a job");
    let cap = min_capacity(&s.job_lock, Some(&s.job_type), data.len()) + 10 * CKB;
    let tx = tx_with(
        vec![s.env.input(&wallet, 0)],
        vec![
            (output(cap, s.job_lock.clone(), Some(s.job_type.clone())), data),
            (output(WALLET - cap - FEE, s.user.lock.clone(), None), Bytes::new()),
        ],
    );
    assert_script_error(s.env.verify_signed(tx, &[&s.user]), "Outputs[0].Type", 6);
}

// --- csr-job-type update rules on their own (always-success lock) --------------------------------

fn type_update(s: &mut S, inp: &JobFields, out: &JobFields) -> Result<u64, CkbError> {
    let anyone = s.env.always_success_script(b"job");
    let cap = 1000 * CKB;
    let op = s.env.cell(cap, anyone.clone(), Some(s.job_type.clone()), inp.data());
    let tx = tx_with(vec![s.env.input(&op, 0)], vec![(output(cap - FEE, anyone, Some(s.job_type.clone())), out.data())]);
    s.env.verify(tx)
}

#[test]
fn job_type_update_every_ok() {
    let mut s = setup();
    let j = s.job(Trigger::Every(10), 3);
    type_update(&mut s, &j, &j.next_run()).expect("update");
}

#[test]
fn job_type_update_changed_payload_rejected() {
    let mut s = setup();
    let j = s.job(Trigger::Every(10), 3);
    let mut out = j.next_run();
    out.payload = b"other".to_vec();
    assert_script_error(type_update(&mut s, &j, &out), "Inputs[0].Type", 10);
}

#[test]
fn job_type_update_same_runs_rejected() {
    let mut s = setup();
    let j = s.job(Trigger::Every(10), 3);
    assert_script_error(type_update(&mut s, &j, &j.clone()), "Inputs[0].Type", 10);
}

#[test]
fn job_type_update_non_every_rejected() {
    let mut s = setup();
    let mut j = s.job(Trigger::AtBlock(5), 1);
    j.remaining_runs = 2; // only possible if created without the job type
    assert_script_error(type_update(&mut s, &j, &j.next_run()), "Inputs[0].Type", 9);
}

// --- run path: tip -------------------------------------------------------------------------------

#[test]
fn job_ondemand_run_tip_to_registered_float_ok() {
    let mut s = setup();
    let cycles = s.run_single(Trigger::OnDemand, 0).expect("run");
    println!("OnDemand run: {cycles} cycles (whole transaction incl. system lock and float lock)");
}

#[test]
fn job_lock_cycles_of_run() {
    // Cycles of the job lock alone: the tip is a new float cell paid from an always-success wallet,
    // and the job cell has no type script.
    let mut s = setup();
    let mut j = s.job(Trigger::OnDemand, 1);
    j.tip_shannons = 200 * CKB;
    let cap = s.required(&j);
    let op = s.env.cell(cap, s.job_lock.clone(), None, j.data());
    let anyone = s.env.always_success_script(b"wallet");
    let w = s.env.cell(WALLET, anyone.clone(), None, Bytes::new());
    let tx = tx_with(
        vec![s.env.input(&op, 0), s.env.input(&w, 0)],
        vec![
            (output(200 * CKB, s.float.clone(), None), Bytes::new()),
            s.refund_output(cap - 200 * CKB - MAX_FEE),
            (output(WALLET, anyone, None), Bytes::new()),
        ],
    )
    .as_advanced_builder()
    .cell_dep(cell_dep(&s.registry))
    .build();
    let cycles = s.env.verify(tx).expect("run");
    println!("job lock (OnDemand run, without job type): {cycles} cycles");
}

#[test]
fn job_tip_to_unregistered_float_rejected() {
    // The tip goes to the float of another runner, which is not the one in the registry cell dep.
    let mut s = setup();
    let impostor = s.env.new_key();
    let other_float = s.env.float_lock_script(&impostor.lock_hash(), &impostor.lock_hash(), FLOAT_MAX_FEE);
    let j = s.job(Trigger::OnDemand, 1);
    let (op, cap) = s.live_job(&j);
    let f = s.env.cell(FLOAT, other_float.clone(), None, Bytes::new());
    let k = s.env.cell(KEY_CELL, impostor.lock.clone(), None, Bytes::new());
    let tx = tx_with(
        vec![s.env.input(&op, 0), s.env.input(&f, 0), s.env.input(&k, 0)],
        vec![
            (output(FLOAT + TIP, other_float, None), Bytes::new()),
            (output(KEY_CELL, impostor.lock.clone(), None), Bytes::new()),
            s.refund_output(cap - TIP - MAX_FEE),
        ],
    )
    .as_advanced_builder()
    .cell_dep(cell_dep(&s.registry))
    .build();
    assert_lock_error(s.env.verify_signed(tx, &[&impostor]), 11);
}

#[test]
fn job_tip_too_small_rejected() {
    let mut s = setup();
    let j = s.job(Trigger::OnDemand, 1);
    let (op, cap) = s.live_job(&j);
    let refund = s.refund_output(cap - TIP - MAX_FEE);
    assert_lock_error(s.run(&op, 0, TIP - 1, 1, vec![refund]), 11);
}

#[test]
fn job_run_with_fake_registry_dep_rejected() {
    // A cell with registry data but another type script is no registry cell: no path applies.
    let mut s = setup();
    let (reg_cell, reg_data) = s.env.ctx.get_cell(&s.registry).unwrap();
    let fake_type = s.env.always_success_script(b"fake registry");
    let fake = s.env.ctx.create_cell(reg_cell.as_builder().type_(Some(fake_type).pack()).build(), reg_data);
    let j = s.job(Trigger::OnDemand, 1);
    let (op, cap) = s.live_job(&j);
    let refund = s.refund_output(cap - TIP - MAX_FEE);
    let tx = s.run_tx(&op, 0, TIP, 0, vec![refund]);
    // replace the registry dep by the fake one
    let deps: Vec<_> = tx.cell_deps().into_iter().filter(|d| d.out_point() != s.registry).collect();
    let tx = tx.as_advanced_builder().set_cell_deps(deps).cell_dep(cell_dep(&fake)).build();
    assert_lock_error(s.env.verify_signed(tx, &[&s.service]), 14);
}

#[test]
fn job_run_with_two_registry_deps_rejected() {
    let mut s = setup();
    let (reg_cell, reg_data) = s.env.ctx.get_cell(&s.registry).unwrap();
    let dup = s.env.ctx.create_cell(reg_cell, reg_data);
    let j = s.job(Trigger::OnDemand, 1);
    let (op, cap) = s.live_job(&j);
    let refund = s.refund_output(cap - TIP - MAX_FEE);
    let tx = s.run_tx(&op, 0, TIP, 0, vec![refund]).as_advanced_builder().cell_dep(cell_dep(&dup)).build();
    assert_lock_error(s.env.verify_signed(tx, &[&s.service]), 10);
}

#[test]
fn job_two_job_inputs_rejected() {
    // Two jobs cannot share one tip.
    let mut s = setup();
    let j = s.job(Trigger::OnDemand, 1);
    let (a, cap) = s.live_job(&j);
    let (b, _) = s.live_job(&j);
    let refund = s.refund_output(2 * (cap - TIP - MAX_FEE));
    let tx = s.run_tx(&a, 0, TIP, 0, vec![refund]);
    let tx = tx.as_advanced_builder().input(s.env.input(&b, 0)).build();
    assert_lock_error(s.env.verify_signed(tx, &[&s.service]), 7);
}

// --- run path: triggers --------------------------------------------------------------------------

#[test]
fn job_at_block_before_trigger_rejected() {
    let mut s = setup();
    assert_lock_error(s.run_single(Trigger::AtBlock(1000), since::abs_block(999)), 9);
}

#[test]
fn job_at_block_without_since_rejected() {
    let mut s = setup();
    assert_lock_error(s.run_single(Trigger::AtBlock(1000), 0), 9);
}

#[test]
fn job_at_block_after_trigger_ok() {
    let mut s = setup();
    s.run_single(Trigger::AtBlock(1000), since::abs_block(1000)).expect("run");
}

#[test]
fn job_at_block_with_relative_since_rejected() {
    let mut s = setup();
    assert_lock_error(s.run_single(Trigger::AtBlock(1000), since::rel_block(1000)), 8);
}

#[test]
fn job_at_epoch_after_trigger_ok() {
    let mut s = setup();
    // epoch 10 1/2 ≥ epoch 10 250/1000
    let t = Trigger::AtEpoch(since::epoch(10, 250, 1000));
    s.run_single(t, since::abs_epoch(since::epoch(10, 1, 2))).expect("run");
}

#[test]
fn job_at_epoch_before_trigger_rejected() {
    let mut s = setup();
    // epoch 10 1/5 < epoch 10 250/1000
    let t = Trigger::AtEpoch(since::epoch(10, 250, 1000));
    assert_lock_error(s.run_single(t, since::abs_epoch(since::epoch(10, 1, 5))), 9);
}

#[test]
fn job_at_timestamp_before_trigger_rejected() {
    let mut s = setup();
    // trigger at 1_800_000_000_500 ms; since 1_800_000_000 s is 500 ms too early
    let t = Trigger::AtTimestamp(1_800_000_000_500);
    assert_lock_error(s.run_single(t, since::abs_timestamp_secs(1_800_000_000)), 9);
}

#[test]
fn job_at_timestamp_after_trigger_ok() {
    let mut s = setup();
    let t = Trigger::AtTimestamp(1_800_000_000_500);
    s.run_single(t, since::abs_timestamp_secs(1_800_000_001)).expect("run");
}

#[test]
fn job_at_timestamp_with_block_since_rejected() {
    let mut s = setup();
    let t = Trigger::AtTimestamp(1_800_000_000_000);
    assert_lock_error(s.run_single(t, since::abs_block(1_800_000_000_000)), 8);
}

// --- run path: Every / anti-siphon ---------------------------------------------------------------

#[test]
fn job_every_run_recreates_with_runs_minus_one_ok() {
    let mut s = setup();
    let j = s.job(Trigger::Every(10), 3);
    let (op, cap) = s.live_job(&j);
    let next = s.job_output(cap - TIP - MAX_FEE, &j.next_run());
    let cycles = s.run(&op, since::rel_block(10), TIP, 0, vec![next]).expect("run");
    println!("Every run with recreation: {cycles} cycles (whole transaction)");
}

#[test]
fn job_every_run_too_early_rejected() {
    let mut s = setup();
    let j = s.job(Trigger::Every(10), 3);
    let (op, cap) = s.live_job(&j);
    let next = s.job_output(cap - TIP - MAX_FEE, &j.next_run());
    assert_lock_error(s.run(&op, since::rel_block(9), TIP, 0, vec![next]), 9);
}

#[test]
fn job_every_run_with_absolute_since_rejected() {
    let mut s = setup();
    let j = s.job(Trigger::Every(10), 3);
    let (op, cap) = s.live_job(&j);
    let next = s.job_output(cap - TIP - MAX_FEE, &j.next_run());
    assert_lock_error(s.run(&op, since::abs_block(10), TIP, 0, vec![next]), 8);
}

#[test]
fn job_every_run_without_recreation_rejected() {
    let mut s = setup();
    let j = s.job(Trigger::Every(10), 3);
    let (op, cap) = s.live_job(&j);
    let refund = s.refund_output(cap - TIP - MAX_FEE);
    assert_lock_error(s.run(&op, since::rel_block(10), TIP, 0, vec![refund]), 12);
}

#[test]
fn job_every_run_with_changed_payload_rejected() {
    let mut s = setup();
    let j = s.job(Trigger::Every(10), 3);
    let (op, cap) = s.live_job(&j);
    let mut changed = j.next_run();
    changed.payload = b"evil job".to_vec();
    let next = s.job_output(cap - TIP - MAX_FEE, &changed);
    assert_lock_error(s.run(&op, since::rel_block(10), TIP, 0, vec![next]), 12);
}

#[test]
fn job_every_run_without_decrement_rejected() {
    let mut s = setup();
    let j = s.job(Trigger::Every(10), 3);
    let (op, cap) = s.live_job(&j);
    let next = s.job_output(cap - TIP - MAX_FEE, &j);
    assert_lock_error(s.run(&op, since::rel_block(10), TIP, 0, vec![next]), 12);
}

#[test]
fn job_every_run_siphons_capacity_rejected() {
    let mut s = setup();
    let j = s.job(Trigger::Every(10), 3);
    let (op, cap) = s.live_job(&j);
    let next = s.job_output(cap - TIP - MAX_FEE - 1, &j.next_run());
    assert_lock_error(s.run(&op, since::rel_block(10), TIP, 1, vec![next]), 12);
}

#[test]
fn job_every_run_two_recreations_rejected() {
    let mut s = setup();
    let j = s.job(Trigger::Every(10), 3);
    let (op, cap) = s.live_job(&j);
    let half = (cap - TIP - MAX_FEE) / 2;
    let a = s.job_output(cap - TIP - MAX_FEE, &j.next_run());
    let b = s.job_output(half, &j.next_run());
    let tx = s.run_tx(&op, since::rel_block(10), TIP, 0, vec![a, b]);
    assert_lock_error(s.env.verify_signed(tx, &[&s.service]), 12);
}

#[test]
fn job_last_run_refunds_remainder_ok() {
    let mut s = setup();
    let j = s.job(Trigger::Every(10), 1);
    let (op, cap) = s.live_job(&j);
    let refund = s.refund_output(cap - TIP - MAX_FEE);
    s.run(&op, since::rel_block(10), TIP, 0, vec![refund]).expect("last run");
}

#[test]
fn job_last_run_without_refund_rejected() {
    // The remainder goes to the runner instead of the refund lock.
    let mut s = setup();
    let j = s.job(Trigger::Every(10), 1);
    let (op, cap) = s.live_job(&j);
    assert_lock_error(s.run(&op, since::rel_block(10), TIP, cap - TIP - MAX_FEE, vec![]), 13);
}

#[test]
fn job_single_run_refund_short_by_one_rejected() {
    let mut s = setup();
    let j = s.job(Trigger::OnDemand, 1);
    let (op, cap) = s.live_job(&j);
    let refund = s.refund_output(cap - TIP - MAX_FEE - 1);
    assert_lock_error(s.run(&op, 0, TIP, 1, vec![refund]), 13);
}

// --- refund path ---------------------------------------------------------------------------------

#[test]
fn job_refund_after_expiry_ok() {
    let mut s = setup();
    s.refund(since::abs_block(EXPIRY_BLOCK), true).expect("refund");
}

#[test]
fn job_refund_before_expiry_rejected() {
    let mut s = setup();
    assert_lock_error(s.refund(since::abs_block(EXPIRY_BLOCK - 1), true), 14);
}

#[test]
fn job_refund_with_relative_since_rejected() {
    let mut s = setup();
    assert_lock_error(s.refund(since::rel_block(EXPIRY_BLOCK), true), 14);
}

#[test]
fn job_refund_without_refund_lock_input_rejected() {
    // After expiry, but the runner (not the refund lock) tries to take the job capacity.
    let mut s = setup();
    assert_lock_error(s.refund(since::abs_block(EXPIRY_BLOCK), false), 14);
}

#[test]
fn job_registry_type_hash_is_lock_args() {
    // Sanity: the job lock args are the hash of the registry type script used in the tests.
    let s = setup();
    let want: [u8; 32] = s.reg_type.calc_script_hash().unpack();
    assert_eq!(s.job_lock.args().raw_data().as_ref(), &want);
}
