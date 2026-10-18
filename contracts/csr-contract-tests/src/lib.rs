//! Test harness for the CSR contracts: deploys the five binaries, the system
//! secp256k1-blake160 lock and a Type ID config cell, and builds/signs transactions.
//!
//! Contract binaries come from `contracts/build/release/` (run `scripts/build-contracts.sh`).

use ckb_testtool::bytes::Bytes;
use ckb_testtool::ckb_crypto::secp::{Generator, Privkey};
use ckb_testtool::ckb_error::Error as CkbError;
use ckb_testtool::ckb_hash::{blake2b_256, new_blake2b};
use ckb_testtool::ckb_types::core::{Capacity, ScriptHashType, TransactionBuilder, TransactionView};
use ckb_testtool::ckb_types::packed::{self, Byte32, CellInput, CellOutput, OutPoint, Script};
use ckb_testtool::ckb_types::prelude::*;
use ckb_testtool::context::Context;
use std::path::PathBuf;

pub const MAX_CYCLES: u64 = 70_000_000;
pub const CKB: u64 = 100_000_000;

pub const CONTRACT_NAMES: [&str; 5] = [
    "csr-float-lock",
    "csr-dual-lock",
    "csr-registry",
    "csr-job-type",
    "csr-job-lock",
];

/// Path of a built contract binary.
pub fn contract_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../build/release").join(name)
}

pub fn load_contract(name: &str) -> Bytes {
    let p = contract_path(name);
    std::fs::read(&p)
        .unwrap_or_else(|e| panic!("cannot read {} ({e}); run scripts/build-contracts.sh", p.display()))
        .into()
}

/// The system secp256k1_blake160_sighash_all lock (ckb-system-scripts 0.5.4).
pub fn load_secp256k1_lock() -> Bytes {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/secp256k1_blake160_sighash_all");
    std::fs::read(p).expect("secp256k1 fixture").into()
}

/// blake160 of arbitrary bytes (first 20 bytes of blake2b-256 with the ckb personalization).
pub fn blake160(data: &[u8]) -> [u8; 20] {
    let mut out = [0u8; 20];
    out.copy_from_slice(&blake2b_256(data)[..20]);
    out
}

/// A key pair with the corresponding system lock script.
pub struct Key {
    pub privkey: Privkey,
    /// Compressed (33 byte) public key.
    pub pubkey: [u8; 33],
    pub lock: Script,
}

impl Key {
    pub fn lock_hash(&self) -> [u8; 32] {
        self.lock.calc_script_hash().unpack()
    }
}

/// Everything a test needs: a context with all contracts deployed.
pub struct Env {
    pub ctx: Context,
    pub float_lock: OutPoint,
    pub dual_lock: OutPoint,
    pub registry: OutPoint,
    pub job_type: OutPoint,
    pub job_lock: OutPoint,
    pub secp: OutPoint,
    /// Precomputed tables the system lock loads as a cell dep.
    pub secp_data: OutPoint,
    pub always_success: OutPoint,
    /// Counter for unique out points of fresh cells.
    seed: u64,
}

impl Env {
    pub fn new() -> Self {
        let mut ctx = Context::new_with_deterministic_rng();
        let float_lock = ctx.deploy_cell(load_contract("csr-float-lock"));
        let dual_lock = ctx.deploy_cell(load_contract("csr-dual-lock"));
        let registry = ctx.deploy_cell(load_contract("csr-registry"));
        let job_type = ctx.deploy_cell(load_contract("csr-job-type"));
        let job_lock = ctx.deploy_cell(load_contract("csr-job-lock"));
        let secp = ctx.deploy_cell(load_secp256k1_lock());
        let secp_data = ctx.deploy_cell(
            std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/secp256k1_data")).expect("secp data").into(),
        );
        let always_success = ctx.deploy_cell(ckb_testtool::builtin::ALWAYS_SUCCESS.clone());
        Env { ctx, float_lock, dual_lock, registry, job_type, job_lock, secp, secp_data, always_success, seed: 1 }
    }

    /// Script referencing a deployed binary by data hash (hash type data1).
    pub fn script(&mut self, code: &OutPoint, args: Bytes) -> Script {
        self.ctx.build_script_with_hash_type(code, ScriptHashType::Data1, args).expect("deployed")
    }

