mod config;
mod network;

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
        _ => {
            eprintln!("not implemented");
            ExitCode::from(1)
        }
    }
}
