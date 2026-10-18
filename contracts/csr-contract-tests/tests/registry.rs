//! Tests for `csr-registry` (type script) and `csr-dual-lock`.
use ckb_testtool::bytes::Bytes;
use ckb_testtool::ckb_error::Error as CkbError;
use ckb_testtool::ckb_types::core::{ScriptHashType, TransactionView};
use ckb_testtool::ckb_types::packed::{OutPoint, Script};
use ckb_testtool::ckb_types::prelude::*;
use csr_contract_tests::*;

const CLOCK_MS: u64 = 1_760_000_000_000;
const EXPIRY_MS: u64 = CLOCK_MS + 7 * 24 * 3600 * 1000;
const WALLET: u64 = 1000 * CKB;
const FEE: u64 = 100_000;
const KEY_CELL: u64 = 100 * CKB;

/// Asserts that verification failed in script `script` (e.g. `Outputs[0].Type`) with error code `code`.
fn assert_script_error(res: Result<u64, CkbError>, script: &str, code: i8) {
    let err = res.expect_err("transaction must be rejected");
    let s = format!("{err:?}");
    assert!(s.contains(script), "not rejected by {script}: {s}");
    assert!(s.contains(&format!("error code {code} ")), "expected error code {code}: {s}");
}

struct S {
    env: Env,
    owner: Key,
    /// The service key: its system lock is the runner lock.
    service: Key,
    monitor: Key,
    config: OutPoint,
    reg_type: Script,
    dual: Script,
    fields: RegistryFields,
}

fn setup() -> S {
    let mut env = Env::new();
    let owner = env.new_key();
    let service = env.new_key();
    let monitor = env.new_key();
    let (config, config_hash) = env.config_cell(&monitor.pubkey, CLOCK_MS, monitor.lock.clone());
    let reg_type = env.registry_type_script(&config_hash);
    let dual = env.dual_lock_script(&owner.lock_hash(), &service.lock_hash());
    let float = env.float_lock_script(&owner.lock_hash(), &service.lock_hash(), CKB);
    let fields = RegistryFields {
        peer_id: b"12D3KooWExamplePeerIdForTheRegistryTests".to_vec(),
        service_pubkey: service.pubkey,
        float_lock_hash: float.calc_script_hash().unpack(),
        network: 2,
        prekeys: Vec::new(),
    };
    S { env, owner, service, monitor, config, reg_type, dual, fields }
}

impl S {
    /// Creation: owner wallet cell in, registry cell (minimal capacity) + change out, config cell as dep.
    fn create_tx(&mut self, data: Bytes, lock: Script, with_config: bool) -> TransactionView {
        let wallet = self.env.min_cell(WALLET, self.owner.lock.clone(), None, Bytes::new());
        let wallet_cap = min_capacity(&self.owner.lock, None, 0) + WALLET;
        let reg_cap = min_capacity(&lock, Some(&self.reg_type), data.len());
        let tx = tx_with(
            vec![self.env.input(&wallet, 0)],
            vec![
                (output(reg_cap, lock, Some(self.reg_type.clone())), data),
                (output(wallet_cap - reg_cap - FEE, self.owner.lock.clone(), None), Bytes::new()),
            ],
        );
        if with_config {
            tx.as_advanced_builder().cell_dep(cell_dep(&self.config)).build()
        } else {
            tx
        }
    }

    fn create(&mut self, data: Bytes, lock: Script) -> Result<u64, CkbError> {
        let tx = self.create_tx(data, lock, true);
        let owner = &self.owner;
        self.env.verify_signed(tx, &[owner])
    }

    /// A live registry cell with valid data and 10 CKB above its occupied capacity. Returns (out point, capacity).
    fn live_registry(&mut self) -> (OutPoint, u64) {
        let data = self.fields.signed_data(&self.monitor, EXPIRY_MS);
        let cap = min_capacity(&self.dual, Some(&self.reg_type), data.len()) + 10 * CKB;
        let op = self.env.cell(cap, self.dual.clone(), Some(self.reg_type.clone()), data);
        (op, cap)
    }

