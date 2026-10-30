//! `csr-dev`: developer CLI for the CSR devnet (deploy scripts, play the monitor, fund, sign).
//! Runs only against a `ckb_dev` or `ckb_testnet` node.

mod chain;
mod keys;
mod rpc;

use chain::{
    ckb_to_shannons, data_hash, lock_hash_hex, lock_matches, occupied, sign_sighash, Chain, Out,
    CKB,
};
use ckb_hash::new_blake2b;
use ckb_jsonrpc_types as json;
use ckb_types::{
    bytes::Bytes,
    core::ScriptHashType,
    packed::{self, Script},
    prelude::*,
};
use clap::{Parser, Subcommand};
use keys::{devnet_key, h, sighash_lock, Key};
use secp256k1::{Message, Secp256k1};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const CONTRACTS: [(&str, &str); 5] = [
    ("config_type", "csr-config-type"),
    ("dual_lock", "csr-dual-lock"),
    ("registry", "csr-registry"),
    ("job_type", "csr-job-type"),
    ("job_lock", "csr-job-lock"),
];
const WEEK_MS: u64 = 7 * 24 * 3600 * 1000;
/// SHA-256 of the vendored anyone-can-pay binary (`devtools/vendor/SHA256SUMS`).
const ACP_SHA256: &str = "5eca558b873dcf54e589fa2135a7a0a13defcfd87469cac7dfc6a38cbf57720a";
/// The public anyone-can-pay deployment of the testnet (a dep group with the secp256k1 data).
const ACP_TESTNET_CODE_HASH: &str =
    "0x3419a1c09eb2567f6552ee7a8ecffd64155cffe0f1796e6e61ec088d740c1356";
const ACP_TESTNET_DEP: &str = "0xec26b0f85ed839ece5f11c4c4e837ec359f5adc4420410f6453b1f6b60fb96a6";
/// Type hash of the Nervos DAO script (the same on every chain).
const DAO_TYPE_HASH: &str = "0x82d76d1b75fe2fd9a27dfbaa65a039221a380d76c926f378d3f81cf3e7e13f2e";
/// The config cell is a monitor-owned cell; its capacity pays the fees of the clock updates.
const CONFIG_CELL_CKB: u64 = 10_000;

#[derive(Parser)]
#[command(name = "csr-dev", about = "CSR developer tool (devnet / testnet only)")]
struct Cli {
    /// Node RPC URL
    #[arg(long, global = true, default_value = "http://127.0.0.1:18114")]
    rpc: String,
    /// Print one JSON object on stdout
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Deploy the five CSR scripts (contracts v3), the monitor config cell (type `csr-config-type`) and
    /// (devnet) the anyone-can-pay lock
    DeployContracts {
        #[arg(long, default_value = "deployments/devnet.json")]
        out: PathBuf,
        /// Directory with the built contracts
        #[arg(long, default_value = "contracts/build/release")]
        contracts_dir: PathBuf,
        /// Vendored anyone-can-pay binary (devnet only; its SHA-256 is checked)
        #[arg(long, default_value = "devtools/vendor/anyone_can_pay")]
        acp: PathBuf,
        /// Keep the config cell (and monitor key) of this deployments file instead of creating one; only
        /// a config cell of type `csr-config-type` with the same binary (a contracts v3 deployment)
        #[arg(long)]
        keep_config: Option<PathBuf>,
        /// With --keep-config: keep every CSR contract whose binary is unchanged (same data hash as
        /// its code_hash there) and whose code cell is live; deploy only the others
        #[arg(long, requires = "keep_config")]
        reuse_unchanged: bool,
    },
    /// Make a Nervos DAO deposit (output 0) owned by the key's secp256k1 lock
    DaoDeposit {
        #[arg(long)]
        ckb: String,
        /// `genesis` (the funding account) or 0x<private key>
        #[arg(long, default_value = "genesis")]
        key: String,
    },
    /// Set clock_ms of the config cell to the tip timestamp
    ClockUpdate {
        #[arg(long, default_value = "deployments/devnet.json")]
        deployments: PathBuf,
    },
    /// Sign a registry attestation (`CSR-ATTEST-v2`) with the monitor key
    Attest {
        #[arg(long)]
        peer_id: String,
        /// The registration's Nervos DAO deposit: 0x<tx hash>:<index>
        #[arg(long)]
        stake: String,
        #[arg(long)]
        service_pubkey: String,
        #[arg(long, value_parser = ["devnet", "testnet", "mainnet"])]
        network: Option<String>,
        #[arg(long)]
        expiry_ms: Option<u64>,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value = "deployments/devnet.json")]
        deployments: PathBuf,
    },
    /// Send CKB from the funding account to a secp256k1_blake160 lock
    Fund {
        #[arg(long)]
        lock_args: String,
        #[arg(long)]
        ckb: String,
    },
    /// Sign every lock group of an unsigned transaction file that the key can sign
    /// (secp256k1_blake160 and anyone-can-pay with the key's args)
    SignTx {
        #[arg(long = "in")]
        input: PathBuf,
        #[arg(long)]
        out: PathBuf,
        /// `genesis` (the funding account) or 0x<private key>
        #[arg(long, default_value = "genesis")]
        key: String,
        /// Deployments file naming the anyone-can-pay code hash (optional)
        #[arg(long, default_value = "deployments/devnet.json")]
        deployments: PathBuf,
    },
    /// Lock args and lock hash of the funding account (the operator wallet in tests)
    OwnerLockArgs,
    /// Sign a text like Neuron's "Sign message": blake2b-256("Nervos Message:" ‖ text)
    SignMessage {
        #[arg(long)]
        text: String,
        /// `genesis` (the funding account) or 0x<private key>
        #[arg(long, default_value = "genesis")]
        key: String,
    },
    /// Spend live plain cells of the key's secp256k1 lock back to that lock (fee taken from them)
    Reclaim {
        /// 0x<tx hash>:<index>; repeat for several cells
        #[arg(long, required = true)]
        outpoint: Vec<String>,
        /// `genesis` (the funding account) or 0x<private key>
        #[arg(long, default_value = "genesis")]
        key: String,
    },
    /// Deploy a service ELF as a code cell (code_hash = data hash)
    DeployService {
        #[arg(long)]
        elf: PathBuf,
    },
    /// Create a job cell (`spec/job-cell-standard.md`)
    Job {
        #[arg(long)]
        code_hash: String,
        #[arg(long)]
        tip_ckb: String,
        #[arg(long)]
        max_fee_shannons: u64,
        #[arg(long)]
        max_cycles: u64,
        /// ondemand | at-block:N | every:N
        #[arg(long)]
        trigger: String,
        #[arg(long)]
        runs: u32,
        #[arg(long, default_value = "0x")]
        payload: String,
        #[arg(long)]
        expiry_block: Option<u64>,
        #[arg(long)]
        refund_lock_args: Option<String>,
        #[arg(long, default_value = "deployments/devnet.json")]
        deployments: PathBuf,
    },
}

