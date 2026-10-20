//! Tests for `csr-registry` (type script) and `csr-dual-lock`.
use ckb_testtool::bytes::Bytes;
use ckb_testtool::ckb_error::Error as CkbError;
use ckb_testtool::ckb_types::core::{ScriptHashType, TransactionView};
use ckb_testtool::ckb_types::packed::{Byte32, OutPoint, Script};
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
    /// The owner's DAO deposit (`MIN_STAKE`), the registry's `stake`.
    stake: OutPoint,
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
    let payout = env.always_success_script(b"payout");
    let stake = env.stake(owner.lock.clone());
    let fields = RegistryFields {
        peer_id: b"12D3KooWExamplePeerIdForTheRegistryTests".to_vec(),
        service_pubkey: service.pubkey,
        payout_lock_hash: payout.calc_script_hash().unpack(),
        network: 2,
        prekeys: Vec::new(),
        stake: stake.clone(),
    };
    S { env, owner, service, monitor, config, reg_type, dual, stake, fields }
}

impl S {
    /// Creation: owner wallet cell + runner (service) key cell in, registry cell (minimal capacity) + change +
    /// key cell out, cell deps `[stake of the data, config cell]`.
    fn create_tx(&mut self, data: Bytes, lock: Script, with_config: bool) -> TransactionView {
        let config = if with_config { Some(self.config.clone()) } else { None };
        let reg_type = self.reg_type.clone();
        self.env.registry_create_tx(&self.owner, Some(&self.service), lock, reg_type, data, config.as_ref())
    }

    fn create(&mut self, data: Bytes, lock: Script) -> Result<u64, CkbError> {
        let tx = self.create_tx(data, lock, true);
        self.verify_create(tx)
    }

    fn verify_create(&mut self, tx: TransactionView) -> Result<u64, CkbError> {
        let (owner, service) = (&self.owner, &self.service);
        self.env.verify_signed(tx, &[owner, service])
    }

    /// Creation with `stake` as the registry's stake (the deposit must exist) and the default lock.
    fn create_with_stake(&mut self, stake: &OutPoint) -> Result<u64, CkbError> {
        let tx = self.create_tx_with_stake(stake);
        self.verify_create(tx)
    }

    fn create_tx_with_stake(&mut self, stake: &OutPoint) -> TransactionView {
        let mut f = self.fields.clone();
        f.stake = stake.clone();
        let data = f.signed_data(&self.monitor, EXPIRY_MS);
        let dual = self.dual.clone();
        self.create_tx(data, dual, true)
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
        self.spend_with(key_is_owner, registry, None)
    }

    /// Like `spend`, plus a key cell of `extra` (in and out, signed), e.g. the new runner key.
    fn spend_with(&mut self, key_is_owner: bool, registry: Option<(u64, Script, Bytes)>, extra: Option<&Key>) -> Result<u64, CkbError> {
        self.spend_with_deps(key_is_owner, registry, extra, Vec::new())
    }