    /// Spends the registry cell together with a key cell of `key`; outputs are `registry` (if any) + the key cell.
    fn spend(&mut self, key_is_owner: bool, registry: Option<(u64, Script, Bytes)>) -> Result<u64, CkbError> {
        let (reg, reg_cap) = self.live_registry();
        let key = if key_is_owner { &self.owner } else { &self.service };
        let key_lock = key.lock.clone();
        let k = self.env.cell(KEY_CELL, key_lock.clone(), None, Bytes::new());
        let mut outputs = Vec::new();
        let mut back = KEY_CELL - FEE;
        match registry {
            Some((cap, lock, data)) => outputs.push((output(cap, lock, Some(self.reg_type.clone())), data)),
            None => back += reg_cap,
        }
        outputs.push((output(back, key_lock, None), Bytes::new()));
        let tx = tx_with(vec![self.env.input(&reg, 0), self.env.input(&k, 0)], outputs)
            .as_advanced_builder()
            .cell_dep(cell_dep(&self.config))
            .build();
        let key = if key_is_owner { &self.owner } else { &self.service };
        self.env.verify_signed(tx, &[key])
    }

    fn reg_cap(&self) -> u64 {
        let data = self.fields.signed_data(&self.monitor, EXPIRY_MS);
        min_capacity(&self.dual, Some(&self.reg_type), data.len()) + 10 * CKB
    }
}

// --- creation -----------------------------------------------------------------------------------

#[test]
fn registry_create_with_valid_attestation_ok() {
    let mut s = setup();
    let data = s.fields.signed_data(&s.monitor, EXPIRY_MS);
    let dual = s.dual.clone();
    let cycles = s.create(data, dual).expect("create");
    println!("registry creation: {cycles} cycles (whole transaction incl. system lock)");
    assert!(cycles < MAX_CYCLES);
}

#[test]
fn registry_create_cycles_of_type_script() {
    // Cycles of the registry type script alone: same creation tx with an always-success input lock.
    let mut s = setup();
    let data = s.fields.signed_data(&s.monitor, EXPIRY_MS);
    let anyone = s.env.always_success_script(b"wallet");
    let w = s.env.cell(WALLET, anyone.clone(), None, Bytes::new());
    let reg_cap = min_capacity(&s.dual, Some(&s.reg_type), data.len());
    let tx = tx_with(
        vec![s.env.input(&w, 0)],
        vec![(output(reg_cap, s.dual.clone(), Some(s.reg_type.clone())), data), (output(WALLET - reg_cap - FEE, anyone, None), Bytes::new())],
    )
    .as_advanced_builder()
    .cell_dep(cell_dep(&s.config))
    .build();
    let cycles = s.env.verify(tx).expect("create");
    println!("registry type script (creation): {cycles} cycles");
}

#[test]
fn registry_create_without_attestation_rejected() {
    // No monitor signature: the attestation carries an all-zero signature.
    let mut s = setup();
    let att = s.fields.attestation_fields(EXPIRY_MS).with_signature([0u8; 65]);
    let data = s.fields.data(&att);
    let dual = s.dual.clone();
    assert_script_error(s.create(data, dual), "Outputs[0].Type", 10);
}

#[test]
fn registry_create_wrong_monitor_key_rejected() {
    let mut s = setup();
    let impostor = s.env.new_key();
    let data = s.fields.signed_data(&impostor, EXPIRY_MS);
    let dual = s.dual.clone();
    assert_script_error(s.create(data, dual), "Outputs[0].Type", 10);
}

#[test]
fn registry_create_expired_attestation_rejected() {
    let mut s = setup();
    // expiry_ms == clock_ms counts as expired
    let data = s.fields.signed_data(&s.monitor, CLOCK_MS);
    let dual = s.dual.clone();
    assert_script_error(s.create(data, dual), "Outputs[0].Type", 12);
}

#[test]
fn registry_create_without_config_cell_dep_rejected() {
    let mut s = setup();
    let data = s.fields.signed_data(&s.monitor, EXPIRY_MS);
    let dual = s.dual.clone();
    let tx = s.create_tx(data, dual, false);
    assert_script_error(s.env.verify_signed(tx, &[&s.owner]), "Outputs[0].Type", 8);
}

