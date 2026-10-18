//! Dashboard HTTP server.

mod routes;

use crate::app::AppState;
use axum::Router;
use std::sync::Arc;

pub fn router(state: Arc<AppState>) -> Router {
    routes::routes().with_state(state)
}
