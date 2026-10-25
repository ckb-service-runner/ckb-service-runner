//! Operator flows of the dashboard: the runner wallet and the registration. CSR signs every transaction
//! itself with the service key; the same JSON as the CLI, errors → 400 `{"error"}`.

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

pub async fn runner_cell(State(s): St) -> Response {
    match chain(&s).await {
        Ok(c) => reply(c.runner_cell_status().await),
        Err(e) => fail(StatusCode::BAD_GATEWAY, e),
    }
}

pub async fn wallet_status(State(s): St) -> Response {
    let cfg = s.config.read().unwrap().clone();
    match chain(&s).await {
        Ok(c) => reply(crate::wallet::status(&c, &cfg, &s.net_dir).await),
        Err(e) => fail(StatusCode::BAD_GATEWAY, e),
    }
}

/// Body: `to` (a full-format address of this network) and `ckb`.
pub async fn wallet_send(State(s): St, Json(b): Json<Value>) -> Response {
    let (Some(to), Some(ckb)) = (b["to"].as_str(), amount(&b)) else {
        return fail(StatusCode::BAD_REQUEST, "to and ckb are required".into());
    };
    match chain(&s).await {
        Ok(c) => {
            // One runner transaction at a time (the keeper and the sweep spend the same wallet).
            let _tx = s.wallet_tx.lock().await;
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
    let r = c.register_create(&att, &stake, consent).await;
    if let Ok(v) = &r {
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
    if crate::wallet::wallet_busy(&s.db) {
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