#[test]
fn registry_create_with_two_config_cell_deps_rejected() {
    // A second cell with the same Type ID type script (cannot exist on chain, but must not confuse the script).
    let mut s = setup();
    let (cfg, _) = s.env.ctx.get_cell(&s.config).unwrap();
    let dup = s.env.ctx.create_cell(cfg, config_data(&s.monitor.pubkey, 0));
    let data = s.fields.signed_data(&s.monitor, EXPIRY_MS);
    let dual = s.dual.clone();
    let tx = s.create_tx(data, dual, true).as_advanced_builder().cell_dep(cell_dep(&dup)).build();
    assert_script_error(s.env.verify_signed(tx, &[&s.owner]), "Outputs[0].Type", 8);
}

#[test]
fn registry_create_config_from_other_monitor_rejected() {
    // A config cell with a different Type ID (an attacker's own "monitor") is not accepted.
    let mut s = setup();
    let fake_monitor = s.env.new_key();
    let (fake_cfg, _) = s.env.config_cell(&fake_monitor.pubkey, CLOCK_MS, fake_monitor.lock.clone());
    let data = s.fields.signed_data(&fake_monitor, EXPIRY_MS);
    let dual = s.dual.clone();
    let wallet = s.env.min_cell(WALLET, s.owner.lock.clone(), None, Bytes::new());
    let reg_cap = min_capacity(&dual, Some(&s.reg_type), data.len());
    let tx = tx_with(
        vec![s.env.input(&wallet, 0)],
        vec![(output(reg_cap, dual, Some(s.reg_type.clone())), data), (output(WALLET - reg_cap, s.owner.lock.clone(), None), Bytes::new())],
    )
    .as_advanced_builder()
    .cell_dep(cell_dep(&fake_cfg))
    .build();
    assert_script_error(s.env.verify_signed(tx, &[&s.owner]), "Outputs[0].Type", 8);
}

#[test]
fn registry_create_attestation_for_other_pubkey_rejected() {
    // The monitor attested another service key; the registry claims ours.
    let mut s = setup();
    let other = s.env.new_key();
    let mut att = s.fields.attestation_fields(EXPIRY_MS);
    att.service_pubkey = other.pubkey;
    let data = s.fields.data(&att.sign(&s.monitor));
    let dual = s.dual.clone();
    assert_script_error(s.create(data, dual), "Outputs[0].Type", 11);
}

#[test]
fn registry_create_attestation_for_other_peer_id_rejected() {
    let mut s = setup();
    let mut att = s.fields.attestation_fields(EXPIRY_MS);
    att.peer_id = b"12D3KooWSomebodyElse".to_vec();
    let data = s.fields.data(&att.sign(&s.monitor));
    let dual = s.dual.clone();
    assert_script_error(s.create(data, dual), "Outputs[0].Type", 11);
}

#[test]
fn registry_create_with_mismatched_runner_lock_rejected() {
    // Dual lock whose runner hash is not the system lock of `service_pubkey`.
    let mut s = setup();
    let other = s.env.new_key();
    let data = s.fields.signed_data(&s.monitor, EXPIRY_MS);
    let lock = s.env.dual_lock_script(&s.owner.lock_hash(), &other.lock_hash());
    assert_script_error(s.create(data, lock), "Outputs[0].Type", 13);
}

#[test]
fn registry_create_with_non_dual_lock_rejected() {
    // Right args, wrong code: an always-success lock carrying owner ‖ runner.
    let mut s = setup();
    let data = s.fields.signed_data(&s.monitor, EXPIRY_MS);
    let mut args = s.owner.lock_hash().to_vec();
    args.extend_from_slice(&s.service.lock_hash());
    let lock = s.env.always_success_script(&args);
    assert_script_error(s.create(data, lock), "Outputs[0].Type", 13);
}

