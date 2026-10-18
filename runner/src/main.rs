mod app;
mod config;
mod db;
mod identity;
mod logs;
mod monitor;
mod network;
mod node;
mod password;
mod ping;
mod registry;
mod resources;
mod run;
mod runtime;
mod service;
mod storage;
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
    disable_version_flag = true
)]
struct Cli {
    /// Data root directory (default: $HOME/.csr)
    #[arg(long, global = true)]
    data_root: Option<PathBuf>,
    /// Config file (default: <data-root>/csr.toml)
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    /// Network override: testnet or mainnet
    #[arg(long, global = true, value_parser = ["testnet", "mainnet"])]
    network: Option<String>,
    #[command(subcommand)]
    command: Command,
}

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
    /// Run the service
    Run {
        #[arg(long)]
        confirm_mainnet: bool,
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

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
}

fn main() -> ExitCode {
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