struct Ctx {
    chain: Chain,
    /// Funding account (devnet genesis key, or the testnet key).
    funder: Key,
    /// Monitor key (devnet monitor key; on testnet the same testnet key).
    monitor: Key,
}

fn testnet_key() -> Result<Key, String> {
    let k = std::env::var("CSR_DEV_TESTNET_KEY")
        .map_err(|_| "testnet needs the private key in env CSR_DEV_TESTNET_KEY".to_string())?;
    Key::from_hex(&k)
}

fn context(url: &str) -> Result<Ctx, String> {
    let chain = Chain::connect(url)?;
    let (funder, monitor) = if chain.devnet {
        (devnet_key("genesis")?, devnet_key("monitor")?)
    } else {
        (testnet_key()?, testnet_key()?)
    };
    Ok(Ctx {
        chain,
        funder,
        monitor,
    })
}

fn read_json(p: &Path) -> Result<Value, String> {
    let t = std::fs::read_to_string(p).map_err(|e| format!("cannot read {}: {e}", p.display()))?;
    serde_json::from_str(&t).map_err(|e| format!("{} is not JSON: {e}", p.display()))
}

fn write_json(p: &Path, v: &Value) -> Result<(), String> {
    if let Some(dir) = p.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let text = serde_json::to_string_pretty(v).map_err(|e| e.to_string())?;
    std::fs::write(p, text + "\n").map_err(|e| format!("cannot write {}: {e}", p.display()))
}

fn hex_arg(s: &str, len: usize, what: &str) -> Result<Vec<u8>, String> {
    let b = hex::decode(s.trim_start_matches("0x")).map_err(|_| format!("{what} is not hex"))?;
    if b.len() != len {
        return Err(format!("{what} must be {len} bytes"));
    }
    Ok(b)
}

fn config_data(pubkey: &[u8], clock_ms: u64) -> Vec<u8> {
    let mut d = pubkey.to_vec();
    d.extend_from_slice(&clock_ms.to_le_bytes());
    d.extend_from_slice(&[0u8; 8]); // reserved
    d
}

/// The config cell's type script: `csr-config-type` (`scripts.config_type`, hash type data2) with the
/// Type ID args; the built-in Type ID script in a contracts v2 file (no `scripts.config_type`).
fn config_type_script(dep: &Value) -> Result<Script, String> {
    let args = dep["config_cell"]["type_args"]
        .as_str()
        .ok_or("deployments file has no config_cell.type_args")?;
    let args = hex_arg(args, 32, "config_cell.type_args")?;
    if dep["scripts"]["config_type"].is_null() {
        return Ok(chain::type_id_script(&args));
    }
    let (code_hash, _) = dep_script(dep, "config_type")?;
    Ok(config_type_with(&code_hash, &args))
}

/// `csr-config-type` with code hash `code_hash` (its data hash), hash type data2 and `args`.
fn config_type_with(code_hash: &[u8], args: &[u8]) -> Script {
    Script::new_builder()
        .code_hash(packed::Byte32::from_slice(code_hash).expect("32 bytes"))
        .hash_type(packed::Byte::new(ScriptHashType::Data2 as u8))
        .args(Bytes::copy_from_slice(args).pack())
        .build()
}

/// Deploys a binary as a code cell of the funder; returns `(data hash, tx hash)`.
fn deploy_code(ctx: &Ctx, bin: Vec<u8>) -> Result<(String, String), String> {
    let lock = ctx.funder.lock();
    let cap = occupied(&lock, &None, bin.len()) * CKB;
    let code_hash = h(data_hash(&bin));
    let tx = ctx.chain.build_funded(
        &ctx.funder,
        vec![Out {
            capacity: cap,
            lock,
            type_: None,
            data: bin,
            type_id: false,
        }],
        vec![],
    )?;
    Ok((code_hash, ctx.chain.send_and_wait(&tx)?))
}