#[test]
fn registry_create_with_genesis_secp_runner_lock_ok() {
    // Runner hash in the form used on chain: the genesis sighash lock (hash type `type`).
    let mut s = setup();
    let code_hash: [u8; 32] = [
        0x9b, 0xd7, 0xe0, 0x6f, 0x3e, 0xcf, 0x4b, 0xe0, 0xf2, 0xfc, 0xd2, 0x18, 0x8b, 0x23, 0xf1, 0xb9, 0xfc, 0xc8,
        0x8e, 0x5d, 0x4b, 0x65, 0xa8, 0x63, 0x7b, 0x17, 0x72, 0x3b, 0xbd, 0xa3, 0xcc, 0xe8,
    ];
    let runner = Script::new_builder()
        .code_hash(code_hash.pack())
        .hash_type(ScriptHashType::Type)
        .args(Bytes::copy_from_slice(&blake160(&s.service.pubkey)).pack())
        .build();
    let lock = s.env.dual_lock_script(&s.owner.lock_hash(), &runner.calc_script_hash().unpack());
    let data = s.fields.signed_data(&s.monitor, EXPIRY_MS);
    s.create(data, lock).expect("create with genesis secp runner lock");
}

#[test]
fn registry_secp_fixture_data_hash_is_embedded_constant() {
    let h = ckb_testtool::ckb_hash::blake2b_256(load_secp256k1_lock());
    assert_eq!(
        ckb_testtool::ckb_types::H256(h).to_string(),
        "709f3fda12f561cfacf92273c57a98fede188a3f1a59b1f888d113f9cce08649"
    );
}

#[test]
fn registry_create_malformed_data_rejected() {
    let mut s = setup();
    let mut data = s.fields.signed_data(&s.monitor, EXPIRY_MS).to_vec();
    data.push(0);
    let dual = s.dual.clone();
    assert_script_error(s.create(data.into(), dual), "Outputs[0].Type", 7);
}

#[test]
fn registry_two_outputs_in_group_rejected() {
    let mut s = setup();
    let data = s.fields.signed_data(&s.monitor, EXPIRY_MS);
    let wallet = s.env.min_cell(WALLET, s.owner.lock.clone(), None, Bytes::new());
    let reg_cap = min_capacity(&s.dual, Some(&s.reg_type), data.len());
    let tx = tx_with(
        vec![s.env.input(&wallet, 0)],
        vec![
            (output(reg_cap, s.dual.clone(), Some(s.reg_type.clone())), data.clone()),
            (output(reg_cap, s.dual.clone(), Some(s.reg_type.clone())), data),
            (output(WALLET - 2 * reg_cap, s.owner.lock.clone(), None), Bytes::new()),
        ],
    )
    .as_advanced_builder()
    .cell_dep(cell_dep(&s.config))
    .build();
    assert_script_error(s.env.verify_signed(tx, &[&s.owner]), "Outputs[0].Type", 6);
}

// --- runner path --------------------------------------------------------------------------------

#[test]
fn registry_runner_refreshes_attestation_ok() {
    let mut s = setup();
    let data = s.fields.signed_data(&s.monitor, EXPIRY_MS + 3_600_000);
    let (cap, dual) = (s.reg_cap(), s.dual.clone());
    let cycles = s.spend(false, Some((cap, dual, data))).expect("refresh");
    println!("registry runner refresh: {cycles} cycles");
}

#[test]
fn registry_runner_updates_prekeys_ok() {
    let mut s = setup();
    let mut f = s.fields.clone();
    f.prekeys = vec![0xab; 96];
    let data = f.signed_data(&s.monitor, EXPIRY_MS);
    // the larger data needs more capacity; the runner adds it
    let cap = s.reg_cap() + 96 * CKB;
    let dual = s.dual.clone();
    s.spend(false, Some((cap, dual, data))).expect("prekeys");
}

#[test]
fn registry_runner_refresh_with_expired_attestation_rejected() {
    let mut s = setup();
    let data = s.fields.signed_data(&s.monitor, CLOCK_MS - 1);
    let (cap, dual) = (s.reg_cap(), s.dual.clone());
    assert_script_error(s.spend(false, Some((cap, dual, data))), "Inputs[0].Type", 12);
}

#[test]
fn registry_runner_changes_float_lock_hash_rejected() {
    let mut s = setup();
    let mut f = s.fields.clone();
    f.float_lock_hash = [0x66; 32]; // e.g. the runner's own float
    let data = f.signed_data(&s.monitor, EXPIRY_MS);
    let (cap, dual) = (s.reg_cap(), s.dual.clone());
    assert_script_error(s.spend(false, Some((cap, dual, data))), "Inputs[0].Type", 14);
}