    pub fn always_success_script(&mut self, args: &[u8]) -> Script {
        let op = self.always_success.clone();
        self.script(&op, Bytes::copy_from_slice(args))
    }

    /// Fresh key with a system lock `secp256k1_blake160_sighash_all` (args = blake160(pubkey)).
    pub fn new_key(&mut self) -> Key {
        self.seed += 1;
        let (privkey, pubkey) = Generator::non_crypto_safe_prng(self.seed).gen_keypair();
        let compressed: [u8; 33] = pubkey.serialize().try_into().expect("compressed pubkey");
        let secp = self.secp.clone();
        let lock = self.script(&secp, Bytes::copy_from_slice(&blake160(&compressed)));
        Key { privkey, pubkey: compressed, lock }
    }

    /// Lock args of the `csr-float-lock` for given hashes.
    pub fn float_lock_script(&mut self, owner: &[u8; 32], runner: &[u8; 32], max_fee: u64) -> Script {
        let mut args = Vec::with_capacity(72);
        args.extend_from_slice(owner);
        args.extend_from_slice(runner);
        args.extend_from_slice(&max_fee.to_le_bytes());
        let op = self.float_lock.clone();
        self.script(&op, args.into())
    }

    pub fn dual_lock_script(&mut self, owner: &[u8; 32], runner: &[u8; 32]) -> Script {
        let mut args = Vec::with_capacity(64);
        args.extend_from_slice(owner);
        args.extend_from_slice(runner);
        let op = self.dual_lock.clone();
        self.script(&op, args.into())
    }

    pub fn registry_type_script(&mut self, config_type_hash: &[u8; 32]) -> Script {
        let op = self.registry.clone();
        self.script(&op, Bytes::copy_from_slice(config_type_hash))
    }

    pub fn job_type_script(&mut self, service_code_hash: &[u8; 32]) -> Script {
        let op = self.job_type.clone();
        self.script(&op, Bytes::copy_from_slice(service_code_hash))
    }

    pub fn job_lock_script(&mut self, registry_type_hash: &[u8; 32]) -> Script {
        let op = self.job_lock.clone();
        self.script(&op, Bytes::copy_from_slice(registry_type_hash))
    }

    /// Creates a live cell with exactly `capacity` shannons (no occupied-capacity check here).
    pub fn cell(&mut self, capacity: u64, lock: Script, type_: Option<Script>, data: Bytes) -> OutPoint {
        let out = output(capacity, lock, type_);
        self.ctx.create_cell(out, data)
    }

    /// Creates a cell with the smallest capacity that satisfies the occupied-capacity rule, plus `extra`.
    pub fn min_cell(&mut self, extra: u64, lock: Script, type_: Option<Script>, data: Bytes) -> OutPoint {
        let cap = min_capacity(&lock, type_.as_ref(), data.len()) + extra;
        self.cell(cap, lock, type_, data)
    }

    /// Creates the monitor config cell (Type ID type script, data = pubkey ‖ clock_ms ‖ reserved).
    /// Returns (out point, type script hash = `config_type_hash`).
    pub fn config_cell(&mut self, monitor_pubkey: &[u8; 33], clock_ms: u64, monitor_lock: Script) -> (OutPoint, [u8; 32]) {
        let type_id = ckb_testtool::context::random_type_id_script();
        let hash: [u8; 32] = type_id.calc_script_hash().unpack();
        let data = config_data(monitor_pubkey, clock_ms);
        let op = self.min_cell(0, monitor_lock, Some(type_id), data);
        (op, hash)
    }

    /// Builds an input for a live cell with the given `since` (0 = none).
    pub fn input(&self, op: &OutPoint, since: u64) -> CellInput {
        CellInput::new_builder().previous_output(op.clone()).since(ckb_testtool::ckb_types::packed::Uint64::from(since)).build()
    }

    /// Completes cell deps (code of all scripts plus the secp256k1 data cell), then verifies.
    pub fn verify(&mut self, tx: TransactionView) -> Result<u64, CkbError> {
        let tx = self.complete(tx);
        self.ctx.verify_tx(&tx, MAX_CYCLES)
    }

    fn complete(&mut self, tx: TransactionView) -> TransactionView {
        let dep = packed::CellDep::new_builder().out_point(self.secp_data.clone()).build();
        self.ctx.complete_tx(tx.as_advanced_builder().cell_dep(dep).build())
    }