/// `scripts.anyone_can_pay`: the vendored binary as a code cell on the devnet, the public
/// deployment on the testnet.
fn deploy_acp(ctx: &Ctx, acp: &Path) -> Result<Value, String> {
    if !ctx.chain.devnet {
        return Ok(json!({
            "code_hash": ACP_TESTNET_CODE_HASH, "hash_type": "type",
            "tx_hash": ACP_TESTNET_DEP, "index": 0, "dep_type": "dep_group",
        }));
    }
    let bin = std::fs::read(acp).map_err(|e| format!("cannot read {}: {e}", acp.display()))?;
    let sha = hex::encode(Sha256::digest(&bin));
    if sha != ACP_SHA256 {
        return Err(format!(
            "{}: SHA-256 {sha} does not match the pinned {ACP_SHA256}",
            acp.display()
        ));
    }
    let (code_hash, tx_hash) = deploy_code(ctx, bin)?;
    Ok(json!({
        "code_hash": code_hash, "hash_type": "data1", "tx_hash": tx_hash, "index": 0, "dep_type": "code",
    }))
}

fn deploy(
    ctx: &Ctx,
    out: &Path,
    dir: &Path,
    acp: &Path,
    keep_config: Option<&Path>,
    reuse_unchanged: bool,
) -> Result<Value, String> {
    let chain = &ctx.chain;
    // Read everything first so that nothing is deployed when an input is missing.
    let kept = keep_config.map(read_json).transpose()?;
    if let Some(k) = &kept {
        if !k["config_cell"].is_object() || k["monitor_pubkey"].as_str().is_none() {
            return Err("--keep-config: the file has no config_cell / monitor_pubkey".into());
        }
        if k["scripts"]["config_type"].is_null() {
            return Err(
                "--keep-config: the config cell of this file is a plain Type ID cell (contracts v2); \
                 contracts v3 need a new config cell of type csr-config-type (deploy without \
                 --keep-config)"
                    .into(),
            );
        }
    }
    let mut bins = Vec::new();
    for (name, file) in CONTRACTS {
        let path = dir.join(file);
        let bin = std::fs::read(&path).map_err(|e| {
            format!(
                "cannot read {} (run scripts/build-contracts.sh): {e}",
                path.display()
            )
        })?;
        bins.push((name, bin));
    }
    if let Some(k) = &kept {
        // The registry and the job lock embed the csr-config-type code hash: a kept config cell must be of
        // the type of this build.
        let bin = &bins
            .iter()
            .find(|(n, _)| *n == "config_type")
            .expect("listed")
            .1;
        let kept_hash = k["scripts"]["config_type"]["code_hash"]
            .as_str()
            .unwrap_or("");
        if !kept_hash.eq_ignore_ascii_case(&h(data_hash(bin))) {
            return Err(format!(
                "--keep-config: the kept config cell is of csr-config-type {kept_hash}, but this build's \
                 csr-config-type is {}; deploy without --keep-config (a new config cell)",
                h(data_hash(bin))
            ));
        }
        let script = config_type_script(k)?;
        if !h(script.calc_script_hash().as_slice())
            .eq_ignore_ascii_case(k["config_cell"]["type_hash"].as_str().unwrap_or(""))
        {
            return Err(
                "--keep-config: config_cell.type_hash does not match csr-config-type".into(),
            );
        }
        if chain.cell_by_type(&script)?.is_none() {
            return Err("--keep-config: the kept config cell is not live".into());
        }
    }
    let mut scripts = serde_json::Map::new();
    let mut hashes = std::collections::HashMap::new();
    let mut reused = Vec::new();
    for (name, bin) in bins {
        if reuse_unchanged {
            if let Some(old) = kept.as_ref().map(|k| &k["scripts"][name]) {
                if reusable(chain, old, &bin)? {
                    hashes.insert(name, h(data_hash(&bin)));
                    scripts.insert(name.into(), old.clone());
                    reused.push(name);
                    continue;
                }
            }
        }
        let (code_hash, tx_hash) = deploy_code(ctx, bin)?;
        hashes.insert(name, code_hash.clone());
        scripts.insert(
            name.into(),
            json!({"code_hash": code_hash, "hash_type": "data2", "tx_hash": tx_hash, "index": 0}),
        );
    }
    scripts.insert("anyone_can_pay".into(), deploy_acp(ctx, acp)?);
    let (config_cell, config_id, monitor_pubkey) = match kept {
        Some(k) => {
            let id = hex_arg(
                k["config_cell"]["type_args"].as_str().unwrap_or(""),
                32,
                "config_cell.type_args",
            )?;
            (
                k["config_cell"].clone(),
                id,
                k["monitor_pubkey"].as_str().unwrap_or("").to_string(),
            )
        }
        None => {
            // Monitor config cell: type csr-config-type (args = Type ID), data = pubkey || clock_ms ||
            // reserved, lock = monitor key.
            let clock = chain.tip_timestamp()?;
            let data = config_data(&ctx.monitor.pubkey, clock);
            let code = hex_arg(&hashes["config_type"], 32, "hash")?;
            let tx = chain.build_funded(
                &ctx.funder,
                vec![Out {
                    capacity: CONFIG_CELL_CKB * CKB,
                    lock: ctx.monitor.lock(),
                    type_: Some(config_type_with(&code, &[0u8; 32])),
                    data,
                    type_id: true,
                }],
                vec![dep_script(&json!({"scripts": scripts.clone()}), "config_type")?.1],
            )?;
            let type_script = tx
                .output(0)
                .and_then(|o| o.type_().to_opt())
                .ok_or("config cell has no type")?;
            let config_type_hash = type_script.calc_script_hash();
            let tx_hash = chain.send_and_wait(&tx)?;
            (
                json!({
                    "type_hash": h(config_type_hash.as_slice()),
                    "type_args": h(type_script.args().raw_data()),
                    "tx_hash": tx_hash,
                    "index": 0,
                }),
                type_script.args().raw_data().to_vec(),
                h(ctx.monitor.pubkey),
            )
        }
    };
    let registry_type = Script::new_builder()
        .code_hash(
            packed::Byte32::from_slice(&hex_arg(&hashes["registry"], 32, "hash")?)
                .map_err(|e| e.to_string())?,
        )
        .hash_type(packed::Byte::new(ScriptHashType::Data2 as u8))
        // contracts v3: the registry's args are the config id (`spec/registry.md` §2)
        .args(Bytes::copy_from_slice(&config_id).pack())
        .build();
    let d = json!({
        "contracts": "v3",
        "network": chain.network_name(),
        "rpc": chain.rpc.url,
        "scripts": scripts,
        "config_cell": config_cell,
        "registry_type_hash": h(registry_type.calc_script_hash().as_slice()),
        "monitor_pubkey": monitor_pubkey,
    });
    write_json(out, &d)?;
    Ok(json!({"file": out, "deployments": d, "reused": reused}))
}

