//! Dashboard HTTP server.

mod auth;
mod routes;
mod settings;

use crate::app::AppState;
pub use auth::Session;
use axum::Router;
use std::sync::Arc;

pub fn router(state: Arc<AppState>) -> Router {
    routes::routes()
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::guard,
        ))
        .with_state(state)
}
