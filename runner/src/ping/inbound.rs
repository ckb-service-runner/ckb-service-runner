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
    let (monitor_pub, network, mode, rpc_url) = {
        let cfg = state.config.read().unwrap();
        if !crate::network::monitor_key_usable(&crate::network::effective_monitor_key(&cfg)) {
            crate::network::monitor_key_problem(&cfg);
            return Err((StatusCode::SERVICE_UNAVAILABLE, "no monitor key set"));
        }
        (
            crate::network::effective_monitor_key(&cfg),
            cfg.network.clone(),
            cfg.ranking.ping_mode.clone(),
            cfg.node.rpc_url.clone(),
        )
    };
    if !proto::valid_nonce(&c.nonce) {
        return Err((StatusCode::BAD_REQUEST, "invalid nonce"));
    }
    let pk = parse_pubkey(&monitor_pub)
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "bad monitor key"))?;
    let msg = proto::challenge_message(&c.nonce, c.height, c.issued_at);
    if !proto::verify(&pk, &msg, &c.monitor_sig) {
        return Err((StatusCode::FORBIDDEN, "invalid monitor signature"));
    }
    if (now - c.issued_at).abs() > proto::FRESHNESS_SECS {
        return Err((StatusCode::BAD_REQUEST, "challenge not fresh"));
    }
    let (tip_number, tip_hash, r2_state, node_version) = {
        let n = state.node.read().unwrap();
        let tip = n.snap.tip.as_ref();
        (
            tip.map(|t| t.number),
            tip.map(|t| t.hash.clone()),
            n.report.state.as_str(),
            n.snap.node_version.clone(),
        )
    };
    let (Some(tip_number), Some(tip_hash)) = (tip_number, tip_hash) else {
        return Err((StatusCode::SERVICE_UNAVAILABLE, "node tip unknown"));
    };
    if c.height > tip_number {
        return Err((StatusCode::BAD_REQUEST, "height above tip"));
    }
    let peer_id = state.peer_id.read().unwrap().clone();
    let (Some(peer_id), Some(node_version)) = (peer_id, node_version) else {
        return Err((StatusCode::SERVICE_UNAVAILABLE, "node info unknown"));
    };
    if !state.reserve_ping_nonce(&c.nonce, now) {
        return Err((StatusCode::CONFLICT, "nonce already answered"));
    }
    let block_hash = match RpcClient::new(&rpc_url)
        .call("get_block_hash", json!([format!("0x{:x}", c.height)]))
        .await
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
    {
        Some(h) => h,
        None => {
            // A failed attempt does not burn the nonce.
            state.release_ping_nonce(&c.nonce);
            return Err((StatusCode::SERVICE_UNAVAILABLE, "block hash unavailable"));
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

async fn challenge(
    State(r): State<Arc<Responder>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    body: Bytes,
) -> Response {
    if !r.rate_ok(addr.ip()) {
        return (StatusCode::TOO_MANY_REQUESTS, "rate limit exceeded").into_response();
    }
    let Ok(c) = serde_json::from_slice::<Challenge>(&body) else {
        return (StatusCode::BAD_REQUEST, "malformed challenge").into_response();
    };
    let now = crate::run::now();
    match answer_challenge(&r.state, &c, now).await {
        Ok((answer, sig)) => {
            r.state.last_check_ok_at.store(now, Ordering::Relaxed);
            Json(json!({"answer": answer, "sig": sig})).into_response()
        }
        Err(e) => {
            tracing::info!("ping challenge rejected: {}", e.1);
            e.into_response()
        }
    }
}

pub fn router(r: Arc<Responder>) -> Router {
    Router::new()
        .route("/v1/challenge", post(challenge))
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .with_state(r)
}
