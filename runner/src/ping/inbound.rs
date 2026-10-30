//! Inbound ping endpoint: `POST /v1/challenge` on `ranking.ping_bind`.

use super::protocol::{self as proto, Answer, Challenge};
use crate::app::AppState;
use crate::config::parse_pubkey;
use crate::node::rpc::RpcClient;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, DefaultBodyLimit, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::json;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const BODY_LIMIT: usize = 4096;
const RATE_LIMIT: usize = 10;
const RATE_WINDOW: Duration = Duration::from_secs(60);
const MAX_TRACKED: usize = 10_000;

pub struct Responder {
    state: Arc<AppState>,
    rate: Mutex<HashMap<IpAddr, Vec<Instant>>>,
}

type Rej = (StatusCode, &'static str);

impl Responder {
    pub fn new(state: Arc<AppState>) -> Arc<Responder> {
        Arc::new(Responder {
            state,
            rate: Mutex::new(HashMap::new()),
        })
    }

    fn rate_ok(&self, ip: IpAddr) -> bool {
        let now = Instant::now();
        let mut m = self.rate.lock().unwrap();
        if m.len() > MAX_TRACKED {
            m.retain(|_, v| v.last().is_some_and(|t| now - *t < RATE_WINDOW));
        }
        let hits = m.entry(ip).or_default();
        hits.retain(|t| now - *t < RATE_WINDOW);
        let ok = hits.len() < RATE_LIMIT;
        hits.push(now);
        ok
    }
}

/// Validates a challenge against the config and node state and builds the signed answer.
pub async fn answer_challenge(
    state: &AppState,
    c: &Challenge,
    now: i64,
) -> Result<(Answer, String), Rej> {
    let (monitors, network, mode, rpc_url) = {
        let cfg = state.config.read().unwrap();
        let usable: Vec<_> = cfg
            .monitors()
            .into_iter()
            .filter(|m| crate::network::monitor_key_usable(&m.pubkey))
            .collect();
        if usable.is_empty() {
            crate::network::monitor_key_problem(&cfg);
            return Err((StatusCode::SERVICE_UNAVAILABLE, "no monitor key set"));
        }
        (
            usable,
            cfg.network.clone(),
            cfg.ranking.ping_mode.clone(),
            cfg.node.rpc_url.clone(),
        )
    };
    if !proto::valid_nonce(&c.nonce) {
        return Err((StatusCode::BAD_REQUEST, "bad_request"));
    }
    let msg = proto::challenge_message(&c.nonce, c.height, c.issued_at);
    let signed = monitors.iter().any(|m| match parse_pubkey(&m.pubkey) {
        Ok(pk) => proto::verify(&pk, &msg, &c.monitor_sig),
        Err(e) => {
            tracing::warn!("skipping monitor {}: key not usable: {e}", m.url);
            false
        }
    });
    if !signed {
        return Err((StatusCode::FORBIDDEN, "bad_monitor_signature"));
    }
    let (tip, r2_state, node_version) = {
        let n = state.node.read().unwrap();
        (
            n.snap.tip.as_ref().map(|t| (t.number, t.hash.clone())),
            n.report.state.as_str(),
            n.snap.node_version.clone(),
        )
    };
    let status = proto::challenge_status(
        tip.as_ref().map_or(0, |t| t.0),
        c.height,
        c.issued_at,
        now,
    );
    let block_known = match status {
        proto::ChallengeStatus::ClockSkew => {
            state.note_clock_skew(now);
            return Err((StatusCode::BAD_REQUEST, "clock_skew"));
        }
        proto::ChallengeStatus::Answer { block_known } => block_known,
    };
    let Some((tip_number, tip_hash)) = tip else {
        return Err((StatusCode::SERVICE_UNAVAILABLE, "node_unavailable"));
    };
    let peer_id = state.peer_id.read().unwrap().clone();
    let (Some(peer_id), Some(node_version)) = (peer_id, node_version) else {
        return Err((StatusCode::SERVICE_UNAVAILABLE, "node_unavailable"));
    };
    if !state.reserve_ping_nonce(&c.nonce, now) {
        return Err((StatusCode::CONFLICT, "replay"));
    }
    let block_hash = if !block_known {
        String::new()
    } else {
        match RpcClient::new(&rpc_url)
            .call("get_block_hash", json!([format!("0x{:x}", c.height)]))
            .await
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
        {
            Some(h) => h,
            None => {
                // A failed attempt does not burn the nonce.
                state.release_ping_nonce(&c.nonce);
                return Err((StatusCode::SERVICE_UNAVAILABLE, "node_unavailable"));
            }
        }
    };
    let binding_sig = proto::sign(
        &state.service_key,
        &proto::binding_message(&peer_id, &state.service_pubkey, &network),
    );
    let answer = Answer {
        nonce: c.nonce.clone(),
        peer_id,
        service_pubkey: state.service_pubkey.clone(),
        binding_sig,
        network,
        csr_version: env!("CARGO_PKG_VERSION").to_string(),
        node_version,
        r2_state: r2_state.to_string(),
        tip_number,
        tip_hash,
        block_hash_at_height: block_hash,
        uptime_s: state.started.elapsed().as_secs(),
        ping_mode: mode,
    };
    let sig = proto::sign(&state.service_key, &proto::answer_message(&answer));
    Ok((answer, sig))
}

fn rej((status, code): Rej) -> Response {
    (status, Json(json!({"error": code}))).into_response()
}

async fn challenge(
    State(r): State<Arc<Responder>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    body: Bytes,
) -> Response {
    if !r.rate_ok(addr.ip()) {
        return rej((StatusCode::TOO_MANY_REQUESTS, "rate_limited"));
    }
    if body.len() > BODY_LIMIT {
        return rej((StatusCode::PAYLOAD_TOO_LARGE, "too_large"));
    }
    let Ok(c) = serde_json::from_slice::<Challenge>(&body) else {
        return rej((StatusCode::BAD_REQUEST, "bad_request"));
    };
    let now = crate::run::now();
    match answer_challenge(&r.state, &c, now).await {
        Ok((answer, sig)) => {
            r.state.last_check_ok_at.store(now, Ordering::Relaxed);
            Json(json!({"answer": answer, "sig": sig})).into_response()
        }
        Err(e) => {
            tracing::info!("ping challenge rejected: {}", e.1);
            rej(e)
        }
    }
}

pub fn router(r: Arc<Responder>) -> Router {
    Router::new()
        .route("/v1/challenge", post(challenge))
        .layer(DefaultBodyLimit::max(BODY_LIMIT * 16))
        .with_state(r)
}
