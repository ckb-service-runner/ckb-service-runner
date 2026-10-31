//! Tests for `csr-registry` (type script) and `csr-dual-lock`.
use ckb_testtool::bytes::Bytes;
use ckb_testtool::ckb_error::Error as CkbError;
use ckb_testtool::ckb_types::core::{ScriptHashType, TransactionView};
use ckb_testtool::ckb_types::packed::{Byte32, OutPoint, Script};
use ckb_testtool::ckb_types::prelude::*;
use csr_contract_tests::*;

const CLOCK_MS: u64 = 1_760_000_000_000;
const EXPIRY_MS: u64 = CLOCK_MS + 7 * 24 * 3600 * 1000;
/// The lifetime cap of an attestation beyond the monitor clock (decision D24 A).
const CAP_MS: u64 = 8 * 86_400_000;
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
    let (config, config_id) = env.config_cell(&monitor.pubkey, CLOCK_MS, monitor.lock.clone());
    let reg_type = env.registry_type_script(&config_id);
    let dual = env.dual_lock_script(&owner.lock_hash(), &service.lock_hash());
    let payout = env.always_success_script(b"payout");
    let stake = env.stake(owner.lock.clone());
    let fields = RegistryFields {
        peer_id: b"12D3KooWExamplePeerIdForTheRegistryTests".to_vec(),
        service_pubkey: service.pubkey,
        payout_lock_hash: payout.calc_script_hash().unpack(),
        network: 2,
        stake: stake.clone(),
        endpoint: Vec::new(),
        prekeys: Vec::new(),
        registry_type_hash: reg_type.calc_script_hash().unpack(),
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
        self.spend_full(key_is_owner, registry, extra, deps, None)
    }

    /// Like `spend_with_deps`, plus `witness0`: bytes put into `WitnessArgs.output_type` (or `input_type` with
    /// `true`) of witness 0, the registry output's witness (e.g. the stake owner's consent).
    fn spend_full(
        &mut self,
        key_is_owner: bool,
        registry: Option<(u64, Script, Bytes)>,
        extra: Option<&Key>,
        deps: Vec<OutPoint>,
        witness0: Option<(&[u8], bool)>,
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
        let mut tx = with_deps_first(tx, deps.iter().map(cell_dep).collect());
        if let Some((bytes, in_input_type)) = witness0 {
            tx = with_witness_type(tx, 0, bytes, in_input_type);
        }
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
fn registry_v3_create_ok() {
    let mut s = setup();
    let data = s.fields.signed_data(&s.monitor, EXPIRY_MS);
    let dual = s.dual.clone();
    let cycles = s.create(data, dual).expect("create");
    println!("registry creation: {cycles} cycles (whole transaction incl. system lock)");
    assert!(cycles < MAX_CYCLES);
}

#[test]
fn registry_create_cycles_of_type_script() {
    // Cycles of the registry type script alone: creation tx with an always-success wallet, the runner key
    // cell (rule 6) and a consent signature, minus a baseline tx that only spends the same key cell.
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
    // The stake owner is no input: its consent signature is checked (the more expensive path of stake rule 4).
    let tx = with_consent(tx, &s.fields.consent_signature(&s.owner));
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
    // with the attacker as owner (and stake) and the attacker's payout cell. Since `CSR-ATTEST-v2` the signature
    // covers the stake, so it no longer verifies for the attacker's deposit (before: the missing runner key).
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
    assert_script_error(s.env.verify_signed(tx, &[&attacker]), "Outputs[0].Type", 10);
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
    // The monitor attested another service key; the registry claims ours: the signature does not verify.
    let mut s = setup();
    let other = s.env.new_key();
    let mut att = s.fields.attestation_fields(EXPIRY_MS);
    att.service_pubkey = other.pubkey;
    let data = s.fields.data(&att.sign(&s.monitor));
    let dual = s.dual.clone();
    assert_script_error(s.create(data, dual), "Outputs[0].Type", 10);
}

#[test]
fn registry_create_attestation_for_other_peer_id_rejected() {
    let mut s = setup();
    let mut att = s.fields.attestation_fields(EXPIRY_MS);
    att.peer_id = b"12D3KooWSomebodyElse".to_vec();
    let data = s.fields.data(&att.sign(&s.monitor));
    let dual = s.dual.clone();
    assert_script_error(s.create(data, dual), "Outputs[0].Type", 10);
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
fn registry_runner_refresh_v3_ok() {
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
    // a new service key needs the stake rules again (D23 A); the stake owner (= the owner) is an input
    let stake = s.stake.clone();
    s.spend_with_deps(true, Some((cap, lock, data)), Some(&new_service), vec![stake]).expect("owner rotates service key");
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

/// The old message `blake2b-256("CSR-ATTEST-v1" ‖ molecule(AttestationBody))` (peer_id, service_pubkey,
/// network, expiry_ms; no stake, no registry type hash).
fn v1_message(f: &RegistryFields, expiry_ms: u64) -> [u8; 32] {
    let peer_id = mol_bytes(&f.peer_id);
    let body = mol_table(&[peer_id.as_slice(), &f.service_pubkey, &[f.network], &expiry_ms.to_le_bytes()]);
    let mut msg = b"CSR-ATTEST-v1".to_vec();
    msg.extend_from_slice(&body);
    ckb_testtool::ckb_hash::blake2b_256(msg)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Vec<u8> {
    let s = s.trim_start_matches("0x");
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

#[test]
fn registry_v2_data_rejected() {
    // The v2 table (7 fields: nested attestation, stake last) with a valid v1-style attestation copy.
    let mut s = setup();
    let f = &s.fields;
    let expiry = EXPIRY_MS.to_le_bytes();
    let peer_id = mol_bytes(&f.peer_id);
    let att_sig = sign_digest(&s.monitor, &v1_message(f, EXPIRY_MS));
    let attestation = mol_table(&[peer_id.as_slice(), &f.service_pubkey, &[f.network], &expiry, &att_sig]);
    let v2 = mol_table(&[
        peer_id.as_slice(),
        &f.service_pubkey,
        &f.payout_lock_hash,
        &[f.network],
        &attestation,
        mol_bytes(&[]).as_slice(),
        f.stake.as_slice(),
    ]);
    let dual = s.dual.clone();
    let tx = s.create_tx(v2.into(), dual, true);
    let stake = s.stake.clone();
    let tx = with_deps_first(tx, vec![cell_dep(&stake)]);
    assert_script_error(s.verify_create(tx), "Outputs[0].Type", 7);
}

#[test]
fn registry_v3_wrong_version_rejected() {
    let mut s = setup();
    let att = s.fields.attestation_fields(EXPIRY_MS).sign(&s.monitor);
    let data = s.fields.data_with_version(2, &att);
    let dual = s.dual.clone();
    assert_script_error(s.create(data, dual), "Outputs[0].Type", 7);
}

/// The shared vector of the spec author (`acceptance/aiworker-contracts-v3/registry-v3.json`, the monitor's
/// encoder); also used by the runner and `monitor-v3`.
const VECTOR_JSON: &str = include_str!("../../../acceptance/aiworker-contracts-v3/registry-v3.json");

/// String or number value of `key` in the flat vector JSON object.
fn vector(key: &str) -> String {
    let pat = format!("\"{key}\":");
    let at = VECTOR_JSON.find(&pat).unwrap_or_else(|| panic!("vector key {key}")) + pat.len();
    let rest = VECTOR_JSON[at..].trim_start();
    if let Some(r) = rest.strip_prefix('"') {
        r[..r.find('"').unwrap()].to_string()
    } else {
        rest[..rest.find(|c: char| c == ',' || c == '}' || c.is_whitespace()).unwrap()].to_string()
    }
}

/// Registry fields of the vector (`payout_lock_hash` 0x77…, no prekeys).
fn vector_fields(endpoint: &[u8]) -> RegistryFields {
    let (tx, index) = vector("stake").split_once(':').map(|(a, b)| (unhex(a), b.parse::<u32>().unwrap())).unwrap();
    RegistryFields {
        peer_id: vector("peer_id").into_bytes(),
        service_pubkey: unhex(&vector("service_pubkey")).try_into().unwrap(),
        payout_lock_hash: [0x77; 32],
        network: vector("network").parse().unwrap(),
        stake: OutPoint::new_builder().tx_hash(Byte32::from_slice(&tx).unwrap()).index(index).build(),
        endpoint: endpoint.to_vec(),
        prekeys: Vec::new(),
        registry_type_hash: unhex(&vector("registry_type_hash")).try_into().unwrap(),
    }
}

#[test]
fn registry_v3_data_matches_vector() {
    let expiry_ms: u64 = vector("expiry_ms").parse().unwrap();
    let signature: [u8; 65] = unhex(&vector("attestation_signature")).try_into().unwrap();
    let monitor_pubkey: [u8; 33] = unhex(&vector("monitor_pubkey")).try_into().unwrap();

    // 1. the test builder produces the vector bytes (body, message, both registry data forms)
    let f = vector_fields(b"");
    let att = f.attestation_fields(expiry_ms);
    assert_eq!(hex(att.body().as_slice()), vector("attestation_body_v2"));
    assert_eq!(hex(&att.message()), vector("attestation_message_v2"));
    let data = f.data(&att.with_signature(signature));
    assert_eq!(data.len().to_string(), vector("registry_v3_no_endpoint_len"));
    assert_eq!(hex(&data), vector("registry_v3_no_endpoint"));
    let fe = vector_fields(vector("endpoint").as_bytes());
    let data_e = fe.data(&fe.attestation_fields(expiry_ms).with_signature(signature));
    assert_eq!(data_e.len().to_string(), vector("registry_v3_with_endpoint_len"));
    assert_eq!(hex(&data_e), vector("registry_v3_with_endpoint"));

    // 2. the contract's own parser and attestation check (csr-registry/src/attest.rs) accept the vector: the
    //    signature verifies against the vector's monitor key (clock 1 day before expiry)
    for d in [&data, &data_e] {
        let r = attest::parse_v3(d).expect("contract parses the vector");
        assert_eq!(hex(&r.message(&f.registry_type_hash)), vector("attestation_message_v2"));
        assert_eq!(attest::recover(&r.message(&f.registry_type_hash), r.attestation_sig), Some(monitor_pubkey));
        attest::check_attestation(&r, &f.registry_type_hash, &monitor_pubkey, expiry_ms - 86_400_000).expect("valid");
        assert_eq!(
            attest::check_attestation(&r, &[0x4f; 32], &monitor_pubkey, expiry_ms - 86_400_000),
            Err(attest::AttestError::Signature)
        );
    }

    // 3. in the VM: the vector's monitor (secret 0x…51) as the config cell's key, the vector entry (with
    //    endpoint) registered under this test deployment's registry type hash
    let mut s = setup();
    let secret: [u8; 32] = unhex(&vector("monitor_secret")).try_into().unwrap();
    let monitor = s.env.key_from_secret(&secret);
    assert_eq!(monitor.pubkey, monitor_pubkey);
    let (config, config_id) = s.env.config_cell(&monitor.pubkey, expiry_ms - 86_400_000, monitor.lock.clone());
    let reg_type = s.env.registry_type_script(&config_id);
    let service = Key { privkey: s.service.privkey.clone(), pubkey: s.service.pubkey, lock: s.service.lock.clone() };
    let mut g = vector_fields(vector("endpoint").as_bytes());
    g.service_pubkey = service.pubkey;
    g.stake = s.stake.clone();
    g.registry_type_hash = reg_type.calc_script_hash().unpack();
    let data = g.signed_data(&monitor, expiry_ms);
    let dual = s.dual.clone();
    let tx = s.env.registry_create_tx(&s.owner, Some(&service), dual, reg_type, data, Some(&config));
    s.env.verify_signed(tx, &[&s.owner, &service]).expect("vector monitor key accepted by the contract");
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
fn registry_create_stake_of_other_owner_without_consent_rejected() {
    // A deposit of somebody else who is no input and gave no consent signature.
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
fn registry_owner_changes_owner_with_new_deposit_ok() {
    let mut s = setup();
    let new_owner = s.env.new_key();
    let lock = s.env.dual_lock_script(&new_owner.lock_hash(), &s.service.lock_hash());
    let new_stake = s.env.stake(new_owner.lock.clone());
    let mut f = s.fields.clone();
    f.stake = new_stake.clone();
    let data = f.signed_data(&s.monitor, EXPIRY_MS);
    let cap = s.reg_cap();
    // the new owner (the new stake owner) is no input: it consents by signature
    let consent = f.consent_signature(&new_owner);
    s.spend_full(true, Some((cap, lock, data)), None, vec![new_stake], Some((&consent, false)))
        .expect("owner transfer with new deposit");
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

// --- consent: the stake owner signs a message (A8, D-A8-1) ---------------------------------------

impl S {
    /// A staker: a key that owns a fresh `MIN_STAKE` deposit and is neither the registry owner nor an input.
    /// Returns (key, deposit).
    fn staker(&mut self) -> (Key, OutPoint) {
        let k = self.env.new_key();
        let stake = self.env.stake(k.lock.clone());
        (k, stake)
    }

    /// Owner-owned creation (`create_tx`) with `f` as the registry fields and `consent` (if any) in
    /// `WitnessArgs.output_type` of witness 0 (or `input_type` with `in_input_type`).
    fn create_with_consent(&mut self, f: &RegistryFields, consent: Option<&[u8]>, in_input_type: bool) -> Result<u64, CkbError> {
        let data = f.signed_data(&self.monitor, EXPIRY_MS);
        let dual = self.dual.clone();
        let mut tx = self.create_tx(data, dual, true);
        if let Some(sig) = consent {
            tx = with_witness_type(tx, 0, sig, in_input_type);
        }
        self.verify_create(tx)
    }

    /// Fields with the stake of `stake`.
    fn fields_with_stake(&self, stake: &OutPoint) -> RegistryFields {
        RegistryFields { stake: stake.clone(), ..self.fields.clone() }
    }
}

#[test]
fn registry_create_with_consent_signature_ok() {
    // The stake owner is no input; it signed the consent text (Neuron "Sign message").
    let mut s = setup();
    let (staker, stake) = s.staker();
    let f = s.fields_with_stake(&stake);
    let sig = f.consent_signature(&staker);
    let cycles = s.create_with_consent(&f, Some(&sig), false).expect("consent by signature");
    println!("registry creation with consent signature: {cycles} cycles (whole transaction incl. system locks)");
}

#[test]
fn registry_create_with_stake_owner_input_ok() {
    // The A7 way: the stake owner's cell is an input (here a staker that is not the registry owner).
    let mut s = setup();
    let (staker, stake) = s.staker();
    let f = s.fields_with_stake(&stake);
    let data = f.signed_data(&s.monitor, EXPIRY_MS);
    let dual = s.dual.clone();
    let tx = s.create_tx(data, dual, true);
    let sc = s.env.cell(KEY_CELL, staker.lock.clone(), None, Bytes::new());
    let tx = tx
        .as_advanced_builder()
        .input(s.env.input(&sc, 0))
        .output(output(KEY_CELL, staker.lock.clone(), None))
        .output_data(Bytes::new().pack())
        .build();
    s.env.verify_signed(tx, &[&s.owner, &s.service, &staker]).expect("consent by input");
}

#[test]
fn registry_create_without_stake_owner_consent_rejected() {
    let mut s = setup();
    let (_, stake) = s.staker();
    let f = s.fields_with_stake(&stake);
    assert_script_error(s.create_with_consent(&f, None, false), "Outputs[0].Type", 21);
}

#[test]
fn registry_create_consent_wrong_length_rejected() {
    let mut s = setup();
    let (staker, stake) = s.staker();
    let f = s.fields_with_stake(&stake);
    let sig = f.consent_signature(&staker);
    assert_script_error(s.create_with_consent(&f, Some(&sig[..64]), false), "Outputs[0].Type", 22);
}

#[test]
fn registry_create_consent_bad_recovery_id_rejected() {
    let mut s = setup();
    let (staker, stake) = s.staker();
    let f = s.fields_with_stake(&stake);
    let mut sig = f.consent_signature(&staker);
    sig[64] = 4;
    assert_script_error(s.create_with_consent(&f, Some(&sig), false), "Outputs[0].Type", 22);
    sig[64] = 31;
    assert_script_error(s.create_with_consent(&f, Some(&sig), false), "Outputs[0].Type", 22);
}

#[test]
fn registry_create_consent_high_s_ok() {
    let mut s = setup();
    let (staker, stake) = s.staker();
    let f = s.fields_with_stake(&stake);
    let sig = to_high_s(&f.consent_signature(&staker));
    s.create_with_consent(&f, Some(&sig), false).expect("high-S is normalised");
}

#[test]
fn registry_create_consent_v27_ok() {
    let mut s = setup();
    let (staker, stake) = s.staker();
    let f = s.fields_with_stake(&stake);
    let mut sig = f.consent_signature(&staker);
    sig[64] += 27;
    s.create_with_consent(&f, Some(&sig), false).expect("v 27/28");
}

#[test]
fn registry_create_consent_by_other_key_rejected() {
    let mut s = setup();
    let (_, stake) = s.staker();
    let other = s.env.new_key();
    let f = s.fields_with_stake(&stake);
    let sig = f.consent_signature(&other);
    assert_script_error(s.create_with_consent(&f, Some(&sig), false), "Outputs[0].Type", 23);
}

#[test]
fn registry_create_consent_for_other_stake_rejected() {
    // The staker signed for another of its deposits.
    let mut s = setup();
    let (staker, stake) = s.staker();
    let other_stake = s.env.stake(staker.lock.clone());
    let f = s.fields_with_stake(&stake);
    let sig = s.fields_with_stake(&other_stake).consent_signature(&staker);
    assert_script_error(s.create_with_consent(&f, Some(&sig), false), "Outputs[0].Type", 23);
}

#[test]
fn registry_create_consent_for_other_service_key_rejected() {
    let mut s = setup();
    let (staker, stake) = s.staker();
    let other = s.env.new_key();
    let f = s.fields_with_stake(&stake);
    let signed = RegistryFields { service_pubkey: other.pubkey, ..f.clone() };
    let sig = signed.consent_signature(&staker);
    assert_script_error(s.create_with_consent(&f, Some(&sig), false), "Outputs[0].Type", 23);
}

#[test]
fn registry_create_consent_for_other_peer_id_rejected() {
    let mut s = setup();
    let (staker, stake) = s.staker();
    let f = s.fields_with_stake(&stake);
    let signed = RegistryFields { peer_id: b"12D3KooWSomebodyElse".to_vec(), ..f.clone() };
    let sig = signed.consent_signature(&staker);
    assert_script_error(s.create_with_consent(&f, Some(&sig), false), "Outputs[0].Type", 23);
}

#[test]
fn registry_create_consent_for_other_network_rejected() {
    // Signed for testnet, registered on devnet (the registry's `network` is 2).
    let mut s = setup();
    let (staker, stake) = s.staker();
    let f = s.fields_with_stake(&stake);
    let signed = RegistryFields { network: 1, ..f.clone() };
    let sig = signed.consent_signature(&staker);
    assert_script_error(s.create_with_consent(&f, Some(&sig), false), "Outputs[0].Type", 23);
}

#[test]
fn registry_create_consent_without_magic_prefix_rejected() {
    // The staker signed blake2b-256(text) without "Nervos Message:".
    let mut s = setup();
    let (staker, stake) = s.staker();
    let f = s.fields_with_stake(&stake);
    let digest = ckb_testtool::ckb_hash::blake2b_256(f.consent_text());
    let sig = sign_digest(&staker, &digest);
    assert_script_error(s.create_with_consent(&f, Some(&sig), false), "Outputs[0].Type", 23);
}

#[test]
fn registry_create_consent_in_input_type_rejected() {
    let mut s = setup();
    let (staker, stake) = s.staker();
    let f = s.fields_with_stake(&stake);
    let sig = f.consent_signature(&staker);
    assert_script_error(s.create_with_consent(&f, Some(&sig), true), "Outputs[0].Type", 21);
}

#[test]
fn registry_owner_changes_stake_with_consent_ok() {
    let mut s = setup();
    let (staker, new_stake) = s.staker();
    let f = s.fields_with_stake(&new_stake);
    let data = f.signed_data(&s.monitor, EXPIRY_MS);
    let sig = f.consent_signature(&staker);
    let (cap, dual) = (s.reg_cap(), s.dual.clone());
    s.spend_full(true, Some((cap, dual, data)), None, vec![new_stake], Some((&sig, false))).expect("new stake with consent");
}

#[test]
fn registry_owner_changes_stake_without_consent_rejected() {
    let mut s = setup();
    let (_, new_stake) = s.staker();
    let f = s.fields_with_stake(&new_stake);
    let data = f.signed_data(&s.monitor, EXPIRY_MS);
    let (cap, dual) = (s.reg_cap(), s.dual.clone());
    assert_script_error(s.spend_with_deps(true, Some((cap, dual, data)), None, vec![new_stake]), "Inputs[0].Type", 21);
}

#[test]
fn registry_owner_changes_owner_without_stake_consent_rejected() {
    // The live registration's stake belongs to a staker; the owner hands the registration to a new owner,
    // keeping the stake, without the staker's consent.
    let mut s = setup();
    let (_, stake) = s.staker();
    s.fields.stake = stake.clone();
    let new_owner = s.env.new_key();
    let lock = s.env.dual_lock_script(&new_owner.lock_hash(), &s.service.lock_hash());
    let data = s.fields.signed_data(&s.monitor, EXPIRY_MS);
    let cap = s.reg_cap();
    assert_script_error(s.spend_with_deps(true, Some((cap, lock, data)), None, vec![stake]), "Inputs[0].Type", 21);
}

#[test]
fn registry_owner_changes_owner_with_stake_owner_input_ok() {
    let mut s = setup();
    let (staker, stake) = s.staker();
    s.fields.stake = stake.clone();
    let new_owner = s.env.new_key();
    let lock = s.env.dual_lock_script(&new_owner.lock_hash(), &s.service.lock_hash());
    let data = s.fields.signed_data(&s.monitor, EXPIRY_MS);
    let cap = s.reg_cap();
    s.spend_with_deps(true, Some((cap, lock, data)), Some(&staker), vec![stake]).expect("staker is an input");
}

#[test]
fn registry_runner_refresh_needs_no_consent_ok() {
    // A registration whose stake owner consented by signature once: the runner's refresh needs neither the
    // stake dep nor a new consent.
    let mut s = setup();
    let (_, stake) = s.staker();
    s.fields.stake = stake;
    let data = s.fields.signed_data(&s.monitor, EXPIRY_MS + 3_600_000);
    let (cap, dual) = (s.reg_cap(), s.dual.clone());
    s.spend(false, Some((cap, dual, data))).expect("refresh");
}

/// The concept's example registration (`spec/registry.md` §4.2).
fn example_fields() -> RegistryFields {
    RegistryFields {
        peer_id: b"QmTestPeer1234567890abcdefghijklmnopqrstuvwxyzAB".to_vec(),
        service_pubkey: unhex("02466d7fcae563e5cb09a0d1870bb580344804617879a14949cf22285f1bae3f27").try_into().unwrap(),
        payout_lock_hash: [0; 32],
        network: 1,
        stake: OutPoint::new_builder().tx_hash(Byte32::from([0x5a; 32])).index(3u32).build(),
        endpoint: Vec::new(),
        prekeys: Vec::new(),
        registry_type_hash: [0; 32],
    }
}

#[test]
fn registry_consent_text_vector() {
    let f = example_fields();
    assert_eq!(
        String::from_utf8(f.consent_text()).unwrap(),
        "CSR-STAKE-CONSENT-v1|testnet|0x5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a:3|\
         0x02466d7fcae563e5cb09a0d1870bb580344804617879a14949cf22285f1bae3f27|QmTestPeer1234567890abcdefghijklmnopqrstuvwxyzAB"
    );
    assert_eq!(hex(&f.consent_digest()), "e3425937793f1e3c6de997e908df0c8ac00efdfd8db48948adca4d4e9a4b51c2");
    assert_eq!(f.consent_digest(), consent::message_digest(&f.consent_text()));
    // index in decimal, other networks by name
    let g = RegistryFields { network: 0, stake: OutPoint::new_builder().tx_hash(Byte32::from([0; 32])).index(1234u32).build(), ..f };
    let t = String::from_utf8(g.consent_text()).unwrap();
    assert!(t.starts_with("CSR-STAKE-CONSENT-v1|mainnet|0x0000"), "{t}");
    assert!(t.contains(":1234|0x02466d"), "{t}");
    assert!(consent::network_name(3).is_none());
}

#[test]
fn registry_consent_neuron_vector() {
    // A real Neuron "Sign message" signature of lock args 0x89c04afd2ded4725e915020cb757a20d353f1094
    // (ckt1qzda0cr08m85hc8jlnfp3zer7xulejywt49kt2rr0vthywaa50xwsqvfcp906t0dguj7j9gzpjm40gsdx5l3p9qutm6yg).
    let digest = consent::message_digest(b"Message from CSR");
    assert_eq!(hex(&digest), "2bb15d28fd5e98471846af625ba4e735e60491793d2159cd595331cb81a1a8a0");
    let sig = unhex(
        "c0fee686c87c2f398a83ef0ab8ddcc462b0acd552411674b3d1cf8e9ce6a86563204ffd3e2b82a6dfa9be40c58e1d670faf8ea8656db45e1d05d4265912877f401",
    );
    let (rs, recid) = consent::parse_signature(&sig).expect("parseable");
    let mut norm = rs.to_vec();
    norm.push(recid);
    let pubkey = recover_pubkey(&digest, &norm);
    assert_eq!(hex(&blake160(&pubkey)), "89c04afd2ded4725e915020cb757a20d353f1094");
    // the high-S and v+27 forms parse to the same signature
    let sig: [u8; 65] = sig.try_into().unwrap();
    assert_eq!(consent::parse_signature(&to_high_s(&sig)), Some((rs, recid)));
    let mut v27 = sig;
    v27[64] += 27;
    assert_eq!(consent::parse_signature(&v27), Some((rs, recid)));
    assert_eq!(consent::parse_signature(&sig[..64]), None);
}

// --- runner-owned registration (A8 runner wallet) ------------------------------------------------

impl S {
    /// Fields of a registration owned by the runner: `payout_lock_hash` = the runner lock, stake of `stake`.
    fn runner_owned_fields(&self, stake: &OutPoint) -> RegistryFields {
        RegistryFields { payout_lock_hash: self.service.lock_hash(), stake: stake.clone(), ..self.fields.clone() }
    }

    /// The dual lock `runner ‖ runner`.
    fn runner_owned_lock(&mut self) -> Script {
        let r = self.service.lock_hash();
        self.env.dual_lock_script(&r, &r)
    }

    /// A live runner-owned registry cell of `f` (10 CKB above its occupied capacity). Returns (out point, capacity).
    fn live_runner_owned(&mut self, f: &RegistryFields) -> (OutPoint, u64) {
        let lock = self.runner_owned_lock();
        let data = f.signed_data(&self.monitor, EXPIRY_MS);
        let cap = min_capacity(&lock, Some(&self.reg_type), data.len()) + 10 * CKB;
        let op = self.env.cell(cap, lock, Some(self.reg_type.clone()), data);
        (op, cap)
    }

    /// Runner-owned update/removal: inputs `[runner wallet cell, registry cell of f_in]`, outputs
    /// `[registry cell with `data_out` (if any), wallet change]`, cell deps `deps ++ [config]`, `consent` in
    /// `WitnessArgs.output_type` of witness 0 (covered by the runner's signature). Signed by the runner only.
    fn runner_owned_spend(&mut self, f_in: &RegistryFields, data_out: Option<Bytes>, deps: Vec<OutPoint>, consent: Option<&[u8]>) -> Result<u64, CkbError> {
        let (reg, reg_cap) = self.live_runner_owned(f_in);
        let runner = self.service.lock.clone();
        let w = self.env.runner_wallet_cell(&self.service, WALLET);
        let lock = self.runner_owned_lock();
        let mut outputs = Vec::new();
        let mut change = WALLET - FEE;
        match data_out {
            Some(d) => outputs.push((output(reg_cap, lock, Some(self.reg_type.clone())), d)),
            None => change += reg_cap,
        }
        outputs.push((output(change, runner, None), Bytes::new()));
        let tx = tx_with(vec![self.env.input(&w, 0), self.env.input(&reg, 0)], outputs)
            .as_advanced_builder()
            .cell_dep(cell_dep(&self.config))
            .build();
        let mut tx = with_deps_first(tx, deps.iter().map(cell_dep).collect());
        if let Some(sig) = consent {
            tx = with_consent(tx, sig);
        }
        let service = &self.service;
        self.env.verify_signed(tx, &[service])
    }
}

#[test]
fn registry_create_owned_by_runner_ok() {
    // Inputs are only runner wallet cells; the stake owner consents by signature; owner = runner.
    let mut s = setup();
    let (staker, stake) = s.staker();
    let f = s.runner_owned_fields(&stake);
    let data = f.signed_data(&s.monitor, EXPIRY_MS);
    let sig = f.consent_signature(&staker);
    let (reg_type, config) = (s.reg_type.clone(), s.config.clone());
    let lock = s.runner_owned_lock();
    let tx = s.env.registry_create_runner_owned_tx(&s.service, lock, reg_type, data, &config, Some(&sig));
    let cycles = s.env.verify_signed(tx, &[&s.service]).expect("runner-owned registration");
    println!("runner-owned registration with consent: {cycles} cycles (whole transaction incl. system lock)");
}

#[test]
fn registry_runner_owned_refresh_ok() {
    // Owner = runner: the refresh takes the owner path; the stake and the owner stay, so no dep, no consent.
    let mut s = setup();
    let (_, stake) = s.staker();
    let f = s.runner_owned_fields(&stake);
    let data = f.signed_data(&s.monitor, EXPIRY_MS + 3_600_000);
    s.runner_owned_spend(&f, Some(data), Vec::new(), None).expect("refresh");
}

#[test]
fn registry_runner_owned_changes_stake_with_consent_ok() {
    let mut s = setup();
    let (_, stake) = s.staker();
    let (staker2, new_stake) = s.staker();
    let f = s.runner_owned_fields(&stake);
    let g = s.runner_owned_fields(&new_stake);
    let data = g.signed_data(&s.monitor, EXPIRY_MS + 3_600_000);
    let sig = g.consent_signature(&staker2);
    s.runner_owned_spend(&f, Some(data), vec![new_stake], Some(&sig)).expect("new stake with consent");
}

#[test]
fn registry_runner_owned_changes_stake_without_consent_rejected() {
    let mut s = setup();
    let (_, stake) = s.staker();
    let (_, new_stake) = s.staker();
    let f = s.runner_owned_fields(&stake);
    let g = s.runner_owned_fields(&new_stake);
    let data = g.signed_data(&s.monitor, EXPIRY_MS);
    assert_script_error(s.runner_owned_spend(&f, Some(data), vec![new_stake], None), "Inputs[1].Type", 21);
}

#[test]
fn registry_runner_owned_destroy_ok() {
    // Removal: the registry cell's capacity returns to the runner wallet.
    let mut s = setup();
    let (_, stake) = s.staker();
    let f = s.runner_owned_fields(&stake);
    s.runner_owned_spend(&f, None, Vec::new(), None).expect("remove");
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

// --- contracts v3: attestation v2, lifetime cap, endpoint, consent on identity changes (D20–D24) ----------

#[test]
fn registry_attest_v1_signature_rejected() {
    // A valid monitor signature over the v1 message (no stake, no registry type hash).
    let mut s = setup();
    let sig = sign_digest(&s.monitor, &v1_message(&s.fields, EXPIRY_MS));
    let data = s.fields.data(&s.fields.attestation_fields(EXPIRY_MS).with_signature(sig));
    let dual = s.dual.clone();
    assert_script_error(s.create(data, dual), "Outputs[0].Type", 10);
}

#[test]
fn registry_attest_v2_other_stake_rejected() {
    // The monitor attested this node with another deposit; the attestation is used with this one.
    let mut s = setup();
    let owner_lock = s.owner.lock.clone();
    let other = s.env.stake(owner_lock);
    let mut att = s.fields.attestation_fields(EXPIRY_MS);
    att.stake = other;
    let data = s.fields.data(&att.sign(&s.monitor));
    let dual = s.dual.clone();
    assert_script_error(s.create(data, dual), "Outputs[0].Type", 10);
}

#[test]
fn registry_attest_v2_other_registry_type_rejected() {
    // An attestation for another registry deployment (same code, other config args → other type hash).
    let mut s = setup();
    let other_id = s.env.fresh_id();
    let other_type = s.env.registry_type_script(&other_id);
    let mut att = s.fields.attestation_fields(EXPIRY_MS);
    att.registry_type_hash = other_type.calc_script_hash().unpack();
    assert_ne!(att.registry_type_hash, s.fields.registry_type_hash);
    let data = s.fields.data(&att.sign(&s.monitor));
    let dual = s.dual.clone();
    assert_script_error(s.create(data, dual), "Outputs[0].Type", 10);
}

#[test]
fn registry_attestation_lifetime_over_cap_rejected() {
    // expiry_ms − clock_ms = 8 days is accepted, 8 days + 1 ms is not (error 24), on creation and on refresh.
    let mut s = setup();
    let data = s.fields.signed_data(&s.monitor, CLOCK_MS + CAP_MS);
    let dual = s.dual.clone();
    s.create(data, dual).expect("exactly at the cap");
    let data = s.fields.signed_data(&s.monitor, CLOCK_MS + CAP_MS + 1);
    let dual = s.dual.clone();
    assert_script_error(s.create(data, dual), "Outputs[0].Type", 24);
    let data = s.fields.signed_data(&s.monitor, CLOCK_MS + CAP_MS + 1);
    let (cap, dual) = (s.reg_cap(), s.dual.clone());
    assert_script_error(s.spend(false, Some((cap, dual, data))), "Inputs[0].Type", 24);
}

#[test]
fn registry_attestation_network_mismatch_rejected() {
    // The monitor attested the node for testnet (1); the cell says devnet (2).
    let mut s = setup();
    let mut att = s.fields.attestation_fields(EXPIRY_MS);
    att.network = 1;
    let data = s.fields.data(&att.sign(&s.monitor));
    let dual = s.dual.clone();
    assert_script_error(s.create(data, dual), "Outputs[0].Type", 10);
    // an unknown network byte is no v3 data
    let mut f = s.fields.clone();
    f.network = 3;
    let data = f.signed_data(&s.monitor, EXPIRY_MS);
    let dual = s.dual.clone();
    assert_script_error(s.create(data, dual), "Outputs[0].Type", 7);
}

#[test]
fn registry_create_with_endpoint_ok() {
    let mut s = setup();
    let mut f = s.fields.clone();
    f.endpoint = b"http://203.0.113.7:8131".to_vec();
    let data = f.signed_data(&s.monitor, EXPIRY_MS);
    let dual = s.dual.clone();
    s.create(data, dual).expect("http endpoint");
    f.endpoint = [b"https://".as_slice(), &[b'a'; 120]].concat();
    assert_eq!(f.endpoint.len(), 128);
    let data = f.signed_data(&s.monitor, EXPIRY_MS);
    let dual = s.dual.clone();
    s.create(data, dual).expect("128-byte endpoint");
}

#[test]
fn registry_endpoint_invalid_rejected() {
    let mut s = setup();
    let bad: [Vec<u8>; 4] = [
        b"ftp://203.0.113.7:21".to_vec(),
        b"203.0.113.7:8131".to_vec(),
        [b"https://".as_slice(), &[b'a'; 121]].concat(),
        b"https://\xff\xfe:1".to_vec(),
    ];
    for e in bad {
        let mut f = s.fields.clone();
        f.endpoint = e;
        let data = f.signed_data(&s.monitor, EXPIRY_MS);
        let dual = s.dual.clone();
        assert_script_error(s.create(data, dual), "Outputs[0].Type", 7);
    }
}

#[test]
fn registry_runner_changes_endpoint_ok() {
    // The runner path may set the endpoint (and pays for the larger cell).
    let mut s = setup();
    let mut f = s.fields.clone();
    f.endpoint = b"https://203.0.113.7:8131".to_vec();
    let data = f.signed_data(&s.monitor, EXPIRY_MS + 3_600_000);
    let cap = s.reg_cap() + 24 * CKB;
    let dual = s.dual.clone();
    s.spend(false, Some((cap, dual, data))).expect("runner sets the endpoint");
}

#[test]
fn registry_runner_changes_peer_id_rejected() {
    let mut s = setup();
    let mut f = s.fields.clone();
    f.peer_id = b"12D3KooWAnotherPeer".to_vec();
    let data = f.signed_data(&s.monitor, EXPIRY_MS);
    let (cap, dual) = (s.reg_cap(), s.dual.clone());
    assert_script_error(s.spend(false, Some((cap, dual, data))), "Inputs[0].Type", 14);
}

impl S {
    /// Makes the live registration's stake a staker's deposit (the staker is neither the owner nor an input).
    /// Returns (staker, deposit).
    fn with_staker(&mut self) -> (Key, OutPoint) {
        let (k, stake) = self.staker();
        self.fields.stake = stake.clone();
        (k, stake)
    }
}

#[test]
fn registry_owner_changes_peer_id_without_consent_rejected() {
    let mut s = setup();
    let (_, stake) = s.with_staker();
    let mut f = s.fields.clone();
    f.peer_id = b"12D3KooWAnotherPeer".to_vec();
    let data = f.signed_data(&s.monitor, EXPIRY_MS);
    let (cap, dual) = (s.reg_cap(), s.dual.clone());
    // without the stake dep the stake rules fail first
    assert_script_error(s.spend(true, Some((cap, dual.clone(), data.clone()))), "Inputs[0].Type", 18);
    assert_script_error(s.spend_with_deps(true, Some((cap, dual, data)), None, vec![stake]), "Inputs[0].Type", 21);
}

#[test]
fn registry_owner_changes_network_without_consent_rejected() {
    let mut s = setup();
    let (staker, stake) = s.with_staker();
    let mut f = s.fields.clone();
    f.network = 1;
    let data = f.signed_data(&s.monitor, EXPIRY_MS);
    let (cap, dual) = (s.reg_cap(), s.dual.clone());
    assert_script_error(s.spend_with_deps(true, Some((cap, dual.clone(), data.clone())), None, vec![stake.clone()]), "Inputs[0].Type", 21);
    // the old consent (for the old network) does not do
    let old = s.fields.consent_signature(&staker);
    assert_script_error(s.spend_full(true, Some((cap, dual, data)), None, vec![stake], Some((&old, false))), "Inputs[0].Type", 23);
}

#[test]
fn registry_owner_changes_service_pubkey_without_consent_rejected() {
    let mut s = setup();
    let (_, stake) = s.with_staker();
    let new_service = s.env.new_key();
    let mut f = s.fields.clone();
    f.service_pubkey = new_service.pubkey;
    let data = f.signed_data(&s.monitor, EXPIRY_MS);
    let lock = s.env.dual_lock_script(&s.owner.lock_hash(), &new_service.lock_hash());
    let cap = s.reg_cap();
    assert_script_error(s.spend_with_deps(true, Some((cap, lock, data)), Some(&new_service), vec![stake]), "Inputs[0].Type", 21);
}

#[test]
fn registry_owner_changes_peer_id_with_consent_ok() {
    let mut s = setup();
    let (staker, stake) = s.with_staker();
    let mut f = s.fields.clone();
    f.peer_id = b"12D3KooWAnotherPeer".to_vec();
    let data = f.signed_data(&s.monitor, EXPIRY_MS);
    let sig = f.consent_signature(&staker);
    let (cap, dual) = (s.reg_cap(), s.dual.clone());
    s.spend_full(true, Some((cap, dual, data)), None, vec![stake], Some((&sig, false))).expect("new peer id with consent");
}

#[test]
fn registry_owner_changes_endpoint_needs_no_consent_ok() {
    let mut s = setup();
    s.with_staker();
    let mut f = s.fields.clone();
    f.endpoint = b"https://node.example.org:8131".to_vec();
    let data = f.signed_data(&s.monitor, EXPIRY_MS);
    let (cap, dual) = (s.reg_cap() + 30 * CKB, s.dual.clone());
    s.spend(true, Some((cap, dual, data))).expect("endpoint and payout are no identity fields");
}

#[test]
fn registry_runner_owned_short_args_ok() {
    // Owner = runner with 32-byte dual-lock args: created with the stake owner's consent, refreshed by the
    // runner, and the dual lock opens with the runner key cell.
    let mut s = setup();
    let (staker, stake) = s.staker();
    let f = RegistryFields { payout_lock_hash: s.service.lock_hash(), stake, ..s.fields.clone() };
    let data = f.signed_data(&s.monitor, EXPIRY_MS);
    let sig = f.consent_signature(&staker);
    let short = s.env.dual_lock_script_short(&s.service.lock_hash());
    let long = s.runner_owned_lock();
    let (reg_type, config) = (s.reg_type.clone(), s.config.clone());
    let tx = s.env.registry_create_runner_owned_tx(&s.service, short.clone(), reg_type.clone(), data.clone(), &config, Some(&sig));
    s.env.verify_signed(tx, &[&s.service]).expect("runner-owned registration, 32-byte args");
    // 32 bytes less than `runner ‖ runner`: 32 CKB less capacity
    assert_eq!(min_capacity(&long, Some(&reg_type), data.len()) - min_capacity(&short, Some(&reg_type), data.len()), 32 * CKB);

    // refresh
    let cap = min_capacity(&short, Some(&reg_type), data.len());
    let reg = s.env.cell(cap, short.clone(), Some(reg_type.clone()), data);
    let w = s.env.runner_wallet_cell(&s.service, WALLET);
    let new = f.signed_data(&s.monitor, EXPIRY_MS + 3_600_000);
    let tx = tx_with(
        vec![s.env.input(&w, 0), s.env.input(&reg, 0)],
        vec![(output(cap, short.clone(), Some(reg_type)), new), (output(WALLET - FEE, s.service.lock.clone(), None), Bytes::new())],
    )
    .as_advanced_builder()
    .cell_dep(cell_dep(&config))
    .build();
    s.env.verify_signed(tx, &[&s.service]).expect("refresh with 32-byte args");

    // without the runner key cell the dual lock stays shut
    let c = s.env.cell(100 * CKB, short, None, Bytes::new());
    let anyone = s.env.always_success_script(b"thief");
    let tx = tx_with(vec![s.env.input(&c, 0)], vec![(output(100 * CKB, anyone, None), Bytes::new())]);
    assert_script_error(s.env.verify(tx), "Inputs[0].Lock", 6);
}

#[test]
fn registry_short_args_runner_mismatch_rejected() {
    // 32-byte args that are not the system lock of `service_pubkey`.
    let mut s = setup();
    let data = s.fields.signed_data(&s.monitor, EXPIRY_MS);
    let lock = s.env.dual_lock_script_short(&s.owner.lock_hash());
    assert_script_error(s.create(data, lock), "Outputs[0].Type", 13);
}

#[test]
fn registry_type_args_length_rejected() {
    let mut s = setup();
    for len in [31usize, 33, 0] {
        let op = s.env.registry.clone();
        let reg_type = s.env.script(&op, Bytes::from(vec![7u8; len]));
        let mut f = s.fields.clone();
        f.registry_type_hash = reg_type.calc_script_hash().unpack();
        let data = f.signed_data(&s.monitor, EXPIRY_MS);
        let (dual, config) = (s.dual.clone(), s.config.clone());
        let tx = s.env.registry_create_tx(&s.owner, Some(&s.service), dual, reg_type, data, Some(&config));
        assert_script_error(s.env.verify_signed(tx, &[&s.owner, &s.service]), "Outputs[0].Type", 5);
    }
}

#[test]
fn registry_short_config_data_rejected() {
    // A config cell (right type script) with 40 bytes of data.
    let mut s = setup();
    let (cell, data) = s.env.ctx.get_cell(&s.config).unwrap();
    let short = s.env.ctx.create_cell(cell, data.slice(0..40));
    let data = s.fields.signed_data(&s.monitor, EXPIRY_MS);
    let dual = s.dual.clone();
    let tx = s.create_tx(data, dual, false).as_advanced_builder().cell_dep(cell_dep(&short)).build();
    assert_script_error(s.verify_create(tx), "Outputs[0].Type", 9);
}

#[test]
fn registry_config_without_config_type_rejected() {
    // A Type ID cell with the right data but not `csr-config-type`, and `csr-config-type` with hash type data1.
    let mut s = setup();
    let (cell, data) = s.env.ctx.get_cell(&s.config).unwrap();
    let id: [u8; 32] = cell.type_().to_opt().unwrap().args().raw_data().as_ref().try_into().unwrap();
    let wrong_hash_type = s.env.ctx.build_script_with_hash_type(&s.env.config_type, ScriptHashType::Data1, Bytes::copy_from_slice(&id)).unwrap();
    let other = s.env.always_success_script(&id);
    for t in [wrong_hash_type, other] {
        let c = s.env.ctx.create_cell(cell.clone().as_builder().type_(Some(t).pack()).build(), data.clone());
        let reg = s.fields.signed_data(&s.monitor, EXPIRY_MS);
        let dual = s.dual.clone();
        let tx = s.create_tx(reg, dual, false).as_advanced_builder().cell_dep(cell_dep(&c)).build();
        assert_script_error(s.verify_create(tx), "Outputs[0].Type", 8);
    }
}
