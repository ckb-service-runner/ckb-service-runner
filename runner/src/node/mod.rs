//! Talking to the CKB node: JSON-RPC client and the R2 sync guard.

// Parts are consumed by the run loop and web API in later phases.
#![allow(dead_code)]

pub mod r2;
pub mod rpc;

use crate::config::Config;
use r2::{evaluate, expected_chain, fetch_snapshot, R2Report, Snapshot};
use rpc::RpcClient;
use serde_json::{json, Value};

/// `node_type` to report for a snapshot (`None` while unreachable).
pub fn node_type(cfg: &Config, s: &Snapshot) -> Option<&'static str> {
    if s.transport_error || s.tip.is_none() {
        None
    } else if s.chain_info.is_none() || s.sync.is_none() {
        Some("unsupported")
    } else if cfg.node.node_type == "neuron" {
        Some("neuron")
    } else {
        Some("ckb")
    }
}

pub struct CheckResult {
    pub json: Value,
    pub report: R2Report,
}

pub async fn check(cfg: &Config) -> CheckResult {
    let client = RpcClient::new(&cfg.node.rpc_url);
    let snap = fetch_snapshot(&client).await;
    let report = evaluate(&snap, expected_chain(&cfg.network));
    let reachable = !snap.transport_error && snap.tip.is_some();
    let json = json!({
        "network": cfg.network,
        "config_ok": true,
        "node": {
            "reachable": reachable,
            "chain": snap.chain_info.as_ref().map(|c| c.0.clone()),
            "node_version": snap.node_version,
            "node_type": node_type(cfg, &snap),
        },
        "r2": report,
    });
    CheckResult { json, report }
}

pub fn print_check_human(j: &Value, report: &R2Report) {
    let n = &j["node"];
    let show = |v: &Value| match v {
        Value::Null => "-".to_string(),
        Value::String(s) => s.clone(),
        o => o.to_string(),
    };
    println!("network:  {}", j["network"].as_str().unwrap_or("-"));
    println!(
        "node:     {}",
        if n["reachable"] == true {
            "reachable"
        } else {
            "unreachable"
        }
    );
    println!("chain:    {}", show(&n["chain"]));
    println!("version:  {}", show(&n["node_version"]));
    println!("R2 state: {}", report.state.as_str());
    for r in &report.reasons {
        println!("  - {r}");
    }
    println!(
        "tip {} / best known {} (lag {}), indexer lag {}, outbound peers {}",
        show(&j["r2"]["tip"]),
        show(&j["r2"]["best_known"]),
        show(&j["r2"]["tip_lag"]),
        show(&j["r2"]["indexer_lag"]),
        show(&j["r2"]["outbound_peers"])
    );
}

pub struct DetectResult {
    pub json: Value,
    pub ok: bool,
}

const INDEXER_HINT: &str =
    "Indexer module is disabled: add \"Indexer\" to `modules` under [rpc] in ckb.toml and restart the node";

pub async fn detect_url(cfg: &Config, url: &str) -> DetectResult {
    let client = RpcClient::new(url);
    let snap = fetch_snapshot(&client).await;
    let reachable = !snap.transport_error && snap.tip.is_some();
    let nt = node_type(cfg, &snap);
    let indexer_enabled = if reachable {
        Some(snap.indexer_tip.is_some() && !snap.indexer_missing)
    } else {
        None
    };
    let hint = if !reachable {
        Some("node not reachable: check node.rpc_url and that the CKB node is running".to_string())
    } else if nt == Some("unsupported") {
        Some("unsupported node: sync_state/get_blockchain_info missing (light client?); a full node is required".to_string())
    } else if snap.indexer_missing {
        Some(INDEXER_HINT.to_string())
    } else {
        None
    };
    DetectResult {
        ok: reachable && nt != Some("unsupported"),
        json: json!({
            "rpc_url": url,
            "reachable": reachable,
            "chain": snap.chain_info.as_ref().map(|c| c.0.clone()),
            "node_version": snap.node_version,
            "node_type": nt,
            "indexer_enabled": indexer_enabled,
            "hint": hint,
        }),
    }
}

/// Everything one guard tick learns about the node.
#[derive(Debug, Clone)]
pub struct Polled {
    pub snap: Snapshot,
    pub report: R2Report,
    /// `(pending, proposed)` from `tx_pool_info`.
    pub tx_pool: Option<(u64, u64)>,
}

impl Polled {
    pub fn unknown() -> Polled {
        Polled {
            snap: Snapshot::default(),
            report: R2Report::unknown(),
            tx_pool: None,
        }
    }
}

/// One guard tick: snapshot, `tx_pool_info` and the R2 evaluation (7 JSON-RPC calls).
pub async fn poll(client: &RpcClient, network: &str) -> Polled {
    let snap = fetch_snapshot(client).await;
    let report = evaluate(&snap, expected_chain(network));
    let tx_pool = if snap.transport_error || snap.tip.is_none() {
        None
    } else {
        client
            .call("tx_pool_info", json!([]))
            .await
            .ok()
            .and_then(|v| {
                Some((
                    rpc::hex_u64(v.get("pending")?)?,
                    rpc::hex_u64(v.get("proposed")?)?,
                ))
            })
    };
    Polled {
        snap,
        report,
        tx_pool,
    }
}

/// Checks done on the first successful contact with the node. `Ok(true)` means
/// the node was contacted and is acceptable, `Ok(false)` that it is unreachable
/// (try again later), `Err` that `csr run` must refuse to work with this node.
pub fn first_contact(snap: &Snapshot, network: &str) -> Result<bool, String> {
    if snap.transport_error || snap.tip.is_none() {
        return Ok(false);
    }
    let (Some((chain, _)), Some(_)) = (&snap.chain_info, snap.sync) else {
        return Err(
            "unsupported node: sync_state or get_blockchain_info missing (light client?); a full node is required"
                .into(),
        );
    };
    let expected = expected_chain(network);
    if chain != expected {
        return Err(format!(
            "chain mismatch: expected {expected}, node reports {chain}"
        ));
    }
    Ok(true)
}
