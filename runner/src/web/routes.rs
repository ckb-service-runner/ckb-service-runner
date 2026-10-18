//! Read API routes.

use crate::app::AppState;
use crate::logs;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

type St = State<Arc<AppState>>;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/api/status", get(status))
        .route("/api/node", get(node))
        .route("/api/network", get(network))
        .route("/api/resources", get(resources))
        .route("/api/resources/history", get(history))
        .route("/api/logs", get(logs_route))
        .route("/api/session", get(super::auth::session))
        .route("/api/login", post(super::auth::login))
        .route(
            "/api/settings",
            get(super::settings::get).put(super::settings::put),
        )
        .route("/api/services", get(services))
        .route("/api/services/{hash}/{verb}", post(service_action))
        .route("/api/runner-cell", get(super::operator::runner_cell))
        .route("/api/float", get(super::operator::float_status))
        .route(
            "/api/float/prepare-create",
            post(super::operator::prepare_create),
        )
        .route(
            "/api/float/prepare-topup",
            post(super::operator::prepare_topup),
        )
        .route(
            "/api/float/prepare-withdraw",
            post(super::operator::prepare_withdraw),
        )
        .route("/api/registry", get(super::operator::registry_status))
        .route(
            "/api/registry/prepare-create",
            post(super::operator::prepare_register),
        )
        .route(
            "/api/tx/submit-signed",
            post(super::operator::submit_signed),
        )
        .route("/api/events", get(super::settings::events))
}

async fn status(State(s): St) -> Json<Value> {
    Json(s.status_json())
}

async fn node(State(s): St) -> Json<Value> {
    let nt = s.config.read().unwrap().node.node_type.clone();
    Json(s.node_json(&nt))
}

async fn network(State(s): St) -> Json<Value> {
    Json(crate::monitor::network_json(&s).await)
}

async fn resources(State(s): St) -> Response {
    match &*s.resources.read().unwrap() {
        Some(r) => Json(r).into_response(),
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": "resources not sampled yet"})),
        )
            .into_response(),
    }
}

async fn history(State(s): St) -> Response {
    let since = crate::run::now() - 24 * 3600;
    match s.db.samples_since(since) {
        Ok(samples) => Json(json!({"samples": samples})).into_response(),
        Err(e) => {
            tracing::warn!("reading samples failed: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "database error"})),
            )
                .into_response()
        }
    }
}

async fn logs_route(State(s): St, Query(q): Query<HashMap<String, String>>) -> Response {
    let level = q.get("level").map(String::as_str).unwrap_or("info");
    if !logs::valid_level(level) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "level must be error, warn, info, debug or trace"})),
        )
            .into_response();
    }
    Json(json!({"lines": logs::tail(&s.log_path(), level)})).into_response()
}

fn storage_root(s: &AppState) -> std::path::PathBuf {
    let cfg = s.config.read().unwrap();
    s.net_dir.join(&cfg.storage.path)
}

async fn services(State(s): St) -> Response {
    match crate::registry::list(&s.db, &storage_root(&s)) {
        Ok(l) => Json(Value::Array(l)).into_response(),
        Err(e) => {
            tracing::warn!("listing services failed: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "database error"})),
            )
                .into_response()
        }
    }
}

async fn service_action(State(s): St, Path((hash, verb)): Path<(String, String)>) -> Response {
    use crate::registry::{act, Action, ActionError};
    let Some(action) = Action::parse(&verb) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unknown action"})),
        )
            .into_response();
    };
    let _guard = s.config_write.lock().unwrap();
    let mut cfg = s.config.read().unwrap().clone();
    let root = storage_root(&s);
    match act(&s.db, &mut cfg, &s.config_path, &root, &hash, action) {
        Ok(e) => {
            *s.config.write().unwrap() = cfg;
            tracing::info!("service {hash}: {verb}");
            Json(e).into_response()
        }
        Err(e) => {
            let code = match e {
                ActionError::Unknown => StatusCode::NOT_FOUND,
                ActionError::Conflict(_) => StatusCode::CONFLICT,
                ActionError::Other(_) => StatusCode::INTERNAL_SERVER_ERROR,
            };
            (code, Json(json!({"error": e.message()}))).into_response()
        }
    }
}
