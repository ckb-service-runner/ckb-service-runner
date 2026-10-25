//! Outbound check-in: CSR posts to the monitor's `/v1/checkin`, answers the returned challenge.

use super::inbound::answer_challenge;
use super::protocol::{self as proto, Challenge};
use crate::app::AppState;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tokio::time::{interval, MissedTickBehavior};

const CHECKIN_INTERVAL: Duration = Duration::from_secs(600);
const FAST_INTERVAL: Duration = Duration::from_secs(2);

/// Loops until `stop` fires; the first check-in happens right away (after the first R2 check).
pub async fn run(state: Arc<AppState>, mut stop: watch::Receiver<bool>) {
    let http = match reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
    {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("outbound check-in disabled: {e}");
            return;
        }
    };
    let p = if crate::run::fast_tick() {
        FAST_INTERVAL
    } else {
        CHECKIN_INTERVAL
    };
    let mut tick = interval(p);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = tick.tick() => {},
            _ = stop.changed() => return,
        }
        let enabled = {
            let c = state.config.read().unwrap();
            c.ranking.join && matches!(c.ranking.ping_mode.as_str(), "outbound" | "both")
        };
        if !enabled {
            continue;
        }
        if let Err(e) = checkin_once(&state, &http).await {
            tracing::warn!("outbound check-in failed: {e}");
        }
    }
}

fn monitor_base(state: &AppState) -> String {
    let c = state.config.read().unwrap();
    let url = if c.monitor.url.is_empty() {
        crate::network::NetworkDefaults::for_network(&c.network).monitor_url
    } else {
        c.monitor.url.clone()
    };
    url.trim_end_matches('/').to_string()
}

async fn checkin_once(state: &AppState, http: &reqwest::Client) -> Result<(), String> {
    let base = monitor_base(state);
    let network = state.config.read().unwrap().network.clone();
    let Some(peer_id) = state.peer_id.read().unwrap().clone() else {
        return Err("node peer id unknown".into());
    };
    let binding_sig = proto::sign(
        &state.service_key,
        &proto::binding_message(&peer_id, &state.service_pubkey, &network),
    );
    let body = json!({
        "peer_id": peer_id,
        "service_pubkey": state.service_pubkey,
        "binding_sig": binding_sig,
        "network": network,
        "csr_version": env!("CARGO_PKG_VERSION"),
    });
    let resp = http
        .post(format!("{base}/v1/checkin"))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("check-in request: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("monitor replied {} to check-in", resp.status()));
    }
    let v: Value = resp
        .json()
        .await
        .map_err(|e| format!("bad check-in reply: {e}"))?;
    let challenge: Challenge =
        serde_json::from_value(v.get("challenge").cloned().unwrap_or_default())
            .map_err(|_| "reply has no valid challenge".to_string())?;
    let now = crate::run::now();
    let (answer, sig) = answer_challenge(state, &challenge, now)
        .await
        .map_err(|e| format!("challenge rejected: {}", e.1))?;
    let resp = http
        .post(format!("{base}/v1/answer"))
        .json(&json!({"answer": answer, "sig": sig}))
        .send()
        .await
        .map_err(|e| {
            state.release_ping_nonce(&challenge.nonce);
            format!("answer request: {e}")
        })?;
    if resp.status().as_u16() == 200 {
        state
            .last_check_ok_at
            .store(crate::run::now(), Ordering::Relaxed);
        tracing::info!("monitor accepted the ping answer");
        Ok(())
    } else {
        Err(format!("monitor replied {} to answer", resp.status()))
    }
}

const IDENTITY_TICK: Duration = Duration::from_secs(10);
const IDENTITY_INTERVAL: Duration = Duration::from_secs(24 * 3600);
/// After a failed post (network, 5xx), try again this much later instead of in 24 h.
const IDENTITY_RETRY: Duration = Duration::from_secs(600);

/// What one `POST /v1/identity` came back with.
#[derive(Debug, Clone, PartialEq)]
enum ProofOutcome {
    Accepted,
    /// 404: the monitor does not know `/v1/identity` yet.
    NotSupported,
    /// 4xx with the monitor's error code (`bad_request`, `stale`, `bad_signature`, `peer_mismatch`).
    Rejected(u16, String),
    /// Network error or 5xx: retried sooner.
    Failed(String),
}