/// A kept script entry can be reused: its `code_hash` is the new binary's data hash and its code cell is live.
fn reusable(chain: &Chain, old: &Value, bin: &[u8]) -> Result<bool, String> {
    let Some(code_hash) = old["code_hash"].as_str() else {
        return Ok(false);
    };
    if !code_hash.eq_ignore_ascii_case(&h(data_hash(bin))) {
        return Ok(false);
    }
    let Some(tx_hash) = old["tx_hash"].as_str() else {
        return Ok(false);
    };
    let op = chain::out_point(tx_hash, old["index"].as_u64().unwrap_or(0) as u32)?;
    Ok(chain.live_cell(&op, false)?.is_some())
}

/// The key given as `genesis` (the funding account) or `0x<private key>`.
fn key_arg(ctx: &Ctx, key: &str) -> Result<Key, String> {
    if key == "genesis" {
        Key::from_hex(&hex::encode(ctx.funder.sk.secret_bytes()))
    } else {
        Key::from_hex(key)
    }
}

fn dao_deposit(ctx: &Ctx, ckb: &str, key: &str) -> Result<Value, String> {
    let key = key_arg(ctx, key)?;
    let capacity = ckb_to_shannons(ckb)?;
    let dao_hash = hex_arg(DAO_TYPE_HASH, 32, "DAO type hash")?;
    let dao = Script::new_builder()
        .code_hash(packed::Byte32::from_slice(&dao_hash).map_err(|e| e.to_string())?)
        .hash_type(packed::Byte::new(ScriptHashType::Type as u8))
        .build();
    let lock = key.lock();
    let type_ = Some(dao);
    if capacity < occupied(&lock, &type_, 8) * CKB {
        return Err(format!(
            "a DAO deposit needs at least {} CKB",
            occupied(&lock, &type_, 8)
        ));
    }
    let dep = ctx.chain.genesis_code_dep(&dao_hash)?;
    let tx = ctx.chain.build_funded(
        &key,
        vec![Out {
            capacity,
            lock,
            type_,
            data: vec![0u8; 8],
            type_id: false,
        }],
        vec![dep],
    )?;
    let tx_hash = ctx.chain.send_and_wait(&tx)?;
    Ok(json!({
        "tx_hash": tx_hash, "index": 0, "capacity_shannons": capacity, "lock_args": h(key.args),
    }))
}

fn clock_update(ctx: &Ctx, path: &Path) -> Result<Value, String> {
    let chain = &ctx.chain;
    let mut dep = read_json(path)?;
    let (cell, data, lock) = chain
        .cell_by_type(&config_type_script(&dep)?)?
        .ok_or("config cell not found on chain")?;
    if data.len() < 41 {
        return Err("config cell data is malformed".into());
    }
    let type_script = config_type_script(&dep)?;
    // csr-config-type lets clock_ms only grow or stay (D24 A).
    let old_clock = u64::from_le_bytes(data[33..41].try_into().unwrap());
    let clock = chain.tip_timestamp()?.max(old_clock);
    let mut new_data = data.clone();
    new_data[33..41].copy_from_slice(&clock.to_le_bytes());
    let secp = chain.secp_dep()?;
    // contracts v3: the config cell's type script runs, so its code cell is a dep.
    let type_dep = if dep["scripts"]["config_type"].is_null() {
        None
    } else {
        Some(dep_script(&dep, "config_type")?.1)
    };
    let build = |cap: u64| {
        let out = packed::CellOutput::new_builder()
            .capacity(Pack::<packed::Uint64>::pack(&cap))
            .lock(lock.clone())
            .type_(Some(type_script.clone()).pack())
            .build();
        ckb_types::core::TransactionBuilder::default()
            .cell_deps(type_dep.clone())
            .cell_dep(secp.clone())
            .input(packed::CellInput::new(cell.out_point.clone(), 0))
            .output(out)
            .output_data(Bytes::from(new_data.clone()).pack())
            .build()
    };
    if !lock_matches(&lock, &ctx.monitor) {
        return Err("the config cell is not locked by the monitor key".into());
    }
    let probe = sign_sighash(&build(cell.capacity), &[0], &ctx.monitor)?;
    let fee = chain::fee_for(&probe);
    let tx = sign_sighash(&build(cell.capacity - fee), &[0], &ctx.monitor)?;
    let tx_hash = chain.send_and_wait(&tx)?;
    dep["config_cell"]["tx_hash"] = json!(tx_hash);
    dep["config_cell"]["index"] = json!(0);
    write_json(path, &dep)?;
    Ok(json!({"clock_ms": clock, "tx_hash": tx_hash}))
}