#[test]
fn registry_runner_changes_service_pubkey_rejected() {
    let mut s = setup();
    let other = s.env.new_key();
    let mut f = s.fields.clone();
    f.service_pubkey = other.pubkey;
    let data = f.signed_data(&s.monitor, EXPIRY_MS);
    let (cap, dual) = (s.reg_cap(), s.dual.clone());
    assert_script_error(s.spend(false, Some((cap, dual, data))), "Inputs[0].Type", 14);
}

#[test]
fn registry_runner_changes_lock_rejected() {
    // The runner tries to take over the owner slot of the dual lock.
    let mut s = setup();
    let data = s.fields.signed_data(&s.monitor, EXPIRY_MS);
    let lock = s.env.dual_lock_script(&s.service.lock_hash(), &s.service.lock_hash());
    let cap = s.reg_cap();
    assert_script_error(s.spend(false, Some((cap, lock, data))), "Inputs[0].Type", 14);
}

#[test]
fn registry_runner_reduces_capacity_rejected() {
    let mut s = setup();
    let data = s.fields.signed_data(&s.monitor, EXPIRY_MS + 1);
    let (cap, dual) = (s.reg_cap() - CKB, s.dual.clone());
    assert_script_error(s.spend(false, Some((cap, dual, data))), "Inputs[0].Type", 15);
}

#[test]
fn registry_runner_destroys_cell_rejected() {
    let mut s = setup();
    assert_script_error(s.spend(false, None), "Inputs[0].Type", 16);
}

// --- owner path ---------------------------------------------------------------------------------

#[test]
fn registry_owner_changes_float_lock_hash_ok() {
    let mut s = setup();
    let mut f = s.fields.clone();
    f.float_lock_hash = [0x77; 32];
    let data = f.signed_data(&s.monitor, EXPIRY_MS);
    let (cap, dual) = (s.reg_cap() - CKB, s.dual.clone());
    s.spend(true, Some((cap, dual, data))).expect("owner update");
}

#[test]
fn registry_owner_update_still_needs_valid_attestation_rejected() {
    let mut s = setup();
    let mut f = s.fields.clone();
    f.float_lock_hash = [0x77; 32];
    let data = f.signed_data(&s.monitor, CLOCK_MS);
    let (cap, dual) = (s.reg_cap(), s.dual.clone());
    assert_script_error(s.spend(true, Some((cap, dual, data))), "Inputs[0].Type", 12);
}

#[test]
fn registry_owner_destroys_cell_ok() {
    let mut s = setup();
    s.spend(true, None).expect("owner destroy");
}

// --- dual lock ----------------------------------------------------------------------------------

#[test]
fn dual_lock_without_key_cell_rejected() {
    let mut s = setup();
    let (reg, cap) = s.live_registry();
    let anyone = s.env.always_success_script(b"thief");
    let tx = tx_with(vec![s.env.input(&reg, 0)], vec![(output(cap, anyone, None), Bytes::new())])
        .as_advanced_builder()
        .cell_dep(cell_dep(&s.config))
        .build();
    assert_script_error(s.env.verify(tx), "Inputs[0].Lock", 6);
}

#[test]
fn dual_lock_bad_args_rejected() {
    let mut s = setup();
    let op = s.env.dual_lock.clone();
    let lock = s.env.script(&op, Bytes::from(vec![1u8; 63]));
    let c = s.env.cell(100 * CKB, lock, None, Bytes::new());
    let anyone = s.env.always_success_script(b"x");
    let tx = tx_with(vec![s.env.input(&c, 0)], vec![(output(100 * CKB, anyone, None), Bytes::new())]);
    assert_script_error(s.env.verify(tx), "Inputs[0].Lock", 5);
}

#[test]
fn dual_lock_runner_key_cell_ok() {
    let mut s = setup();
    let c = s.env.cell(100 * CKB, s.dual.clone(), None, Bytes::new());
    let k = s.env.cell(KEY_CELL, s.service.lock.clone(), None, Bytes::new());
    let tx = tx_with(
        vec![s.env.input(&c, 0), s.env.input(&k, 0)],
        vec![(output(100 * CKB + KEY_CELL - FEE, s.service.lock.clone(), None), Bytes::new())],
    );
    s.env.verify_signed(tx, &[&s.service]).expect("runner unlocks dual lock");
}