/// Node identity (D6 A): follows the node ID, and while the P2P key is proven and `ranking.join` is
/// on, posts the identity proof to the monitor at start, every 24 h and after a node restart.
pub async fn identity_loop(state: Arc<AppState>, mut stop: watch::Receiver<bool>) {
    let http = match crate::monitor::http_client(Duration::from_secs(15)) {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("node identity proof disabled: {e}");
            return;
        }
    };
    let p = if crate::run::fast_tick() {
        Duration::from_secs(1)
    } else {
        IDENTITY_TICK
    };
    let mut tick = interval(p);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    // When the next proof is due (`None` = now) and whether the node answered at the last tick.
    let mut next: Option<std::time::Instant> = None;
    let mut node_seen = true;
    loop {
        tokio::select! {
            _ = tick.tick() => {},
            _ = stop.changed() => return,
        }
        let node_id = state.node.read().unwrap().snap.node_id.clone();
        let status = state.p2p_key.status(node_id.as_deref());
        {
            let mut cur = state.identity_status.write().unwrap();
            if *cur != status && status.state != "unknown" {
                match status.state {
                    "proven" => tracing::info!("node identity: proven ({})", node_id.as_deref().unwrap_or("")),
                    s => tracing::error!("node identity: {s}: {}", status.message.as_deref().unwrap_or("")),
                }
            }
            // While the node is unreachable the last known state stays visible.
            if status.state != "unknown" {
                *cur = status;
            }
        }
        // The node ID seen again after the node was unreachable: the node restarted.
        if node_id.is_some() && !node_seen {
            next = None;
        }
        node_seen = node_id.is_some();
        let (join, network) = {
            let c = state.config.read().unwrap();
            (c.ranking.join, c.network.clone())
        };
        let Some(key) = state.p2p_key.proven(node_id.as_deref()) else {
            continue;
        };
        if !join || next.is_some_and(|t| std::time::Instant::now() < t) {
            continue;
        }
        let proof = crate::identity::identity_proof(
            key,
            &state.service_key,
            &state.service_pubkey,
            &network,
            crate::run::now(),
        );
        let base = monitor_base(&state);
        let outcome = post_identity(&http, &base, &proof).await;
        let wait = match &outcome {
            ProofOutcome::Accepted => {
                tracing::info!("monitor {base} accepted the node identity proof");
                IDENTITY_INTERVAL
            }
            ProofOutcome::NotSupported => {
                tracing::debug!("monitor {base} has no /v1/identity yet");
                IDENTITY_INTERVAL
            }
            ProofOutcome::Rejected(st, code) => {
                tracing::warn!("monitor {base} rejected the node identity proof: {st} {code}");
                IDENTITY_INTERVAL
            }
            ProofOutcome::Failed(e) => {
                tracing::warn!("node identity proof to {base} failed: {e}");
                IDENTITY_RETRY
            }
        };
        next = Some(std::time::Instant::now() + if crate::run::fast_tick() { p * 5 } else { wait });
        let mut entry = match outcome {
            ProofOutcome::Accepted => json!({"result": "accepted"}),
            ProofOutcome::NotSupported => json!({"result": "not_supported"}),
            ProofOutcome::Rejected(st, code) => json!({"result": "rejected", "status": st, "error": code}),
            ProofOutcome::Failed(e) => json!({"result": "failed", "error": e}),
        };
        entry["at"] = json!(crate::run::now());
        state.identity_proofs.lock().unwrap().insert(base, entry);
    }
}

async fn post_identity(http: &reqwest::Client, base: &str, proof: &Value) -> ProofOutcome {
    let resp = match http.post(format!("{base}/v1/identity")).json(proof).send().await {
        Ok(r) => r,
        Err(e) => return ProofOutcome::Failed(format!("request: {}", e.without_url())),
    };
    let st = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap_or_default();
    let code = body
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or("")
        .chars()
        .take(64)
        .collect::<String>();
    match st {
        200 if body.get("ok") == Some(&json!(true)) => ProofOutcome::Accepted,
        200 => ProofOutcome::Failed("monitor replied 200 without \"ok\": true".into()),
        404 => ProofOutcome::NotSupported,
        400..=499 => ProofOutcome::Rejected(st, code),
        _ => ProofOutcome::Failed(format!("monitor replied {st}")),
    }
}
