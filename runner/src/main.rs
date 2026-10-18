mod app;
mod config;
mod db;
mod identity;
mod logs;
mod network;
mod node;
mod password;
mod ping;
mod resources;
mod run;
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
    /// Run the service
    Run {
        #[arg(long)]
        confirm_mainnet: bool,
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
