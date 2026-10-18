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
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const CONTRACTS: [(&str, &str); 5] = [
    ("float_lock", "csr-float-lock"),
    ("dual_lock", "csr-dual-lock"),
    ("registry", "csr-registry"),
    ("job_type", "csr-job-type"),
    ("job_lock", "csr-job-lock"),
];
const WEEK_MS: u64 = 7 * 24 * 3600 * 1000;
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
    /// Deploy the five CSR scripts and the monitor config cell
    DeployContracts {
        #[arg(long, default_value = "deployments/devnet.json")]
        out: PathBuf,
        /// Directory with the built contracts
        #[arg(long, default_value = "contracts/build/release")]
        contracts_dir: PathBuf,
    },
    /// Set clock_ms of the config cell to the tip timestamp
    ClockUpdate {
        #[arg(long, default_value = "deployments/devnet.json")]
        deployments: PathBuf,
    },
    /// Sign a registry attestation with the monitor key
    Attest {
        #[arg(long)]
        peer_id: String,
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
    /// Sign every input of an unsigned transaction file that the key can sign
    SignTx {
        #[arg(long = "in")]
        input: PathBuf,
        #[arg(long)]
        out: PathBuf,
        /// `genesis` (the funding account) or 0x<private key>
        #[arg(long, default_value = "genesis")]
        key: String,
    },
    /// Lock args and lock hash of the funding account (the operator wallet in tests)
    OwnerLockArgs,
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

fn config_type_script(dep: &Value) -> Result<Script, String> {
    let args = dep["config_cell"]["type_args"]
        .as_str()
        .ok_or("deployments file has no config_cell.type_args")?;
    Ok(chain::type_id_script(&hex_arg(
        args,
        32,
        "config_cell.type_args",
    )?))
}

fn deploy(ctx: &Ctx, out: &Path, dir: &Path) -> Result<Value, String> {
    let chain = &ctx.chain;
    let mut scripts = serde_json::Map::new();
    let mut hashes = std::collections::HashMap::new();
    for (name, file) in CONTRACTS {
        let path = dir.join(file);
        let bin = std::fs::read(&path).map_err(|e| {
            format!(
                "cannot read {} (run scripts/build-contracts.sh): {e}",
                path.display()
            )
        })?;
        let lock = ctx.funder.lock();
        let cap = occupied(&lock, &None, bin.len()) * CKB;
        let tx = chain.build_funded(
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
        let tx_hash = chain.send_and_wait(&tx)?;
        let code_hash = h(data_hash(&bin));
        hashes.insert(name, code_hash.clone());
        scripts.insert(
            name.into(),
            json!({"code_hash": code_hash, "hash_type": "data2", "tx_hash": tx_hash, "index": 0}),
        );
    }
    // Monitor config cell: Type ID, data = pubkey || clock_ms || reserved, lock = monitor key.
    let clock = chain.tip_timestamp()?;
    let data = config_data(&ctx.monitor.pubkey, clock);
    let tx = chain.build_funded(
        &ctx.funder,
        vec![Out {
            capacity: CONFIG_CELL_CKB * CKB,
            lock: ctx.monitor.lock(),
            type_: None,
            data,
            type_id: true,
        }],
        vec![],
    )?;
    let type_script = tx
        .output(0)
        .and_then(|o| o.type_().to_opt())
        .ok_or("config cell has no type")?;
    let config_type_hash = type_script.calc_script_hash();
    let tx_hash = chain.send_and_wait(&tx)?;
    let registry_type = Script::new_builder()
        .code_hash(
            packed::Byte32::from_slice(&hex_arg(&hashes["registry"], 32, "hash")?)
                .map_err(|e| e.to_string())?,
        )
        .hash_type(packed::Byte::new(ScriptHashType::Data2 as u8))
        .args(Bytes::copy_from_slice(config_type_hash.as_slice()).pack())
        .build();
    let d = json!({
        "network": chain.network_name(),
        "rpc": chain.rpc.url,
        "scripts": scripts,
        "config_cell": {
            "type_hash": h(config_type_hash.as_slice()),
            "type_args": h(type_script.args().raw_data()),
            "tx_hash": tx_hash,
            "index": 0,
        },
        "registry_type_hash": h(registry_type.calc_script_hash().as_slice()),
        "monitor_pubkey": h(ctx.monitor.pubkey),
    });
    write_json(out, &d)?;
    Ok(json!({"file": out, "deployments": d}))
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
    let clock = chain.tip_timestamp()?;
    let mut new_data = data.clone();
    new_data[33..41].copy_from_slice(&clock.to_le_bytes());
    let secp = chain.secp_dep()?;
    let build = |cap: u64| {
        let out = packed::CellOutput::new_builder()
            .capacity(Pack::<packed::Uint64>::pack(&cap))
            .lock(lock.clone())
            .type_(Some(type_script.clone()).pack())
            .build();
        ckb_types::core::TransactionBuilder::default()
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

/// Molecule `AttestationBody` and the signed message (`spec/registry.md` §3).
fn attestation_message(peer_id: &[u8], pubkey: &[u8], network: u8, expiry_ms: u64) -> [u8; 32] {
    let p = peer_id.len() as u32;
    let mut body = Vec::new();
    body.extend_from_slice(&(20 + 4 + p + 33 + 1 + 8).to_le_bytes());
    for off in [20, 24 + p, 57 + p, 58 + p] {
        body.extend_from_slice(&off.to_le_bytes());
    }
    body.extend_from_slice(&p.to_le_bytes());
    body.extend_from_slice(peer_id);
    body.extend_from_slice(pubkey);
    body.push(network);
    body.extend_from_slice(&expiry_ms.to_le_bytes());
    let mut hasher = new_blake2b();
    hasher.update(b"CSR-ATTEST-v1");
    hasher.update(&body);
    let mut out = [0u8; 32];
    hasher.finalize(&mut out);
    out
}

#[allow(clippy::too_many_arguments)]
fn attest(
    ctx: &Ctx,
    peer_id: &str,
    pubkey: &str,
    network: Option<&str>,
    expiry: Option<u64>,
    out: &Path,
    deployments: &Path,
) -> Result<Value, String> {
    let pk = hex_arg(pubkey, 33, "service pubkey")?;
    let network = network.unwrap_or(ctx.chain.network_name());
    let net_id = match network {
        "mainnet" => 0u8,
        "testnet" => 1,
        _ => 2,
    };
    let expiry_ms = match expiry {
        Some(e) => e,
        None => {
            let dep = read_json(deployments)?;
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
    let msg = attestation_message(peer_id.as_bytes(), &pk, net_id, expiry_ms);
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

/// Transaction file format of `ckb-cli` (`transaction`, `multisig_configs`, `signatures`).
fn sign_tx(ctx: &Ctx, input: &Path, out: &Path, key: &str) -> Result<Value, String> {
    let key = if key == "genesis" {
        Key::from_hex(&hex::encode(ctx.funder.sk.secret_bytes()))?
    } else {
        Key::from_hex(key)?
    };
    let file = read_json(input)?;
    let raw = file.get("transaction").cloned().unwrap_or(file.clone());
    let jtx: json::Transaction =
        serde_json::from_value(raw).map_err(|e| format!("not a transaction: {e}"))?;
    let tx = packed::Transaction::from(jtx).into_view();
    let mut group = Vec::new();
    for (i, input) in tx.inputs().into_iter().enumerate() {
        let cell = ctx
            .chain
            .live_cell(&input.previous_output(), false)?
            .and_then(|c| c.cell)
            .ok_or_else(|| format!("input {i} is not a live cell"))?;
        if lock_matches(&Script::from(cell.output.lock), &key) {
            group.push(i);
        }
    }
    if group.is_empty() {
        return Err("the key cannot sign any input of this transaction".into());
    }
    let signed = sign_sighash(&tx, &group, &key)?;
    let sig = signed
        .witnesses()
        .get(group[0])
        .and_then(|w| packed::WitnessArgs::from_slice(&w.raw_data()).ok())
        .and_then(|w| w.lock().to_opt())
        .map(|l| h(l.raw_data()))
        .unwrap_or_default();
    let mut result = if file.get("transaction").is_some() {
        file
    } else {
        json!({"multisig_configs": {}, "signatures": {}})
    };
    result["transaction"] = serde_json::to_value(json::TransactionView::from(signed).inner)
        .map_err(|e| e.to_string())?;
    if result.get("multisig_configs").is_none() {
        result["multisig_configs"] = json!({});
    }
    let signatures = result
        .get("signatures")
        .cloned()
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    result["signatures"] = signatures;
    result["signatures"][h(key.args)] = json!([sig]);
    write_json(out, &result)?;
    Ok(json!({"file": out}))
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
    let ctx = context(&cli.rpc)?;
    match &cli.command {
        Command::DeployContracts { out, contracts_dir } => deploy(&ctx, out, contracts_dir),
        Command::ClockUpdate { deployments } => clock_update(&ctx, deployments),
        Command::Attest {
            peer_id,
            service_pubkey,
            network,
            expiry_ms,
            out,
            deployments,
        } => attest(
            &ctx,
            peer_id,
            service_pubkey,
            network.as_deref(),
            *expiry_ms,
            out,
            deployments,
        ),
        Command::Fund { lock_args, ckb } => fund(&ctx, lock_args, ckb),
        Command::SignTx { input, out, key } => sign_tx(&ctx, input, out, key),
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
    fn attestation_message_depends_on_every_field() {
        let pk = [2u8; 33];
        let m = attestation_message(b"Qm1", &pk, 2, 10);
        assert_ne!(m, attestation_message(b"Qm2", &pk, 2, 10));
        assert_ne!(m, attestation_message(b"Qm1", &pk, 1, 10));
        assert_ne!(m, attestation_message(b"Qm1", &pk, 2, 11));
    }
}
