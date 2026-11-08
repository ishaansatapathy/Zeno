//! Zeno HTTP API.

pub mod auth;
pub mod config;
pub mod courses;
pub mod error;
pub mod health;
pub mod mail;
pub mod net;
pub mod outbox;
pub mod playback;
pub mod secret;
pub mod security;
pub mod state;
pub mod stubs;
pub mod videos;

use std::time::Duration;

use axum::body::Body;
use axum::extract::DefaultBodyLimit;
use axum::http::{HeaderName, Request, StatusCode};
use axum::middleware::from_fn_with_state;
use axum::Router;
use tower_http::classify::ServerErrorsFailureClass;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::{DefaultOnResponse, TraceLayer};
use tracing::{Level, Span};

use crate::state::AppState;

const MAX_BODY_BYTES: usize = 64 * 1024;
const REQUEST_ID_HEADER: &str = "x-request-id";

pub fn build_app(state: AppState) -> Router {
    let cors = security::cors(&state.config.client_origin);
    let request_id = HeaderName::from_static(REQUEST_ID_HEADER);

    // One span per request carrying the request id, so every log line from handling it can be
    // tied together. Only the path is recorded: query strings and headers can hold secrets.
    let trace = TraceLayer::new_for_http()
        .make_span_with(|request: &Request<Body>| {
            let id = request
                .headers()
                .get(REQUEST_ID_HEADER)
                .and_then(|value| value.to_str().ok())
                .unwrap_or("-");
            tracing::info_span!(
                "http",
                method = %request.method(),
                path = %request.uri().path(),
                request_id = %id,
            )
        })
        .on_response(DefaultOnResponse::new().level(Level::INFO))
        .on_failure(
            |class: ServerErrorsFailureClass, latency: Duration, _span: &Span| match class {
                // Routes that are planned but unbuilt aren't incidents.
                ServerErrorsFailureClass::StatusCode(StatusCode::NOT_IMPLEMENTED) => {
                    tracing::debug!(?latency, "not implemented");
                }
                other => tracing::error!(%other, ?latency, "request failed"),
            },
        );

    Router::new()
        .merge(health::router())
        .nest("/v1/auth", auth::routes::router())
        .nest("/v1/courses", courses::routes::router())
        .nest("/v1/lessons", courses::lessons::routes::item_router())
        .nest("/v1/videos", videos::routes::router())
        .nest("/v1/play", playback::routes::router())
        .merge(stubs::router())
        .layer(from_fn_with_state(
            state.clone(),
            security::require_same_origin,
        ))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(cors)
        .layer(PropagateRequestIdLayer::new(request_id.clone()))
        .layer(trace)
        .layer(SetRequestIdLayer::new(request_id, MakeRequestUuid))
        .with_state(state)
}
