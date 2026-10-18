//! Read API routes.

use crate::app::AppState;
use crate::logs;
use axum::extract::{Query, State};
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
        .route("/api/resources", get(resources))
        .route("/api/resources/history", get(history))
        .route("/api/logs", get(logs_route))
        .route("/api/session", get(super::auth::session))
        .route("/api/login", post(super::auth::login))
        .route(
            "/api/settings",
            get(super::settings::get).put(super::settings::put),
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