    /// Completes cell deps, signs the inputs locked by `keys`, then verifies.
    pub fn verify_signed(&mut self, tx: TransactionView, keys: &[&Key]) -> Result<u64, CkbError> {
        let tx = self.complete(tx);
        let tx = sign_tx(&self.ctx, tx, keys);
        self.ctx.verify_tx(&tx, MAX_CYCLES)
    }
}

impl Default for Env {
    fn default() -> Self {
        Self::new()
    }
}

/// Config cell data: `monitor_pubkey (33) ‖ clock_ms (u64 LE) ‖ reserved`.
pub fn config_data(monitor_pubkey: &[u8; 33], clock_ms: u64) -> Bytes {
    let mut d = Vec::with_capacity(41);
    d.extend_from_slice(monitor_pubkey);
    d.extend_from_slice(&clock_ms.to_le_bytes());
    d.into()
}

pub fn output(capacity: u64, lock: Script, type_: Option<Script>) -> CellOutput {
    CellOutput::new_builder()
        .capacity(ckb_testtool::ckb_types::packed::Uint64::from(capacity))
        .lock(lock)
        .type_(type_.pack())
        .build()
}

/// Occupied capacity (shannons) of a cell with this lock/type/data length.
pub fn min_capacity(lock: &Script, type_: Option<&Script>, data_len: usize) -> u64 {
    let out = CellOutput::new_builder().lock(lock.clone()).type_(type_.cloned().pack()).build();
    out.occupied_capacity(Capacity::bytes(data_len).unwrap()).unwrap().as_u64()
}

/// Signs every input whose lock is the system secp256k1 lock of one of `keys`
/// (all inputs of one key form one script group; sighash_all rules).
/// Existing witnesses are kept; missing ones are created.
pub fn sign_tx(ctx: &Context, tx: TransactionView, keys: &[&Key]) -> TransactionView {
    let n_in = tx.inputs().len();
    let mut witnesses: Vec<Bytes> = tx.witnesses().into_iter().map(|w| w.unpack()).collect();
    while witnesses.len() < n_in {
        witnesses.push(Bytes::new());
    }
    let lock_hash_of = |i: usize| -> Byte32 {
        let (cell, _) = ctx.get_cell(&tx.inputs().get(i).unwrap().previous_output()).expect("input cell");
        cell.lock().calc_script_hash()
    };
    for key in keys {
        let want = key.lock.calc_script_hash();
        let group: Vec<usize> = (0..n_in).filter(|&i| lock_hash_of(i) == want).collect();
        let Some(&first) = group.first() else { continue };
        let zero_lock = packed::WitnessArgs::new_builder().lock(Some(Bytes::from(vec![0u8; 65])).pack()).build();
        let mut hasher = new_blake2b();
        hasher.update(tx.hash().as_slice());
        let w0 = zero_lock.as_bytes();
        hasher.update(&(w0.len() as u64).to_le_bytes());
        hasher.update(&w0);
        for &i in &group[1..] {
            hasher.update(&(witnesses[i].len() as u64).to_le_bytes());
            hasher.update(&witnesses[i]);
        }
        for w in &witnesses[n_in..] {
            hasher.update(&(w.len() as u64).to_le_bytes());
            hasher.update(w);
        }
        let mut msg = [0u8; 32];
        hasher.finalize(&mut msg);
        let sig = key
            .privkey
            .sign_recoverable(&msg.into())
            .expect("sign")
            .serialize();
        witnesses[first] = packed::WitnessArgs::new_builder()
            .lock(Some(Bytes::from(sig)).pack())
            .build()
            .as_bytes();
    }
    tx.as_advanced_builder().set_witnesses(witnesses.into_iter().map(|w| w.pack()).collect()).build()
}

/// Minimal transaction skeleton with the given inputs/outputs (no cell deps; `Env::verify` adds them).
pub fn tx_with(inputs: Vec<CellInput>, outputs: Vec<(CellOutput, Bytes)>) -> TransactionView {
    let (outs, datas): (Vec<_>, Vec<_>) = outputs.into_iter().unzip();
    TransactionBuilder::default()
        .inputs(inputs)
        .outputs(outs)
        .outputs_data(datas.into_iter().map(|d| d.pack()).collect::<Vec<_>>())
        .build()
}
