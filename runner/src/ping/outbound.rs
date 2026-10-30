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
    let http = match crate::network::http_client(Duration::from_secs(15)) {
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
            c.ranking.join
                && matches!(c.ranking.ping_mode.as_str(), "outbound" | "both")
        };
        if !enabled {
            continue;
        }
        let monitors = state.config.read().unwrap().monitors();
        for m in monitors {
            // A monitor without a usable key (empty, or the placeholder) gets no check-in.
            if !crate::network::monitor_key_usable(&m.pubkey) {
                tracing::debug!("no check-in to {}: no usable monitor key", m.url);
                continue;
            }
            if let Err(e) = checkin_once(&state, &http, &m.url).await {
                tracing::warn!("outbound check-in to {} failed: {e}", m.url);
            }
        }
    }
}

async fn checkin_once(
    state: &AppState,
    http: &reqwest::Client,
    base: &str,
) -> Result<(), String> {
    let base = base.trim_end_matches('/');
    let network = state.config.read().unwrap().network.clone();
    let Some(peer_id) = state.peer_id.read().unwrap().clone() else {
        return Err("node peer id unknown".into());
    };
    let binding_sig = proto::sign(
        &state.service_key,
        &proto::binding_message(&peer_id, &state.service_pubkey, &network),
    );
    let ts = crate::run::now();
    let checkin_sig = proto::sign(
        &state.service_key,
        &proto::checkin_message(&peer_id, &state.service_pubkey, &network, ts),
    );
    let body = json!({
        "peer_id": peer_id,
        "service_pubkey": state.service_pubkey,
        "binding_sig": binding_sig,
        "network": network,
        "csr_version": env!("CARGO_PKG_VERSION"),
        "ts": ts,
        "checkin_sig": checkin_sig,
    });
    let resp = http
        .post(format!("{base}/v1/checkin"))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("check-in request: {e}"))?;
    crate::network::check_no_redirect(&resp)?;
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
    crate::network::check_no_redirect(&resp)?;
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