/// Molecule `AttestationBodyV2` and the signed `CSR-ATTEST-v2` message (`spec/registry.md` §3).
fn attestation_message(
    peer_id: &[u8],
    pubkey: &[u8],
    network: u8,
    expiry_ms: u64,
    stake: &[u8; 36],
    registry_type_hash: &[u8; 32],
) -> [u8; 32] {
    let p = peer_id.len() as u32;
    let mut body = Vec::new();
    body.extend_from_slice(&(142 + p).to_le_bytes());
    for off in [28, 32 + p, 65 + p, 66 + p, 74 + p, 110 + p] {
        body.extend_from_slice(&off.to_le_bytes());
    }
    body.extend_from_slice(&p.to_le_bytes());
    body.extend_from_slice(peer_id);
    body.extend_from_slice(pubkey);
    body.push(network);
    body.extend_from_slice(&expiry_ms.to_le_bytes());
    body.extend_from_slice(stake);
    body.extend_from_slice(registry_type_hash);
    let mut hasher = new_blake2b();
    hasher.update(b"CSR-ATTEST-v2");
    hasher.update(&body);
    let mut out = [0u8; 32];
    hasher.finalize(&mut out);
    out
}

/// `0x<tx hash>:<index>` → the 36 bytes of a molecule `OutPoint`.
fn stake_arg(s: &str) -> Result<[u8; 36], String> {
    let bad = || format!("bad --stake '{s}': expected 0x<64 hex tx hash>:<index>");
    let (tx, i) = s.trim().split_once(':').ok_or_else(bad)?;
    let tx = hex_arg(tx, 32, "--stake tx hash").map_err(|_| bad())?;
    let i: u32 = i.parse().map_err(|_| bad())?;
    let mut out = [0u8; 36];
    out[..32].copy_from_slice(&tx);
    out[32..].copy_from_slice(&i.to_le_bytes());
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn attest(
    ctx: &Ctx,
    peer_id: &str,
    stake: &str,
    pubkey: &str,
    network: Option<&str>,
    expiry: Option<u64>,
    out: &Path,
    deployments: &Path,
) -> Result<Value, String> {
    let pk = hex_arg(pubkey, 33, "service pubkey")?;
    let stake_bytes = stake_arg(stake)?;
    let dep = read_json(deployments)?;
    if dep["scripts"]["config_type"].is_null() {
        return Err(format!(
            "{} is a contracts v2 deployment; CSR-ATTEST-v2 needs a v3 deployment (deploy-contracts)",
            deployments.display()
        ));
    }
    let registry_type_hash: [u8; 32] = hex_arg(
        dep["registry_type_hash"].as_str().unwrap_or(""),
        32,
        "registry_type_hash",
    )?
    .try_into()
    .expect("32 bytes");
    let network = network.unwrap_or(ctx.chain.network_name());
    let net_id = match network {
        "mainnet" => 0u8,
        "testnet" => 1,
        _ => 2,
    };
    let expiry_ms = match expiry {
        Some(e) => e,
        None => {
            let (_, data, _) = ctx
                .chain
                .cell_by_type(&config_type_script(&dep)?)?
                .ok_or("config cell not found on chain")?;
            if data.len() < 41 {
                return Err("config cell data is malformed".into());
            }
            u64::from_le_bytes(data[33..41].try_into().unwrap()) + WEEK_MS
        }
    };
    let msg = attestation_message(
        peer_id.as_bytes(),
        &pk,
        net_id,
        expiry_ms,
        &stake_bytes,
        &registry_type_hash,
    );
    let (rec, compact) = Secp256k1::new()
        .sign_ecdsa_recoverable(&Message::from_digest(msg), &ctx.monitor.sk)
        .serialize_compact();
    let mut sig = compact.to_vec();
    sig.push(rec.to_i32() as u8);
    let att = json!({
        "peer_id": peer_id,
        "service_pubkey": pubkey,
        "network": network,
        "network_id": net_id,
        "expiry_ms": expiry_ms,
        "stake": format!("{}:{}", h(&stake_bytes[..32]), u32::from_le_bytes(stake_bytes[32..].try_into().unwrap())),
        "registry_type_hash": h(registry_type_hash),
        "signature": h(sig),
        "monitor_pubkey": h(ctx.monitor.pubkey),
    });
    write_json(out, &att)?;
    Ok(json!({"file": out}))
}

fn fund(ctx: &Ctx, lock_args: &str, ckb: &str) -> Result<Value, String> {
    let args = hex_arg(lock_args, 20, "lock args")?;
    let capacity = ckb_to_shannons(ckb)?;
    let lock = sighash_lock(&args);
    if capacity < occupied(&lock, &None, 0) {
        return Err("amount is below the minimum cell capacity (61 CKB)".into());
    }
    let tx = ctx.chain.build_funded(
        &ctx.funder,
        vec![Out {
            capacity,
            lock,
            type_: None,
            data: vec![],
            type_id: false,
        }],
        vec![],
    )?;
    Ok(json!({"tx_hash": ctx.chain.send_and_wait(&tx)?}))
}

/// Anyone-can-pay code hashes the key may sign for: the public testnet one and the one of the
/// deployments file (if it exists).
fn acp_code_hashes(deployments: &Path) -> Vec<Vec<u8>> {
    let mut out = vec![hex_arg(ACP_TESTNET_CODE_HASH, 32, "").expect("constant")];
    if let Ok(d) = read_json(deployments) {
        let s = &d["scripts"]["anyone_can_pay"];
        if let Ok(c) = hex_arg(s["code_hash"].as_str().unwrap_or(""), 32, "") {
            out.push(c);
        }
    }
    out
}

/// Transaction file format of `ckb-cli` (`transaction`, `multisig_configs`, `signatures`).
/// Every lock group the key can sign gets one signature in its first input.
fn sign_tx(
    ctx: &Ctx,
    input: &Path,
    out: &Path,
    key: &str,
    deployments: &Path,
) -> Result<Value, String> {
    let key = key_arg(ctx, key)?;
    let acp = acp_code_hashes(deployments);
    let file = read_json(input)?;
    let raw = file.get("transaction").cloned().unwrap_or(file.clone());
    let jtx: json::Transaction =
        serde_json::from_value(raw).map_err(|e| format!("not a transaction: {e}"))?;
    let mut tx = packed::Transaction::from(jtx).into_view();
    // Lock groups in order of their first input.
    let mut groups: Vec<(Script, Vec<usize>)> = Vec::new();
    for (i, input) in tx.inputs().into_iter().enumerate() {
        let cell = ctx
            .chain
            .live_cell(&input.previous_output(), false)?
            .and_then(|c| c.cell)
            .ok_or_else(|| format!("input {i} is not a live cell"))?;
        let lock = Script::from(cell.output.lock);
        let is_acp = acp
            .iter()
            .any(|c| lock.code_hash().as_slice() == c.as_slice())
            && lock.args().raw_data() == key.args.as_slice();
        if !lock_matches(&lock, &key) && !is_acp {
            continue;
        }
        match groups.iter_mut().find(|(l, _)| *l == lock) {
            Some((_, g)) => g.push(i),
            None => groups.push((lock, vec![i])),
        }
    }
    if groups.is_empty() {
        return Err("the key cannot sign any input of this transaction".into());
    }
    let mut sigs = Vec::new();
    for (_, group) in &groups {
        tx = sign_sighash(&tx, group, &key)?;
        let sig = tx
            .witnesses()
            .get(group[0])
            .and_then(|w| packed::WitnessArgs::from_slice(&w.raw_data()).ok())
            .and_then(|w| w.lock().to_opt())
            .map(|l| h(l.raw_data()))
            .unwrap_or_default();
        sigs.push(sig);
    }
    let mut result = if file.get("transaction").is_some() {
        file
    } else {
        json!({"multisig_configs": {}, "signatures": {}})
    };
    result["transaction"] =
        serde_json::to_value(json::TransactionView::from(tx).inner).map_err(|e| e.to_string())?;
    if result.get("multisig_configs").is_none() {
        result["multisig_configs"] = json!({});
    }
    let signatures = result
        .get("signatures")
        .cloned()
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    result["signatures"] = signatures;
    result["signatures"][h(key.args)] = json!(sigs);
    write_json(out, &result)?;
    Ok(json!({"file": out, "groups": groups.len()}))
}

/// Neuron's "Sign message": digest `blake2b-256("Nervos Message:" ‖ text)` (personalization
/// `ckb-default-hash`), signature `r ‖ s ‖ v` with `v` 0–3.
fn sign_message(key: &Key, text: &str) -> Value {
    let mut hasher = new_blake2b();
    hasher.update(b"Nervos Message:");
    hasher.update(text.as_bytes());
    let mut digest = [0u8; 32];
    hasher.finalize(&mut digest);
    let (rec, compact) = Secp256k1::new()
        .sign_ecdsa_recoverable(&Message::from_digest(digest), &key.sk)
        .serialize_compact();
    let mut sig = compact.to_vec();
    sig.push(rec.to_i32() as u8);
    json!({"signature": h(sig), "lock_args": h(key.args)})
}

/// Spends the given live cells of the key's own secp256k1 lock (no type) into one cell of that lock.
fn reclaim(ctx: &Ctx, outpoints: &[String], key: &str) -> Result<Value, String> {
    let key = key_arg(ctx, key)?;
    let lock = key.lock();
    let mut cells = Vec::new();
    for s in outpoints {
        let (tx, i) = s
            .split_once(':')
            .ok_or_else(|| format!("bad out point {s}: expected 0x<tx hash>:<index>"))?;
        let i: u32 = i.parse().map_err(|_| format!("bad out point {s}"))?;
        let op = chain::out_point(tx, i)?;
        let cell = ctx
            .chain
            .live_cell(&op, false)?
            .and_then(|c| c.cell)
            .ok_or_else(|| format!("{s} is not a live cell"))?;
        if cell.output.type_.is_some() {
            return Err(format!("{s} has a type script; refusing to reclaim it"));
        }
        if !lock_matches(&Script::from(cell.output.lock), &key) {
            return Err(format!(
                "{s} is not locked by the key's secp256k1 lock; refusing to reclaim it"
            ));
        }
        let cap: u64 = cell.output.capacity.into();
        cells.push((op, cap));
    }
    if cells
        .iter()
        .any(|(a, _)| cells.iter().filter(|(b, _)| a == b).count() > 1)
    {
        return Err("an out point is given twice".into());
    }
    let total: u64 = cells.iter().map(|(_, c)| c).sum();
    let tx = ctx.chain.build_spend(&key, &cells, total)?;
    let out: u64 = tx
        .outputs()
        .get(0)
        .map(|o| o.capacity().unpack())
        .unwrap_or(0);
    if out < occupied(&lock, &None, 0) * CKB {
        return Err("the cells cannot pay a 61 CKB cell and the fee".into());
    }
    let tx_hash = ctx.chain.send_and_wait(&tx)?;
    Ok(json!({"tx_hash": tx_hash, "capacity_shannons": out}))
}

fn owner_lock_args(ctx: &Ctx) -> Result<Value, String> {
    let lock = ctx.funder.lock();
    Ok(json!({"lock_args": h(ctx.funder.args), "lock_hash": lock_hash_hex(&lock)}))
}

fn deploy_service(ctx: &Ctx, elf: &Path) -> Result<Value, String> {
    let bin = std::fs::read(elf).map_err(|e| format!("cannot read {}: {e}", elf.display()))?;
    let lock = ctx.funder.lock();
    let cap = occupied(&lock, &None, bin.len()) * CKB;
    let tx = ctx.chain.build_funded(
        &ctx.funder,
        vec![Out {
            capacity: cap,
            lock,
            type_: None,
            data: bin.clone(),
            type_id: false,
        }],
        vec![],
    )?;
    let tx_hash = ctx.chain.send_and_wait(&tx)?;
    Ok(json!({"code_hash": h(data_hash(&bin)), "tx_hash": tx_hash, "index": 0}))
}

/// Molecule table: header (total size, field offsets) followed by the fields.
fn mol_table(fields: &[Vec<u8>]) -> Vec<u8> {
    let header = 4 + 4 * fields.len();
    let total = header + fields.iter().map(Vec::len).sum::<usize>();
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

/// Molecule `Trigger` union from `ondemand | at-block:N | every:N`.
fn trigger_molecule(s: &str) -> Result<Vec<u8>, String> {
    let (id, arg) = match s.split_once(':') {
        None if s == "ondemand" => (0u32, None),
        Some(("at-block", n)) => (1, Some(n)),
        Some(("every", n)) => (4, Some(n)),
        _ => return Err("trigger must be ondemand, at-block:N or every:N".into()),
    };
    let mut out = id.to_le_bytes().to_vec();
    match arg {
        None => out.extend(mol_table(&[])),
        Some(n) => {
            let n: u64 = n.parse().map_err(|_| format!("bad trigger number '{n}'"))?;
            out.extend(mol_table(&[n.to_le_bytes().to_vec()]));
        }
    }
    Ok(out)
}

fn dep_script(dep: &Value, name: &str) -> Result<(Vec<u8>, packed::CellDep), String> {
    let s = &dep["scripts"][name];
    let code_hash = hex_arg(s["code_hash"].as_str().unwrap_or(""), 32, "code_hash")?;
    let op = chain::out_point(
        s["tx_hash"].as_str().unwrap_or(""),
        s["index"].as_u64().unwrap_or(0) as u32,
    )?;
    let cd = packed::CellDep::new_builder()
        .out_point(op)
        .dep_type(packed::Byte::new(ckb_types::core::DepType::Code as u8))
        .build();
    Ok((code_hash, cd))
}

#[allow(clippy::too_many_arguments)]
fn job(
    ctx: &Ctx,
    code_hash: &str,
    tip: &str,
    max_fee: u64,
    max_cycles: u64,
    trigger: &str,
    runs: u32,
    payload: &str,
    expiry: Option<u64>,
    refund_args: Option<&str>,
    deployments: &Path,
) -> Result<Value, String> {
    let dep = read_json(deployments)?;
    let service = hex_arg(code_hash, 32, "--code-hash")?;
    let tip_shannons = ckb_to_shannons(tip)?;
    let payload =
        hex::decode(payload.trim_start_matches("0x")).map_err(|_| "payload is not hex")?;
    let trig = trigger_molecule(trigger)?;
    if runs == 0 || (runs > 1 && !trigger.starts_with("every:")) {
        return Err("runs must be >= 1, and > 1 only for every:N".into());
    }
    let expiry_block = match expiry {
        Some(e) => e,
        None => ctx.chain.rpc.tip_number()? + 100_000,
    };
    let refund_lock = match refund_args {
        Some(a) => sighash_lock(&hex_arg(a, 20, "--refund-lock-args")?),
        None => ctx.funder.lock(),
    };
    let data = mol_table(&[
        trig,
        tip_shannons.to_le_bytes().to_vec(),
        max_fee.to_le_bytes().to_vec(),
        max_cycles.to_le_bytes().to_vec(),
        [(payload.len() as u32).to_le_bytes().to_vec(), payload].concat(),
        expiry_block.to_le_bytes().to_vec(),
        refund_lock.calc_script_hash().as_slice().to_vec(),
        runs.to_le_bytes().to_vec(),
    ]);
    let (type_hash, type_dep) = dep_script(&dep, "job_type")?;
    let (lock_hash, _) = dep_script(&dep, "job_lock")?;
    let registry_type_hash = hex_arg(
        dep["registry_type_hash"].as_str().unwrap_or(""),
        32,
        "registry_type_hash",
    )?;
    let script = |code: &[u8], args: &[u8]| {
        Script::new_builder()
            .code_hash(packed::Byte32::from_slice(code).expect("32 bytes"))
            .hash_type(packed::Byte::new(ScriptHashType::Data2 as u8))
            .args(Bytes::copy_from_slice(args).pack())
            .build()
    };
    let lock = script(&lock_hash, &registry_type_hash);
    let type_ = Some(script(&type_hash, &service));
    let capacity =
        occupied(&lock, &type_, data.len()) * CKB + runs as u64 * (tip_shannons + max_fee) + CKB;
    let tx = ctx.chain.build_funded(
        &ctx.funder,
        vec![Out {
            capacity,
            lock,
            type_,
            data,
            type_id: false,
        }],
        vec![type_dep],
    )?;
    let tx_hash = ctx.chain.send_and_wait(&tx)?;
    Ok(json!({"tx_hash": tx_hash, "index": 0, "capacity_shannons": capacity}))
}

fn run(cli: &Cli) -> Result<Value, String> {
    // A message is signed without a node, unless the key is the chain's funding account.
    if let Command::SignMessage { text, key } = &cli.command {
        let key = if key == "genesis" {
            match context(&cli.rpc) {
                Ok(ctx) => key_arg(&ctx, key)?,
                Err(_) => devnet_key("genesis")?,
            }
        } else {
            Key::from_hex(key)?
        };
        return Ok(sign_message(&key, text));
    }
    let ctx = context(&cli.rpc)?;
    match &cli.command {
        Command::DeployContracts {
            out,
            contracts_dir,
            acp,
            keep_config,
            reuse_unchanged,
        } => deploy(
            &ctx,
            out,
            contracts_dir,
            acp,
            keep_config.as_deref(),
            *reuse_unchanged,
        ),
        Command::SignMessage { .. } => unreachable!("handled above"),
        Command::Reclaim { outpoint, key } => reclaim(&ctx, outpoint, key),
        Command::DaoDeposit { ckb, key } => dao_deposit(&ctx, ckb, key),
        Command::ClockUpdate { deployments } => clock_update(&ctx, deployments),
        Command::Attest {
            peer_id,
            stake,
            service_pubkey,
            network,
            expiry_ms,
            out,
            deployments,
        } => attest(
            &ctx,
            peer_id,
            stake,
            service_pubkey,
            network.as_deref(),
            *expiry_ms,
            out,
            deployments,
        ),
        Command::Fund { lock_args, ckb } => fund(&ctx, lock_args, ckb),
        Command::SignTx {
            input,
            out,
            key,
            deployments,
        } => sign_tx(&ctx, input, out, key, deployments),
        Command::OwnerLockArgs => owner_lock_args(&ctx),
        Command::DeployService { elf } => deploy_service(&ctx, elf),
        Command::Job {
            code_hash,
            tip_ckb,
            max_fee_shannons,
            max_cycles,
            trigger,
            runs,
            payload,
            expiry_block,
            refund_lock_args,
            deployments,
        } => job(
            &ctx,
            code_hash,
            tip_ckb,
            *max_fee_shannons,
            *max_cycles,
            trigger,
            *runs,
            payload,
            *expiry_block,
            refund_lock_args.as_deref(),
            deployments,
        ),
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(v) => {
            if cli.json {
                println!("{v}");
            } else {
                println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("csr-dev: {e}");
            if cli.json {
                println!("{}", json!({"error": e}));
            }
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts() {
        assert_eq!(ckb_to_shannons("100").unwrap(), 100 * CKB);
        assert_eq!(ckb_to_shannons("0.5").unwrap(), CKB / 2);
        assert!(ckb_to_shannons("1.123456789").is_err());
        assert!(ckb_to_shannons("x").is_err());
    }

    #[test]
    fn sign_message_matches_the_neuron_format() {
        let key = Key::from_hex(&"42".repeat(32)).unwrap();
        let v = sign_message(&key, "Message from CSR");
        let sig = hex::decode(&v["signature"].as_str().unwrap()[2..]).unwrap();
        assert_eq!(sig.len(), 65);
        let mut hasher = new_blake2b();
        hasher.update(b"Nervos Message:Message from CSR");
        let mut digest = [0u8; 32];
        hasher.finalize(&mut digest);
        // The Neuron vector's digest (`spec/registry.md` §4.2).
        assert_eq!(
            hex::encode(digest),
            "2bb15d28fd5e98471846af625ba4e735e60491793d2159cd595331cb81a1a8a0"
        );
        let rec = secp256k1::ecdsa::RecoveryId::from_i32(sig[64] as i32).unwrap();
        let rs = secp256k1::ecdsa::RecoverableSignature::from_compact(&sig[..64], rec).unwrap();
        let pk = Secp256k1::new()
            .recover_ecdsa(&Message::from_digest(digest), &rs)
            .unwrap();
        assert_eq!(pk.serialize(), key.pubkey);
        assert_eq!(v["lock_args"], h(key.args));
    }

    #[test]
    fn attestation_message_depends_on_every_field() {
        let pk = [2u8; 33];
        let (st, rt) = ([5u8; 36], [4u8; 32]);
        let m = attestation_message(b"Qm1", &pk, 2, 10, &st, &rt);
        assert_ne!(m, attestation_message(b"Qm2", &pk, 2, 10, &st, &rt));
        assert_ne!(m, attestation_message(b"Qm1", &pk, 1, 10, &st, &rt));
        assert_ne!(m, attestation_message(b"Qm1", &pk, 2, 11, &st, &rt));
        assert_ne!(m, attestation_message(b"Qm1", &pk, 2, 10, &[6u8; 36], &rt));
        assert_ne!(m, attestation_message(b"Qm1", &pk, 2, 10, &st, &[3u8; 32]));
    }

    #[test]
    fn attestation_message_matches_the_shared_vector() {
        let v: Value = serde_json::from_str(include_str!(
            "../../../acceptance/aiworker-contracts-v3/registry-v3.json"
        ))
        .unwrap();
        let hx = |k: &str| hex::decode(v[k].as_str().unwrap()).unwrap();
        let mut stake = [0x5au8; 36];
        stake[32..].copy_from_slice(&3u32.to_le_bytes());
        assert_eq!(stake_arg(v["stake"].as_str().unwrap()).unwrap(), stake);
        let m = attestation_message(
            v["peer_id"].as_str().unwrap().as_bytes(),
            &hx("service_pubkey"),
            1,
            1_800_000_000_000,
            &stake,
            &hx("registry_type_hash").try_into().unwrap(),
        );
        assert_eq!(
            hex::encode(m),
            v["attestation_message_v2"].as_str().unwrap()
        );
    }
}
