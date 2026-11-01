//! HTTP API: auth, courses, videos, playback manifests and payment webhooks.
use axum::{
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use zeno_core::config::Config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let config = Config::from_env();

    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/auth/register", post(not_implemented))
        .route("/v1/auth/login", post(not_implemented))
        .route("/v1/auth/refresh", post(not_implemented))
        .route("/v1/courses", post(not_implemented))
        .route("/v1/courses/:id", get(not_implemented).patch(not_implemented))
        .route("/v1/courses/:id/lessons", post(not_implemented))
        .route("/v1/videos", post(not_implemented))
        .route("/v1/videos/:id", get(not_implemented))
        .route("/v1/play/:lesson_id/master.m3u8", get(not_implemented))
        .route("/v1/play/:session_id/heartbeat", post(not_implemented))
        .route("/v1/checkout/:course_id", post(not_implemented))
        .route("/v1/webhooks/razorpay", post(not_implemented))
        .route("/v1/analytics/courses/:id", get(not_implemented));

    let listener = tokio::net::TcpListener::bind(&config.bind_addr).await?;
    tracing::info!("api listening on {}", config.bind_addr);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health() -> &'static str {
    "ok"
}

async fn not_implemented() -> (StatusCode, Json<Value>) {
    (StatusCode::NOT_IMPLEMENTED, Json(json!({ "error": "not implemented" })))
}
