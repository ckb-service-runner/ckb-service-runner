//! Operator signing flow: prepare unsigned payout transactions, submit signed ones.

use crate::app::AppState;
use crate::chain::Chain;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};
use std::sync::Arc;

type St = State<Arc<AppState>>;

fn fail(code: StatusCode, msg: String) -> Response {
    (code, Json(json!({"error": msg}))).into_response()
}

async fn chain(s: &AppState) -> Result<Chain, String> {
    let cfg = s.config.read().unwrap().clone();
    Chain::connect(&cfg, &s.service_pubkey)
        .await
        .map(|c| c.with_service_key(s.service_key))
}

fn reply(r: Result<Value, String>) -> Response {
    match r {
        Ok(v) => Json(v).into_response(),
        Err(e) => fail(StatusCode::BAD_REQUEST, e),
    }
}

/// A CKB amount given as a JSON number or string.
fn amount(b: &Value) -> Option<String> {
    match &b["ckb"] {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn prepared(p: crate::chain::Prepared) -> Value {
    json!({"file": p.file, "fee_shannons": p.fee_shannons, "tx_hash": p.tx_hash})
}

pub async fn runner_cell(State(s): St) -> Response {
    match chain(&s).await {
        Ok(c) => reply(c.runner_cell_status().await),
        Err(e) => fail(StatusCode::BAD_GATEWAY, e),
    }
}

pub async fn payout_status(State(s): St) -> Response {
    match chain(&s).await {
        Ok(c) => reply(c.payout_status().await),
        Err(e) => fail(StatusCode::BAD_GATEWAY, e),
    }
}

/// Body: `ckb` and `owner_lock_args` (the wallet that pays for the cell).
pub async fn payout_prepare_create(State(s): St, Json(b): Json<Value>) -> Response {
    let (Some(ckb), Some(owner)) = (amount(&b), b["owner_lock_args"].as_str()) else {
        return fail(
            StatusCode::BAD_REQUEST,
            "ckb and owner_lock_args are required".into(),
        );
    };
    match chain(&s).await {
        Ok(c) => reply(c.prepare_payout_create(&ckb, owner).await.map(prepared)),
        Err(e) => fail(StatusCode::BAD_GATEWAY, e),
    }
}

/// Body: `ckb` or `all: true`.
pub async fn payout_prepare_withdraw(State(s): St, Json(b): Json<Value>) -> Response {
    let all = b["all"].as_bool().unwrap_or(false);
    let ckb = amount(&b);
    if all == ckb.is_some() {
        return fail(
            StatusCode::BAD_REQUEST,
            "give either ckb or all = true".into(),
        );
    }
    match chain(&s).await {
        Ok(c) => reply(
            c.prepare_payout_withdraw(ckb.as_deref(), all)
                .await
                .map(prepared),
        ),
        Err(e) => fail(StatusCode::BAD_GATEWAY, e),
    }
}

pub async fn submit_signed(State(s): St, Json(b): Json<Value>) -> Response {
    match chain(&s).await {
        Ok(c) => {
            let r = c.submit(&b).await;
            if let Ok(v) = &r {
                tracing::info!("signed transaction submitted: {}", v["tx_hash"]);
            }
            reply(r)
        }
        Err(e) => fail(StatusCode::BAD_GATEWAY, e),
    }
}

pub async fn registry_status(State(s): St) -> Response {
    match chain(&s).await {
        Ok(c) => reply(c.register_status().await),
        Err(e) => fail(StatusCode::BAD_GATEWAY, e),
    }
}

/// Body: `attestation` (the monitor's attestation JSON) and `owner_lock_args`.
pub async fn prepare_register(State(s): St, Json(b): Json<Value>) -> Response {
    let Some(owner) = b["owner_lock_args"].as_str() else {
        return fail(
            StatusCode::BAD_REQUEST,
            "owner_lock_args is required".into(),
        );
    };
    if !b["attestation"].is_object() {
        return fail(StatusCode::BAD_REQUEST, "attestation is required".into());
    }
    match chain(&s).await {
        Ok(c) => reply(
            c.prepare_register(&b["attestation"], owner)
                .await
                .map(prepared),
        ),
        Err(e) => fail(StatusCode::BAD_GATEWAY, e),
    }
}
