//! Operator flows of the dashboard: the runner wallet, the registration and the update. CSR signs
//! every transaction itself with the service key; the same JSON as the CLI, errors → 400 `{"error"}`.

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

/// The wallet connection: needs no CSR deployment.
async fn wallet_chain(s: &AppState) -> Result<Chain, String> {
    let cfg = s.config.read().unwrap().clone();
    Chain::connect_wallet(&cfg, &s.service_pubkey)
        .await
        .map(|c| c.with_service_key(s.service_key))
}

/// 409 while a keeper or sweep transaction of the runner is pending (they spend the same wallet).
/// Call after taking `wallet_tx`.
fn busy(s: &AppState) -> Option<Response> {
    let submitting = s
        .db
        .runs_in(&[crate::keeper::SUBMITTING])
        .map(|r| r.len())
        .unwrap_or(0);
    let sweep = s
        .db
        .get_setting("sweep_pending")
        .ok()
        .flatten()
        .is_some_and(|v| !v.is_empty());
    let reason = if crate::wallet::refresh_pending(&s.db) {
        Some("an attestation refresh is pending".to_string())
    } else {
        crate::wallet::wallet_busy(submitting, sweep)
    };
    reason.map(|detail| {
        (
            StatusCode::CONFLICT,
            Json(json!({"error": "busy", "detail": detail})),
        )
            .into_response()
    })
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

pub async fn runner_cell(State(s): St) -> Response {
    match wallet_chain(&s).await {
        Ok(c) => reply(c.runner_cell_status().await),
        Err(e) => fail(StatusCode::BAD_GATEWAY, e),
    }
}

pub async fn wallet_status(State(s): St) -> Response {
    let cfg = s.config.read().unwrap().clone();
    match wallet_chain(&s).await {
        Ok(c) => reply(crate::wallet::status(&c, &cfg, &s.net_dir).await),
        Err(e) => fail(StatusCode::BAD_GATEWAY, e),
    }
}

/// Body: `to` (a full-format address of this network) and `ckb`.
pub async fn wallet_send(State(s): St, Json(b): Json<Value>) -> Response {
    let (Some(to), Some(ckb)) = (b["to"].as_str(), amount(&b)) else {
        return fail(StatusCode::BAD_REQUEST, "to and ckb are required".into());
    };
    match wallet_chain(&s).await {
        Ok(c) => {
            // One runner transaction at a time (the keeper and the sweep spend the same wallet).
            let _tx = s.wallet_tx.lock().await;
            if let Some(r) = busy(&s) {
                return r;
            }
            reply(crate::wallet::send(&c, to, &ckb).await)
        }
        Err(e) => fail(StatusCode::BAD_GATEWAY, e),
    }
}

pub async fn registry_status(State(s): St) -> Response {
    match chain(&s).await {
        Ok(c) => reply(c.register_status().await.map(|mut v| {
            let e = s.db.get_setting(crate::renew::RENEWAL_ERROR).ok().flatten();
            v["renewal_error"] = e.filter(|e| !e.is_empty()).map_or(Value::Null, Value::from);
            v
        })),
        Err(e) => fail(StatusCode::BAD_GATEWAY, e),
    }
}

fn stake_of(b: &Value) -> Result<String, Response> {
    let stake = b["stake"].as_str().ok_or_else(|| {
        fail(
            StatusCode::BAD_REQUEST,
            "stake is required (0x<tx hash>:<index> of a Nervos DAO deposit)".into(),
        )
    })?;
    crate::chain::parse_stake(stake).map_err(|e| fail(StatusCode::BAD_REQUEST, e))
}

/// Body: `stake`.
pub async fn consent_message(State(s): St, Json(b): Json<Value>) -> Response {
    let stake = match stake_of(&b) {
        Ok(s) => s,
        Err(r) => return r,
    };
    match chain(&s).await {
        Ok(c) => reply(c.consent_message(&stake).await),
        Err(e) => fail(StatusCode::BAD_GATEWAY, e),
    }
}

/// Body: `stake`, `consent` (the stake owner's signature) and optionally `attestation` (the monitor's
/// attestation object; without it the monitor is asked).
pub async fn register_create(State(s): St, Json(b): Json<Value>) -> Response {
    let stake = match stake_of(&b) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let Some(consent) = b["consent"].as_str() else {
        return fail(
            StatusCode::BAD_REQUEST,
            "consent is required (the stake owner's signature of the consent message)".into(),
        );
    };
    let att = match &b["attestation"] {
        Value::Null => None,
        v if v.is_object() => Some(v.clone()),
        _ => {
            return fail(
                StatusCode::BAD_REQUEST,
                "attestation must be the attestation JSON object".into(),
            )
        }
    };
    let c = match chain(&s).await {
        Ok(c) => c,
        Err(e) => return fail(StatusCode::BAD_GATEWAY, e),
    };
    let att = match att {
        Some(a) => a,
        None => {
            let cfg = s.config.read().unwrap().clone();
            match crate::attest::request(&cfg, &s.service_key, &s.service_pubkey, &stake).await {
                Ok(a) => a,
                Err(e) => return fail(StatusCode::BAD_REQUEST, e),
            }
        }
    };
    let _tx = s.wallet_tx.lock().await;
    if let Some(r) = busy(&s) {
        return r;
    }
    let r = c.register_create(&att, &stake, consent).await;
    if let Ok(v) = &r {
        // A new attestation is in the cell: an error of an earlier renewal no longer applies.
        let _ = s.db.put_setting(crate::renew::RENEWAL_ERROR, "");
        tracing::info!(
            "registration {}: {} (stake {stake})",
            if v["updated"] == true {
                "updated"
            } else {
                "created"
            },
            v["tx_hash"]
        );
    }
    reply(r)
}

pub async fn register_remove(State(s): St) -> Response {
    match chain(&s).await {
        Ok(c) => {
            let _tx = s.wallet_tx.lock().await;
            if let Some(r) = busy(&s) {
                return r;
            }
            let r = c.register_remove().await;
            if let Ok(v) = &r {
                tracing::info!("registration removed: {}", v["tx_hash"]);
            }
            reply(r)
        }
        Err(e) => fail(StatusCode::BAD_GATEWAY, e),
    }
}

/// Asks the monitor for a new attestation and refreshes the registry cell (no body).
pub async fn register_refresh(State(s): St) -> Response {
    let c = match chain(&s).await {
        Ok(c) => c,
        Err(e) => return fail(StatusCode::BAD_GATEWAY, e),
    };
    let _tx = s.wallet_tx.lock().await;
    if crate::wallet::busy_reason(&s.db).is_some() {
        return fail(
            StatusCode::BAD_REQUEST,
            "a runner transaction is still pending; try again when it is committed".into(),
        );
    }
    let cfg = s.config.read().unwrap().clone();
    let r = crate::renew::renew(&c, &cfg, &s.service_key, &s.service_pubkey, &s.db).await;
    match &r {
        Ok(v) => {
            let _ = s.db.put_setting(crate::renew::RENEWAL_ERROR, "");
            tracing::info!("attestation renewed: {}", v["tx_hash"]);
        }
        Err(e) => {
            let _ = s.db.put_setting(crate::renew::RENEWAL_ERROR, e);
        }
    }
    reply(r)
}

/// The latest GitHub release compared with this version (cached for an hour).
pub async fn update_check(State(s): St) -> Response {
    // "Check for update" in the dashboard: ask GitHub again unless the answer is under a minute old
    // (the daily background check keeps the one-hour cache).
    match s.update.check_fresh().await {
        Ok(v) => Json(v).into_response(),
        Err(e) => fail(StatusCode::BAD_GATEWAY, e),
    }
}

/// Downloads, verifies and stages the latest release; the root installer takes it from there.
pub async fn update_install(State(s): St) -> Response {
    reply(s.update.stage(&s.net_dir.join("update")).await)
}
