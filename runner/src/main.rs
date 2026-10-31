mod address;
mod app;
mod attest;
mod chain;
mod config;
mod consent;
mod db;
mod identity;
mod jobs;
mod keeper;
mod logs;
mod monitor;
mod network;
mod node;
mod password;
mod ping;
mod registry;
mod renew;
mod resources;
mod run;
mod runtime;
mod service;
mod storage;
mod update;
mod wallet;
mod web;

/// Scratch directory for unit tests (`/tmp` may be read-only).
#[cfg(test)]
fn test_tmp() -> tempfile::TempDir {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/unit-tmp");
    std::fs::create_dir_all(&base).unwrap();
    tempfile::Builder::new().tempdir_in(base).unwrap()
}

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "csr",
    about = "CSR - CKB Service Runner",
    version
)]
struct Cli {
    /// Data root directory (default: $HOME/.csr)
    #[arg(long, global = true)]
    data_root: Option<PathBuf>,
    /// Config file (default: <data-root>/csr.toml)
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    /// Network override: testnet or mainnet (devnet in debug builds)
    #[arg(long, global = true, value_parser = NETWORKS)]
    network: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[cfg(debug_assertions)]
const NETWORKS: [&str; 3] = ["testnet", "mainnet", "devnet"];
#[cfg(not(debug_assertions))]
const NETWORKS: [&str; 2] = ["testnet", "mainnet"];

#[derive(Subcommand)]
enum Command {
    /// Print the version
    Version,
    /// Config tools
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// One R2 check against the node
    Check {
        #[arg(long)]
        json: bool,
    },
    /// Inspect the configured node
    Detect {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        probe: bool,
    },
    /// Store a dashboard password hash in the config
    SetPassword {
        #[arg(long)]
        password_stdin: bool,
    },
    /// Service tools
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
    /// Storage tools
    Storage {
        #[command(subcommand)]
        action: StorageAction,
    },
    /// Print the node's service identity (creates the keys if missing)
    Identity {
        #[arg(long)]
        json: bool,
    },
    /// The runner key cell
    RunnerCell {
        #[command(subcommand)]
        action: RunnerCellAction,
    },
    /// The runner wallet: pays the registration and fees, receives the tips
    Wallet {
        #[command(subcommand)]
        action: WalletAction,
    },
    /// The registry cell
    Register {
        #[command(subcommand)]
        action: RegisterAction,
    },
    /// Transactions
    Tx {
        #[command(subcommand)]
        action: TxAction,
    },
    /// Keeper jobs
    Jobs {
        #[command(subcommand)]
        action: JobsAction,
    },
    /// Check for, stage and verify updates (signed GitHub releases)
    Update {
        #[command(subcommand)]
        action: UpdateAction,
    },
    /// Run the service
    Run {
        #[arg(long)]
        confirm_mainnet: bool,
    },
}

#[derive(Subcommand)]
enum UpdateAction {
    /// Ask GitHub for the latest release and compare it with this version
    Check {
        #[arg(long)]
        json: bool,
    },
    /// Download and verify the latest release and stage it in <data root>/<network>/update/
    Stage {
        #[arg(long)]
        json: bool,
    },
    /// Check staged files with this binary's release key and version (no network); exit 0 if valid
    Verify {
        #[arg(long)]
        dir: PathBuf,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum JobsAction {
    /// Discover job cells on chain and show what would run and what is excluded
    Plan {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum RunnerCellAction {
    /// Show the cell(s) under the runner lock
    Status {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum WalletAction {
    /// Address, balance and sweep settings of the runner wallet
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Send CKB from the runner wallet to a full-format address of this network
    Send {
        #[arg(long)]
        to: String,
        #[arg(long)]
        ckb: String,
        #[arg(long)]
        json: bool,
    },
    /// Sweep sweep_amount_ckb to payout.sweep_address if the wallet holds sweep_threshold_ckb
    Sweep {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum RegisterAction {
    /// The text the stake owner signs in its wallet ("Sign message") to consent to this registration
    ConsentMessage {
        /// The Nervos DAO deposit staked for this registration: 0x<tx hash>:<index>
        #[arg(long)]
        stake: String,
        #[arg(long)]
        json: bool,
    },
    /// Who signed a message (lock args and address of the signer); needs no node
    VerifyConsent {
        #[arg(long)]
        message: String,
        #[arg(long)]
        signature: String,
        #[arg(long)]
        json: bool,
    },
    /// Register (or update the registration) with the stake and the stake owner's consent signature
    Create {
        /// The Nervos DAO deposit staked for this registration: 0x<tx hash>:<index>
        #[arg(long)]
        stake: String,
        /// The stake owner's signature of the consent message (0x + 65 bytes)
        #[arg(long)]
        consent: String,
        /// The monitor's attestation (file); without it the monitor is asked
        #[arg(long)]
        attestation: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Renew the monitor's attestation of the registry cell (no stake owner consent needed)
    Refresh {
        #[arg(long)]
        json: bool,
    },
    /// Remove the registry cell; its capacity returns to the runner wallet
    Remove {
        #[arg(long)]
        json: bool,
    },
    /// Request the monitor's attestation for this node and write it to a file
    Attest {
        /// The Nervos DAO deposit staked for this registration: 0x<tx hash>:<index>
        #[arg(long)]
        stake: String,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Show the registry cell of this node
    Status {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum TxAction {
    /// Broadcast a signed transaction file through the local node
    Submit {
        file: PathBuf,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum ServiceAction {
    /// Run a service once in the sandbox (developer tool, ignores approval)
    RunLocal {
        #[arg(long)]
        manifest: PathBuf,
        /// Job payload as 0x-prefixed hex
        #[arg(long)]
        payload: String,
        #[arg(long)]
        max_cycles: Option<u64>,
        #[arg(long)]
        max_memory_mb: Option<u32>,
        #[arg(long)]
        max_wall_seconds: Option<u32>,
        /// TIP:EPOCH:MEDIAN_MS
        #[arg(long)]
        block_info: Option<String>,
    },
    /// Register a local service (prints {code_hash, name})
    AddLocal {
        #[arg(long)]
        manifest: PathBuf,
    },
    /// Register a service from its code cell (prints {code_hash, name})
    AddOnchain {
        /// 0x<tx hash>:<index>
        #[arg(long)]
        outpoint: String,
    },
    /// List registered services
    List {
        #[arg(long)]
        json: bool,
    },
    /// Approve a service (adds it to runtime.allowed_services)
    Approve { code_hash: String },
    /// Revoke the approval of a service
    Revoke { code_hash: String },
    /// Enable an approved service
    Enable { code_hash: String },
    /// Disable an approved service
    Disable { code_hash: String },
}

#[derive(Subcommand)]
enum StorageAction {
    /// Apply the delete_on_consume rules as if the outpoint had been consumed
    SimulateConsumed {
        /// 0x<64 hex>:<index>
        #[arg(long)]
        outpoint: String,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Validate the config file
    Validate,
}

fn data_root(cli: &Cli) -> PathBuf {
    cli.data_root
        .clone()
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".csr"))
}

fn load_config(cli: &Cli) -> Result<config::Config, String> {
    let path = cli
        .config
        .clone()
        .unwrap_or_else(|| data_root(cli).join("csr.toml"));
    let mut cfg = config::Config::load(&path)?;
    if let Some(n) = &cli.network {
        cfg.network = n.clone();
        cfg.validate()?;
    }
    Ok(cfg)
}

/// Runs a chain command: loads the config and identity, connects to the node and prints the result.
fn chain_cmd<F, Fut>(cli: &Cli, json: bool, f: F) -> ExitCode
where
    F: FnOnce(chain::Chain) -> Fut,
    Fut: std::future::Future<Output = Result<serde_json::Value, String>>,
{
    node_cmd(cli, json, |c, _, _| f(c))
}

/// As `chain_cmd`, also handing over the config, the identity and the network's data directory.
fn node_cmd<F, Fut>(cli: &Cli, json: bool, f: F) -> ExitCode
where
    F: FnOnce(chain::Chain, config::Config, (identity::Identity, PathBuf)) -> Fut,
    Fut: std::future::Future<Output = Result<serde_json::Value, String>>,
{
    node_cmd_with(cli, json, false, f)
}

/// As `node_cmd` for the runner wallet commands, which work before the CSR scripts are deployed.
fn wallet_cmd<F, Fut>(cli: &Cli, json: bool, f: F) -> ExitCode
where
    F: FnOnce(chain::Chain, config::Config, (identity::Identity, PathBuf)) -> Fut,
    Fut: std::future::Future<Output = Result<serde_json::Value, String>>,
{
    node_cmd_with(cli, json, true, f)
}

fn node_cmd_with<F, Fut>(cli: &Cli, json: bool, wallet: bool, f: F) -> ExitCode
where
    F: FnOnce(chain::Chain, config::Config, (identity::Identity, PathBuf)) -> Fut,
    Fut: std::future::Future<Output = Result<serde_json::Value, String>>,
{
    let r = (|| -> Result<serde_json::Value, String> {
        let cfg = load_config(cli)?;
        let net_dir = data_root(cli).join(&cfg.network);
        std::fs::create_dir_all(&net_dir)
            .map_err(|e| format!("cannot create {}: {e}", net_dir.display()))?;
        let id = identity::Identity::load_or_create(&net_dir)?;
        runtime().block_on(async {
            let c = if wallet {
                chain::Chain::connect_wallet(&cfg, &id.service_pubkey).await?
            } else {
                chain::Chain::connect(&cfg, &id.service_pubkey).await?
            }
            .with_service_key(id.service_key);
            f(c, cfg, (id, net_dir)).await
        })
    })();
    print_result(r, json)
}

/// Prints a command result (one JSON line, or `key: value` lines); errors go to stderr.
fn print_result(r: Result<serde_json::Value, String>, json: bool) -> ExitCode {
    match r {
        Ok(j) => {
            if json {
                println!("{j}");
            } else if let Some(o) = j.as_object() {
                for (k, v) in o {
                    match v {
                        serde_json::Value::String(s) => println!("{k}: {s}"),
                        v => println!("{k}: {v}"),
                    }
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
}

fn main() -> ExitCode {
    // Every file the runner creates (data, database, logs, keys) is private to its user, whatever the
    // caller's umask; this runs before anything is written.
    unsafe {
        libc::umask(0o077);
    }
    let cli = Cli::parse();
    match &cli.command {
        Command::Version => {
            println!("csr {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Command::Config {
            action: ConfigAction::Validate,
        } => match load_config(&cli) {
            Ok(_) => {
                println!("config ok");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::from(1)
            }
        },
        Command::Check { json } => {
            let cfg = match load_config(&cli) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("{e}");
                    return ExitCode::from(1);
                }
            };
            let res = runtime().block_on(node::check(&cfg));
            if *json {
                println!("{}", res.json);
            } else {
                node::print_check_human(&res.json, &res.report);
            }
            if res.report.state == node::r2::R2State::Synced {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(2)
            }
        }
        Command::Detect { json, probe } => {
            let cfg = match load_config(&cli) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("{e}");
                    return ExitCode::from(1);
                }
            };
            let rt = runtime();
            let mut res = rt.block_on(node::detect_url(&cfg, &cfg.node.rpc_url));
            let mut probes = Vec::new();
            if *probe {
                for url in ["http://127.0.0.1:8114", "http://127.0.0.1:8124"] {
                    probes.push(rt.block_on(node::detect_url(&cfg, url)).json);
                }
                res.json["probes"] = serde_json::Value::Array(probes.clone());
            }
            if *json {
                println!("{}", res.json);
            } else {
                for j in std::iter::once(&res.json).chain(probes.iter()) {
                    let s = |k: &str| match &j[k] {
                        serde_json::Value::Null => "-".to_string(),
                        serde_json::Value::String(s) => s.clone(),
                        v => v.to_string(),
                    };
                    println!(
                        "{}: reachable={} chain={} version={} type={} indexer={}",
                        s("rpc_url"),
                        s("reachable"),
                        s("chain"),
                        s("node_version"),
                        s("node_type"),
                        s("indexer_enabled")
                    );
                    if !j["hint"].is_null() {
                        println!("  hint: {}", s("hint"));
                    }
                }
            }
            if res.ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(2)
            }
        }
        Command::Run { confirm_mainnet } => {
            let cfg = match load_config(&cli) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("{e}");
                    return ExitCode::from(1);
                }
            };
            let path = cli
                .config
                .clone()
                .unwrap_or_else(|| data_root(&cli).join("csr.toml"));
            match runtime().block_on(run::run(cfg, path, data_root(&cli), *confirm_mainnet)) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::from(1)
                }
            }
        }
        Command::Identity { json } => {
            let r = (|| -> Result<serde_json::Value, String> {
                let cfg = load_config(&cli)?;
                let net_dir = data_root(&cli).join(&cfg.network);
                std::fs::create_dir_all(&net_dir)
                    .map_err(|e| format!("cannot create {}: {e}", net_dir.display()))?;
                let id = identity::Identity::load_or_create(&net_dir)?;
                let (args, hash) = id.runner_lock()?;
                let raw = hex::decode(args.trim_start_matches("0x")).map_err(|e| e.to_string())?;
                Ok(serde_json::json!({
                    "network": cfg.network,
                    "service_pubkey": id.service_pubkey,
                    "runner_lock_args": args,
                    "runner_lock_hash": hash,
                    "runner_address": address::sighash_address(&cfg.network, &raw),
                }))
            })();
            match r {
                Ok(j) => {
                    if *json {
                        println!("{j}");
                    } else {
                        for k in [
                            "network",
                            "service_pubkey",
                            "runner_lock_args",
                            "runner_lock_hash",
                            "runner_address",
                        ] {
                            println!("{k}: {}", j[k].as_str().unwrap_or(""));
                        }
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::from(1)
                }
            }
        }
        Command::Update { action } => match action {
            UpdateAction::Check { json } => {
                let cache = update::Cache::default();
                print_result(runtime().block_on(cache.check()), *json)
            }
            UpdateAction::Stage { json } => {
                let r = load_config(&cli).and_then(|cfg| {
                    let dir = data_root(&cli).join(&cfg.network).join("update");
                    runtime().block_on(update::Cache::default().stage(&dir))
                });
                print_result(r, *json)
            }
            UpdateAction::Verify { dir, json } => print_result(update::verify_dir(dir), *json),
        },
        Command::RunnerCell {
            action: RunnerCellAction::Status { json },
        } => chain_cmd(&cli, *json, |c| async move { c.runner_cell_status().await }),
        Command::Wallet { action } => match action {
            WalletAction::Status { json } => wallet_cmd(&cli, *json, |c, cfg, (_, dir)| async move {
                wallet::status(&c, &cfg, &dir).await
            }),
            WalletAction::Send { to, ckb, json } => wallet_cmd(&cli, *json, |c, _, _| async move {
                wallet::send(&c, to, ckb).await
            }),
            WalletAction::Sweep { json } => wallet_cmd(&cli, *json, |c, cfg, (_, dir)| async move {
                wallet::sweep(&c, &cfg, &dir).await
            }),
        },
        Command::Register { action } => match action {
            RegisterAction::Status { json } => {
                chain_cmd(&cli, *json, |c| async move { c.register_status().await })
            }
            RegisterAction::VerifyConsent {
                message,
                signature,
                json,
            } => {
                let r = (|| {
                    let cfg = load_config(&cli)?;
                    let sig = consent::parse_signature_hex(signature)?;
                    let args = consent::recover_signer(message, &sig)?;
                    Ok(serde_json::json!({
                        "lock_args": format!("0x{}", hex::encode(args)),
                        "address": address::sighash_address(&cfg.network, &args),
                    }))
                })();
                print_result(r, *json)
            }
            RegisterAction::ConsentMessage { stake, json } => {
                if let Err(e) = chain::parse_stake(stake) {
                    eprintln!("{e}");
                    return ExitCode::from(1);
                }
                chain_cmd(
                    &cli,
                    *json,
                    |c| async move { c.consent_message(stake).await },
                )
            }
            RegisterAction::Create {
                stake,
                consent,
                attestation,
                json,
            } => {
                if let Err(e) = chain::parse_stake(stake) {
                    eprintln!("{e}");
                    return ExitCode::from(1);
                }
                node_cmd(&cli, *json, |c, cfg, (id, net_dir)| async move {
                    let att = match attestation {
                        Some(f) => chain::read_json(f)?,
                        None => {
                            let stake = chain::parse_stake(stake)?;
                            attest::request(&cfg, &id.service_key, &id.service_pubkey, &stake)
                                .await?
                        }
                    };
                    let r = c.register_create(&att, stake, consent).await?;
                    // A new attestation is in the cell: an error of an earlier renewal no longer applies.
                    if let Ok(db) = db::Db::open(&net_dir.join("csr.db")) {
                        let _ = db.put_setting(crate::renew::RENEWAL_ERROR, "");
                    }
                    Ok(r)
                })
            }
            RegisterAction::Refresh { json } => node_cmd(&cli, *json, |c, cfg, (id, net_dir)| async move {
                let db = db::Db::open(&net_dir.join("csr.db"))?;
                renew::renew(&c, &cfg, &id.service_key, &id.service_pubkey, &db).await
            }),
            RegisterAction::Remove { json } => {
                chain_cmd(&cli, *json, |c| async move { c.register_remove().await })
            }
            RegisterAction::Attest { stake, out, json } => {
                let r = (|| {
                    let stake = chain::parse_stake(stake)?;
                    let cfg = load_config(&cli)?;
                    let net_dir = data_root(&cli).join(&cfg.network);
                    std::fs::create_dir_all(&net_dir)
                        .map_err(|e| format!("cannot create {}: {e}", net_dir.display()))?;
                    let id = identity::Identity::load_or_create(&net_dir)?;
                    runtime().block_on(attest::run(&cfg, &id, &stake, out))
                })();
                print_result(r, *json)
            }
        },
        Command::Tx {
            action: TxAction::Submit { file, json },
        } => chain_cmd(&cli, *json, |c| async move {
            c.submit(&chain::read_json(file)?).await
        }),
        Command::Service {
            action:
                ServiceAction::RunLocal {
                    manifest,
                    payload,
                    max_cycles,
                    max_memory_mb,
                    max_wall_seconds,
                    block_info,
                },
        } => {
            let cfg = match load_config(&cli) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("{e}");
                    return ExitCode::from(1);
                }
            };
            let args = service::RunLocalArgs {
                manifest,
                payload,
                max_cycles: *max_cycles,
                max_memory_mb: *max_memory_mb,
                max_wall_seconds: *max_wall_seconds,
                block_info: block_info.as_deref(),
            };
            match service::run_local(&cfg, &data_root(&cli), &args) {
                Ok(j) => {
                    println!("{j}");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::from(1)
                }
            }
        }
        Command::Jobs {
            action: JobsAction::Plan { json },
        } => {
            let r = (|| -> Result<(), String> {
                let cfg = load_config(&cli)?;
                let net_dir = data_root(&cli).join(&cfg.network);
                std::fs::create_dir_all(&net_dir)
                    .map_err(|e| format!("cannot create {}: {e}", net_dir.display()))?;
                let id = identity::Identity::load_or_create(&net_dir)?;
                let db = db::Db::open(&net_dir.join("csr.db"))?;
                let plan = runtime().block_on(async {
                    let c = chain::Chain::connect(&cfg, &id.service_pubkey).await?;
                    jobs::plan_now(&c, &db, &cfg).await
                })?;
                jobs::store(&db, &plan)?;
                let j = plan.to_json();
                if *json {
                    println!("{j}");
                } else {
                    println!("{}", serde_json::to_string_pretty(&j).unwrap_or_default());
                }
                Ok(())
            })();
            match r {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::from(1)
                }
            }
        }
        Command::Service { action } => {
            let r = (|| -> Result<(), String> {
                let mut cfg = load_config(&cli)?;
                let net_dir = data_root(&cli).join(&cfg.network);
                std::fs::create_dir_all(&net_dir)
                    .map_err(|e| format!("cannot create {}: {e}", net_dir.display()))?;
                let db = db::Db::open(&net_dir.join("csr.db"))?;
                let root = service::storage_root(&cfg, &data_root(&cli));
                let cfg_path = cli
                    .config
                    .clone()
                    .unwrap_or_else(|| data_root(&cli).join("csr.toml"));
                let (hash, act) = match action {
                    ServiceAction::AddLocal { manifest } => {
                        println!("{}", registry::add_local(&db, manifest)?);
                        return Ok(());
                    }
                    ServiceAction::AddOnchain { outpoint } => {
                        let v = runtime().block_on(registry::add_onchain(
                            &db,
                            &cfg.node.rpc_url,
                            outpoint,
                        ))?;
                        println!("{v}");
                        return Ok(());
                    }
                    ServiceAction::List { json } => {
                        let list = registry::list(&db, &root)?;
                        if *json {
                            println!("{}", serde_json::Value::Array(list));
                        } else {
                            for s in &list {
                                println!(
                                    "{} {} {} {} storage={}B",
                                    s["code_hash"].as_str().unwrap_or(""),
                                    s["name"].as_str().unwrap_or(""),
                                    s["language"].as_str().unwrap_or(""),
                                    s["status"].as_str().unwrap_or(""),
                                    s["storage_bytes"]
                                );
                            }
                        }
                        return Ok(());
                    }
                    ServiceAction::Approve { code_hash } => (code_hash, registry::Action::Approve),
                    ServiceAction::Revoke { code_hash } => (code_hash, registry::Action::Revoke),
                    ServiceAction::Enable { code_hash } => (code_hash, registry::Action::Enable),
                    ServiceAction::Disable { code_hash } => (code_hash, registry::Action::Disable),
                    ServiceAction::RunLocal { .. } => unreachable!("handled above"),
                };
                let e = registry::act(&db, &mut cfg, &cfg_path, &root, hash, act)
                    .map_err(|e| e.message())?;
                println!("{e}");
                Ok(())
            })();
            match r {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::from(1)
                }
            }
        }
        Command::Storage {
            action: StorageAction::SimulateConsumed { outpoint },
        } => {
            let cfg = match load_config(&cli) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("{e}");
                    return ExitCode::from(1);
                }
            };
            match service::simulate_consumed(&cfg, &data_root(&cli), outpoint) {
                Ok(n) => {
                    println!("{}", serde_json::json!({ "deleted": n }));
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::from(1)
                }
            }
        }
        Command::SetPassword { password_stdin } => {
            let path = cli
                .config
                .clone()
                .unwrap_or_else(|| data_root(&cli).join("csr.toml"));
            match password::set_password(&path, *password_stdin) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::from(1)
                }
            }
        }
    }
}