    /// Like `spend_with`, with `deps` placed first in the cell deps (e.g. the new stake).
    fn spend_with_deps(
        &mut self,
        key_is_owner: bool,
        registry: Option<(u64, Script, Bytes)>,
        extra: Option<&Key>,
        deps: Vec<OutPoint>,
    ) -> Result<u64, CkbError> {
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
        let mut inputs = vec![self.env.input(&reg, 0), self.env.input(&k, 0)];
        if let Some(x) = extra {
            let xc = self.env.cell(KEY_CELL, x.lock.clone(), None, Bytes::new());
            inputs.push(self.env.input(&xc, 0));
            outputs.push((output(KEY_CELL, x.lock.clone(), None), Bytes::new()));
        }
        let tx = tx_with(inputs, outputs).as_advanced_builder().cell_dep(cell_dep(&self.config)).build();
        let tx = with_deps_first(tx, deps.iter().map(cell_dep).collect());
        let key = if key_is_owner { &self.owner } else { &self.service };
        let mut keys = vec![key];
        keys.extend(extra);
        self.env.verify_signed(tx, &keys)
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
    // Cycles of the registry type script alone: creation tx with an always-success wallet and the runner key
    // cell (rule 6), minus a baseline tx that only spends the same key cell.
    let mut s = setup();
    let data = s.fields.signed_data(&s.monitor, EXPIRY_MS);
    let anyone = s.env.always_success_script(b"wallet");
    let w = s.env.cell(WALLET, anyone.clone(), None, Bytes::new());
    let k = s.env.cell(KEY_CELL, s.service.lock.clone(), None, Bytes::new());
    let reg_cap = min_capacity(&s.dual, Some(&s.reg_type), data.len());
    let tx = tx_with(
        vec![s.env.input(&w, 0), s.env.input(&k, 0)],
        vec![
            (output(reg_cap, s.dual.clone(), Some(s.reg_type.clone())), data),
            (output(WALLET - reg_cap - FEE, anyone, None), Bytes::new()),
            (output(KEY_CELL, s.service.lock.clone(), None), Bytes::new()),
        ],
    )
    .as_advanced_builder()
    .cell_dep(cell_dep(&s.stake))
    .cell_dep(cell_dep(&s.config))
    .build();
    let total = s.env.verify_signed(tx, &[&s.service]).expect("create");
    let k = s.env.cell(KEY_CELL, s.service.lock.clone(), None, Bytes::new());
    let base = tx_with(vec![s.env.input(&k, 0)], vec![(output(KEY_CELL - FEE, s.service.lock.clone(), None), Bytes::new())]);
    let base = s.env.verify_signed(base, &[&s.service]).expect("baseline");
    println!("registry type script (creation): {} cycles", total - base);
}

#[test]
fn registry_create_without_runner_key_rejected() {
    // Valid attestation, owner wallet only: nobody proves possession of the service key.
    let mut s = setup();
    let data = s.fields.signed_data(&s.monitor, EXPIRY_MS);
    let (dual, reg_type, config) = (s.dual.clone(), s.reg_type.clone(), s.config.clone());
    let tx = s.env.registry_create_tx(&s.owner, None, dual, reg_type, data, Some(&config));
    assert_script_error(s.env.verify_signed(tx, &[&s.owner]), "Outputs[0].Type", 17);
}

#[test]
fn registry_create_with_copied_attestation_rejected() {
    // The attacker copies the victim's (public) attestation and registers the victim's peer_id and service key
    // with the attacker as owner (and stake) and the attacker's payout cell; the attacker's own key cell is no
    // runner key.
    let mut s = setup();
    let attacker = s.env.new_key();
    let victim_attestation = s.fields.attestation_fields(EXPIRY_MS).sign(&s.monitor);
    let mut f = s.fields.clone();
    f.payout_lock_hash = attacker.lock_hash();
    f.stake = s.env.stake(attacker.lock.clone());
    let data = f.data(&victim_attestation);
    let lock = s.env.dual_lock_script(&attacker.lock_hash(), &s.service.lock_hash());
    let (reg_type, config) = (s.reg_type.clone(), s.config.clone());
    let tx = s.env.registry_create_tx(&attacker, Some(&attacker), lock, reg_type, data, Some(&config));
    assert_script_error(s.env.verify_signed(tx, &[&attacker]), "Outputs[0].Type", 17);
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
    assert_script_error(s.env.verify_signed(tx, &[&s.owner, &s.service]), "Outputs[0].Type", 8);
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
    assert_script_error(s.env.verify_signed(tx, &[&s.owner, &s.service]), "Outputs[0].Type", 8);
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
    // Rule 6 needs a runner key cell with exactly this lock: deploy the sighash binary under the genesis
    // Type ID type script (whose hash is the code hash above), as on mainnet/testnet.
    let type_id = Script::new_builder()
        .code_hash({
            // "TYPE_ID" right-aligned in 32 bytes
            let mut h = [0u8; 32];
            h[25..].copy_from_slice(b"TYPE_ID");
            h.pack()
        })
        .hash_type(ScriptHashType::Type)
        .args(
            Bytes::copy_from_slice(&[
                0x85, 0x36, 0xc9, 0xd5, 0xd9, 0x08, 0xbd, 0x89, 0xfc, 0x70, 0x09, 0x9e, 0x42, 0x84, 0x87, 0x07, 0x08,
                0xb6, 0x63, 0x23, 0x56, 0xaa, 0xd9, 0x87, 0x34, 0xfc, 0xf4, 0x3f, 0x6f, 0x71, 0xc3, 0x04,
            ])
            .pack(),
        )
        .build();
    let type_id_hash: [u8; 32] = type_id.calc_script_hash().unpack();
    assert_eq!(type_id_hash, code_hash);
    let bin = load_secp256k1_lock();
    let secp_cell = output(min_capacity(&s.owner.lock, Some(&type_id), bin.len()), s.owner.lock.clone(), Some(type_id));
    s.env.ctx.create_cell(secp_cell, bin);
    let genesis_service = Key { privkey: s.service.privkey.clone(), pubkey: s.service.pubkey, lock: runner };
    let (reg_type, config) = (s.reg_type.clone(), s.config.clone());
    let tx = s.env.registry_create_tx(&s.owner, Some(&genesis_service), lock, reg_type, data, Some(&config));
    s.env.verify_signed(tx, &[&s.owner, &genesis_service]).expect("create with genesis secp runner lock");
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
    let wallet = s.env.min_cell(2 * WALLET, s.owner.lock.clone(), None, Bytes::new());
    let reg_cap = min_capacity(&s.dual, Some(&s.reg_type), data.len());
    let tx = tx_with(
        vec![s.env.input(&wallet, 0)],
        vec![
            (output(reg_cap, s.dual.clone(), Some(s.reg_type.clone())), data.clone()),
            (output(reg_cap, s.dual.clone(), Some(s.reg_type.clone())), data),
            (output(2 * WALLET - 2 * reg_cap, s.owner.lock.clone(), None), Bytes::new()),
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
fn registry_runner_changes_payout_lock_hash_rejected() {
    let mut s = setup();
    let mut f = s.fields.clone();
    f.payout_lock_hash = s.service.lock_hash(); // e.g. the runner's own wallet
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
fn registry_owner_changes_payout_lock_hash_ok() {
    // No stake dep needed: neither the stake nor the owner changes.
    let mut s = setup();
    let mut f = s.fields.clone();
    f.payout_lock_hash = [0x77; 32];
    let data = f.signed_data(&s.monitor, EXPIRY_MS);
    let (cap, dual) = (s.reg_cap() - CKB, s.dual.clone());
    s.spend(true, Some((cap, dual, data))).expect("owner update");
}

#[test]
fn registry_owner_changes_service_pubkey_without_new_runner_key_rejected() {
    // New service key with a fresh attestation and matching dual lock, but its key cell is not an input.
    let mut s = setup();
    let new_service = s.env.new_key();
    let mut f = s.fields.clone();
    f.service_pubkey = new_service.pubkey;
    let data = f.signed_data(&s.monitor, EXPIRY_MS);
    let lock = s.env.dual_lock_script(&s.owner.lock_hash(), &new_service.lock_hash());
    let cap = s.reg_cap();
    assert_script_error(s.spend(true, Some((cap, lock, data))), "Inputs[0].Type", 17);
}

#[test]
fn registry_owner_changes_service_pubkey_with_new_runner_key_ok() {
    let mut s = setup();
    let new_service = s.env.new_key();
    let mut f = s.fields.clone();
    f.service_pubkey = new_service.pubkey;
    let data = f.signed_data(&s.monitor, EXPIRY_MS + 3_600_000);
    let lock = s.env.dual_lock_script(&s.owner.lock_hash(), &new_service.lock_hash());
    let cap = s.reg_cap();
    s.spend_with(true, Some((cap, lock, data)), Some(&new_service)).expect("owner rotates service key");
}

#[test]
fn registry_owner_changes_service_pubkey_with_old_runner_key_rejected() {
    // The old service key cell does not authorize the new key.
    let mut s = setup();
    let new_service = s.env.new_key();
    let mut f = s.fields.clone();
    f.service_pubkey = new_service.pubkey;
    let data = f.signed_data(&s.monitor, EXPIRY_MS);
    let lock = s.env.dual_lock_script(&s.owner.lock_hash(), &new_service.lock_hash());
    let cap = s.reg_cap();
    let (reg, _) = s.live_registry();
    let o = s.env.cell(KEY_CELL, s.owner.lock.clone(), None, Bytes::new());
    let k = s.env.cell(KEY_CELL, s.service.lock.clone(), None, Bytes::new());
    let tx = tx_with(
        vec![s.env.input(&reg, 0), s.env.input(&o, 0), s.env.input(&k, 0)],
        vec![
            (output(cap, lock, Some(s.reg_type.clone())), data),
            (output(KEY_CELL - FEE, s.owner.lock.clone(), None), Bytes::new()),
            (output(KEY_CELL, s.service.lock.clone(), None), Bytes::new()),
        ],
    )
    .as_advanced_builder()
    .cell_dep(cell_dep(&s.config))
    .build();
    assert_script_error(s.env.verify_signed(tx, &[&s.owner, &s.service]), "Inputs[0].Type", 17);
}

#[test]
fn registry_owner_update_still_needs_valid_attestation_rejected() {
    let mut s = setup();
    let mut f = s.fields.clone();
    f.payout_lock_hash = [0x77; 32];
    let data = f.signed_data(&s.monitor, CLOCK_MS);
    let (cap, dual) = (s.reg_cap(), s.dual.clone());
    assert_script_error(s.spend(true, Some((cap, dual, data))), "Inputs[0].Type", 12);
}

#[test]
fn registry_owner_destroys_cell_ok() {
    let mut s = setup();
    s.spend(true, None).expect("owner destroy");
}

// --- data format ------------------------------------------------------------------------------

/// A molecule table of the given (already encoded) fields.
fn mol_table(fields: &[&[u8]]) -> Vec<u8> {
    let header = 4 * (fields.len() + 1);
    let total = header + fields.iter().map(|f| f.len()).sum::<usize>();
    let mut out = (total as u32).to_le_bytes().to_vec();
    let mut off = header;
    for f in fields {
        out.extend_from_slice(&(off as u32).to_le_bytes());
        off += f.len();
    }
    for f in fields {
        out.extend_from_slice(f);
    }
    out
}

/// Field slices of a molecule table.
fn mol_fields(t: &[u8]) -> Vec<&[u8]> {
    let u = |at: usize| u32::from_le_bytes(t[at..at + 4].try_into().unwrap()) as usize;
    let n = u(4) / 4 - 1;
    (0..n).map(|i| &t[u(4 + 4 * i)..if i + 1 < n { u(8 + 4 * i) } else { t.len() }]).collect()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Vec<u8> {
    let s = s.trim_start_matches("0x");
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

#[test]
fn registry_v1_data_rejected() {
    // The v1 table (6 fields: no stake) with a valid attestation.
    let mut s = setup();
    let v2 = s.fields.signed_data(&s.monitor, EXPIRY_MS);
    let fields = mol_fields(&v2);
    assert_eq!(fields.len(), 7);
    let v1 = mol_table(&fields[..6]);
    let dual = s.dual.clone();
    let tx = s.create_tx(v1.into(), dual, true);
    let stake = s.stake.clone();
    let tx = with_deps_first(tx, vec![cell_dep(&stake)]);
    assert_script_error(s.verify_create(tx), "Outputs[0].Type", 7);
}

#[test]
fn registry_v2_data_matches_monitor_vector() {
    // The cross-check vector of the CSR Network Monitor (`spec/registry.md` §2).
    const VECTOR: &str = "7501000020000000540000007500000095000000960000004d0100005101000030000000516d5465737450656572313233343536373839306162636465666768696a6b6c6d6e6f707172737475767778797a414202466d7fcae563e5cb09a0d1870bb580344804617879a14949cf22285f1bae3f27777777777777777777777777777777777777777777777777777777777777777701b7000000180000004c0000006d0000006e0000007600000030000000516d5465737450656572313233343536373839306162636465666768696a6b6c6d6e6f707172737475767778797a414202466d7fcae563e5cb09a0d1870bb580344804617879a14949cf22285f1bae3f270100505c18a301000004d5db9878eceb46d700d0c55677777b3b1d2a401773b24dae15789c071ef7d36794160555e176e0b1ca6b997892e7c9ce7320b2a2016e3102c46a495421ed5f01000000005a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a03000000";
    let peer_id = b"QmTestPeer1234567890abcdefghijklmnopqrstuvwxyzAB".to_vec();
    let service_pubkey: [u8; 33] =
        unhex("0x02466d7fcae563e5cb09a0d1870bb580344804617879a14949cf22285f1bae3f27").try_into().unwrap();
    let signature: [u8; 65] = unhex("0x04d5db9878eceb46d700d0c55677777b3b1d2a401773b24dae15789c071ef7d36794160555e176e0b1ca6b997892e7c9ce7320b2a2016e3102c46a495421ed5f01").try_into().unwrap();
    let stake = OutPoint::new_builder().tx_hash(Byte32::from([0x5a; 32])).index(3u32).build();
    let fields = RegistryFields {
        peer_id,
        service_pubkey,
        payout_lock_hash: [0x77; 32],
        network: 1,
        prekeys: Vec::new(),
        stake,
    };
    let att = fields.attestation_fields(1_800_000_000_000).with_signature(signature);
    assert_eq!(hex(&fields.data(&att)), VECTOR);
}

// --- stake (DAO deposit) --------------------------------------------------------------------------

#[test]
fn registry_create_stake_exactly_min_ok() {
    let mut s = setup();
    let owner_lock = s.owner.lock.clone();
    let stake = s.env.deposit(MIN_STAKE, owner_lock);
    assert_eq!(MIN_STAKE, 1_000_000_000_000);
    let cycles = s.create_with_stake(&stake).expect("create with a deposit of exactly MIN_STAKE");
    println!("registry creation with stake: {cycles} cycles");
}

#[test]
fn registry_create_without_stake_dep_rejected() {
    let mut s = setup();
    let stake = s.stake.clone();
    let tx = s.create_tx_with_stake(&stake);
    let tx = without_dep(tx, &stake);
    assert_script_error(s.verify_create(tx), "Outputs[0].Type", 18);
}

#[test]
fn registry_create_stake_dep_after_dep_group_rejected() {
    // A dep group before the stake shifts the `Source::CellDep` indices; the position is not accepted.
    let mut s = setup();
    let anyone = s.env.always_success_script(b"member");
    let member = s.env.cell(100 * CKB, anyone, None, Bytes::from_static(b"some code"));
    let group = s.env.dep_group(&[member]);
    let stake = s.stake.clone();
    let tx = s.create_tx_with_stake(&stake);
    let tx = with_deps_first(tx, vec![dep_group_dep(&group)]);
    assert_script_error(s.verify_create(tx), "Outputs[0].Type", 18);
}

#[test]
fn registry_create_stake_dep_is_dep_group_rejected() {
    // The registry's `stake` names a dep group (which lists a real deposit) referenced as `dep_group`.
    let mut s = setup();
    let deposit = s.stake.clone();
    let group = s.env.dep_group(&[deposit]);
    let tx = s.create_tx_with_stake(&group);
    let tx = with_deps_first(without_dep(tx, &group), vec![dep_group_dep(&group)]);
    assert_script_error(s.verify_create(tx), "Outputs[0].Type", 18);
}

#[test]
fn registry_create_stake_outpoint_mismatch_rejected() {
    // A valid deposit of the owner is a dep, but it is not the registry's `stake`.
    let mut s = setup();
    let owner_lock = s.owner.lock.clone();
    let other = s.env.stake(owner_lock);
    let stake = s.stake.clone();
    let tx = s.create_tx_with_stake(&stake);
    let tx = with_deps_first(without_dep(tx, &stake), vec![cell_dep(&other)]);
    assert_script_error(s.verify_create(tx), "Outputs[0].Type", 18);
}

#[test]
fn registry_create_stake_not_dao_rejected() {
    let mut s = setup();
    let other_type = s.env.always_success_script(b"not the dao");
    let owner_lock = s.owner.lock.clone();
    let stake = s.env.cell(MIN_STAKE, owner_lock, Some(other_type), deposit_data());
    assert_script_error(s.create_with_stake(&stake), "Outputs[0].Type", 19);
}

#[test]
fn registry_create_stake_without_type_rejected() {
    let mut s = setup();
    let owner_lock = s.owner.lock.clone();
    let stake = s.env.cell(MIN_STAKE, owner_lock, None, deposit_data());
    assert_script_error(s.create_with_stake(&stake), "Outputs[0].Type", 19);
}

#[test]
fn registry_create_stake_dao_args_not_empty_rejected() {
    let mut s = setup();
    let dao = dao_type_script().as_builder().args(Bytes::from_static(&[0]).pack()).build();
    let owner_lock = s.owner.lock.clone();
    let stake = s.env.cell(MIN_STAKE, owner_lock, Some(dao), deposit_data());
    assert_script_error(s.create_with_stake(&stake), "Outputs[0].Type", 19);
}

#[test]
fn registry_create_stake_dao_hash_type_data_rejected() {
    let mut s = setup();
    let dao = dao_type_script().as_builder().hash_type(ScriptHashType::Data1).build();
    let owner_lock = s.owner.lock.clone();
    let stake = s.env.cell(MIN_STAKE, owner_lock, Some(dao), deposit_data());
    assert_script_error(s.create_with_stake(&stake), "Outputs[0].Type", 19);
}

#[test]
fn registry_create_stake_withdrawing_rejected() {
    // Phase 1 withdrawing cell: same DAO type, data = the deposit block number.
    let mut s = setup();
    let owner_lock = s.owner.lock.clone();
    let stake = s.env.cell(MIN_STAKE, owner_lock, Some(dao_type_script()), Bytes::copy_from_slice(&1234u64.to_le_bytes()));
    assert_script_error(s.create_with_stake(&stake), "Outputs[0].Type", 19);
}

#[test]
fn registry_create_stake_below_min_rejected() {
    // 9,999.99999999 CKB
    let mut s = setup();
    let owner_lock = s.owner.lock.clone();
    let stake = s.env.deposit(MIN_STAKE - 1, owner_lock);
    assert_script_error(s.create_with_stake(&stake), "Outputs[0].Type", 20);
}

#[test]
fn registry_create_stake_of_other_owner_rejected() {
    let mut s = setup();
    let other = s.env.new_key();
    let stake = s.env.stake(other.lock.clone());
    assert_script_error(s.create_with_stake(&stake), "Outputs[0].Type", 21);
}

#[test]
fn registry_runner_changes_stake_rejected() {
    let mut s = setup();
    let owner_lock = s.owner.lock.clone();
    let new_stake = s.env.stake(owner_lock);
    let mut f = s.fields.clone();
    f.stake = new_stake.clone();
    let data = f.signed_data(&s.monitor, EXPIRY_MS);
    let (cap, dual) = (s.reg_cap(), s.dual.clone());
    assert_script_error(s.spend_with_deps(false, Some((cap, dual, data)), None, vec![new_stake]), "Inputs[0].Type", 14);
}

#[test]
fn registry_runner_refresh_without_stake_dep_ok() {
    let mut s = setup();
    let data = s.fields.signed_data(&s.monitor, EXPIRY_MS + 3_600_000);
    let (cap, dual) = (s.reg_cap(), s.dual.clone());
    s.spend(false, Some((cap, dual, data))).expect("refresh needs no stake dep");
}

#[test]
fn registry_owner_changes_stake_with_new_deposit_ok() {
    let mut s = setup();
    let owner_lock = s.owner.lock.clone();
    let new_stake = s.env.stake(owner_lock);
    let mut f = s.fields.clone();
    f.stake = new_stake.clone();
    let data = f.signed_data(&s.monitor, EXPIRY_MS);
    let (cap, dual) = (s.reg_cap(), s.dual.clone());
    s.spend_with_deps(true, Some((cap, dual, data)), None, vec![new_stake]).expect("owner moves the stake");
}

#[test]
fn registry_owner_changes_stake_without_dep_rejected() {
    let mut s = setup();
    let owner_lock = s.owner.lock.clone();
    let new_stake = s.env.stake(owner_lock);
    let mut f = s.fields.clone();
    f.stake = new_stake;
    let data = f.signed_data(&s.monitor, EXPIRY_MS);
    let (cap, dual) = (s.reg_cap(), s.dual.clone());
    // the old deposit as dep does not help
    let old = s.stake.clone();
    assert_script_error(s.spend_with_deps(true, Some((cap, dual, data)), None, vec![old]), "Inputs[0].Type", 18);
}

#[test]
fn registry_owner_changes_owner_with_old_deposit_rejected() {
    // The owner hands the registration to a new owner, keeping the old owner's deposit as stake.
    let mut s = setup();
    let new_owner = s.env.new_key();
    let lock = s.env.dual_lock_script(&new_owner.lock_hash(), &s.service.lock_hash());
    let data = s.fields.signed_data(&s.monitor, EXPIRY_MS);
    let cap = s.reg_cap();
    let stake = s.stake.clone();
    assert_script_error(s.spend_with_deps(true, Some((cap, lock, data)), None, vec![stake]), "Inputs[0].Type", 21);
}

#[test]
fn registry_owner_changes_owner_with_new_deposit_ok() {
    let mut s = setup();
    let new_owner = s.env.new_key();
    let lock = s.env.dual_lock_script(&new_owner.lock_hash(), &s.service.lock_hash());
    let new_stake = s.env.stake(new_owner.lock.clone());
    let mut f = s.fields.clone();
    f.stake = new_stake.clone();
    let data = f.signed_data(&s.monitor, EXPIRY_MS);
    let cap = s.reg_cap();
    s.spend_with_deps(true, Some((cap, lock, data)), None, vec![new_stake]).expect("owner transfer with new deposit");
}

#[test]
fn registry_owner_changes_owner_without_stake_dep_rejected() {
    let mut s = setup();
    let new_owner = s.env.new_key();
    let lock = s.env.dual_lock_script(&new_owner.lock_hash(), &s.service.lock_hash());
    let data = s.fields.signed_data(&s.monitor, EXPIRY_MS);
    let cap = s.reg_cap();
    assert_script_error(s.spend(true, Some((cap, lock, data))), "Inputs[0].Type", 18);
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
