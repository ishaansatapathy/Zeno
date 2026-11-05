//! Zeno HTTP API.

pub mod auth;
pub mod config;
pub mod error;
pub mod health;
pub mod mail;
pub mod net;
pub mod security;
pub mod state;
pub mod stubs;

use axum::extract::DefaultBodyLimit;
use axum::middleware::from_fn_with_state;
use axum::Router;
use tower_http::trace::TraceLayer;

use crate::state::AppState;

const MAX_BODY_BYTES: usize = 64 * 1024;

pub fn build_app(state: AppState) -> Router {
    let cors = security::cors(&state.config.client_origin);
    Router::new()
        .merge(health::router())
        .nest("/v1/auth", auth::routes::router())
        .merge(stubs::router())
        .layer(from_fn_with_state(
            state.clone(),
            security::require_same_origin,
        ))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
