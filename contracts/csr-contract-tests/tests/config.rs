//! Tests for `csr-config-type`, the type script of the monitor config cell (`spec/registry.md` §1).
use ckb_testtool::bytes::Bytes;
use ckb_testtool::ckb_error::Error as CkbError;
use ckb_testtool::ckb_types::packed::{OutPoint, Script};
use ckb_testtool::ckb_types::prelude::*;
use csr_contract_tests::*;

const CLOCK_MS: u64 = 1_760_000_000_000;
const FEE: u64 = 100_000;

fn assert_script_error(res: Result<u64, CkbError>, script: &str, code: i8) {
    let err = res.expect_err("transaction must be rejected");
    let s = format!("{err:?}");
    assert!(s.contains(script), "not rejected by {script}: {s}");
    assert!(s.contains(&format!("error code {code} ")), "expected error code {code}: {s}");
}

struct S {
    env: Env,
    monitor: Key,
}

fn setup() -> S {
    let mut env = Env::new();
    let monitor = env.new_key();
    S { env, monitor }
}

impl S {
    /// Creation: inputs `[monitor wallet cell]`, outputs `[config cell(s) with `data`, change]`; the Type ID of
    /// output 0 (or `id` if given).
    fn create(&mut self, data: Bytes, copies: usize, id: Option<[u8; 32]>) -> Result<u64, CkbError> {
        let wallet = self.env.cell(1000 * CKB, self.monitor.lock.clone(), None, Bytes::new());
        let input = self.env.input(&wallet, 0);
        let id = id.unwrap_or_else(|| type_id(&input, 0));
        let t = self.env.config_type_script(&id);
        let cap = min_capacity(&self.monitor.lock, Some(&t), data.len());
        let mut outputs = Vec::new();
        for _ in 0..copies {
            outputs.push((output(cap, self.monitor.lock.clone(), Some(t.clone())), data.clone()));
        }
        outputs.push((output(1000 * CKB - copies as u64 * cap - FEE, self.monitor.lock.clone(), None), Bytes::new()));
        let tx = tx_with(vec![input], outputs);
        let monitor = &self.monitor;
        self.env.verify_signed(tx, &[monitor])
    }

    /// A live config cell (placed directly). Returns (out point, its type script, capacity).
    fn live(&mut self, clock_ms: u64) -> (OutPoint, Script, u64) {
        let id = self.env.fresh_id();
        let t = self.env.config_type_script(&id);
        let data = config_data(&self.monitor.pubkey, clock_ms);
        let cap = min_capacity(&self.monitor.lock, Some(&t), data.len()) + CKB;
        let op = self.env.cell(cap, self.monitor.lock.clone(), Some(t.clone()), data);
        (op, t, cap)
    }

    /// Clock update: the live config cell (clock `CLOCK_MS`) in, `outs` copies with `data` out.
    fn update(&mut self, data: Option<Bytes>, outs: usize) -> Result<u64, CkbError> {
        let (op, t, cap) = self.live(CLOCK_MS);
        let mut outputs = Vec::new();
        match data {
            Some(d) => {
                for _ in 0..outs {
                    outputs.push((output(cap - FEE, self.monitor.lock.clone(), Some(t.clone())), d.clone()));
                }
            }
            None => outputs.push((output(cap - FEE, self.monitor.lock.clone(), None), Bytes::new())),
        }
        let tx = tx_with(vec![self.env.input(&op, 0)], outputs);
        let monitor = &self.monitor;
        self.env.verify_signed(tx, &[monitor])
    }
}

#[test]
fn config_create_ok() {
    let mut s = setup();
    let data = config_data(&s.monitor.pubkey, CLOCK_MS);
    s.create(data.clone(), 1, None).expect("create");
    // reserved bytes after the clock are allowed
    let mut long = data.to_vec();
    long.extend_from_slice(&[0u8; 23]);
    s.create(long.into(), 1, None).expect("create with reserved bytes");
}

#[test]
fn config_create_wrong_id_rejected() {
    let mut s = setup();
    let data = config_data(&s.monitor.pubkey, CLOCK_MS);
    assert_script_error(s.create(data, 1, Some([9u8; 32])), "Outputs[0].Type", 6);
}

#[test]
fn config_create_short_data_rejected() {
    let mut s = setup();
    let data = config_data(&s.monitor.pubkey, CLOCK_MS);
    assert_script_error(s.create(data.slice(0..40), 1, None), "Outputs[0].Type", 7);
}

#[test]
fn config_args_length_rejected() {
    let mut s = setup();
    let op = s.env.config_type.clone();
    let t = s.env.ctx.build_script_with_hash_type(&op, ckb_testtool::ckb_types::core::ScriptHashType::Data2, Bytes::from(vec![1u8; 33])).unwrap();
    let data = config_data(&s.monitor.pubkey, CLOCK_MS);
    let anyone = s.env.always_success_script(b"x");
    let w = s.env.cell(1000 * CKB, anyone.clone(), None, Bytes::new());
    let tx = tx_with(vec![s.env.input(&w, 0)], vec![(output(1000 * CKB, anyone, Some(t)), data)]);
    assert_script_error(s.env.verify(tx), "Outputs[0].Type", 5);
}

#[test]
fn config_clock_forward_ok() {
    let mut s = setup();
    let data = config_data(&s.monitor.pubkey, CLOCK_MS + 60_000);
    let cycles = s.update(Some(data), 1).expect("clock forward");
    println!("config clock update: {cycles} cycles (whole transaction incl. system lock)");
}

#[test]
fn config_clock_same_ok() {
    let mut s = setup();
    let data = config_data(&s.monitor.pubkey, CLOCK_MS);
    s.update(Some(data), 1).expect("clock unchanged");
}

#[test]
fn config_clock_backward_rejected() {
    let mut s = setup();
    let data = config_data(&s.monitor.pubkey, CLOCK_MS - 1);
    assert_script_error(s.update(Some(data), 1), "Inputs[0].Type", 9);
}

#[test]
fn config_monitor_key_change_rejected() {
    let mut s = setup();
    let other = s.env.new_key();
    let data = config_data(&other.pubkey, CLOCK_MS + 1);
    assert_script_error(s.update(Some(data), 1), "Inputs[0].Type", 8);
}

#[test]
fn config_update_short_data_rejected() {
    let mut s = setup();
    let data = config_data(&s.monitor.pubkey, CLOCK_MS + 1);
    assert_script_error(s.update(Some(data.slice(0..40)), 1), "Inputs[0].Type", 7);
}

#[test]
fn config_destroy_rejected() {
    let mut s = setup();
    assert_script_error(s.update(None, 1), "Inputs[0].Type", 10);
}

#[test]
fn config_second_cell_same_type_rejected() {
    let mut s = setup();
    let data = config_data(&s.monitor.pubkey, CLOCK_MS);
    // two new cells with the same Type ID
    assert_script_error(s.create(data.clone(), 2, None), "Outputs[0].Type", 6);
    // an update that splits the cell in two
    let data = config_data(&s.monitor.pubkey, CLOCK_MS + 1);
    assert_script_error(s.update(Some(data), 2), "Inputs[0].Type", 6);
}
