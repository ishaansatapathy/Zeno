//! Live class control: stream-key check against the scheduled class, and the
//! live HLS endpoints. The RTMP ingest itself runs in MediaMTX; this service
//! authorizes it and fronts the rolling playlist.
use axum::{
    extract::Path,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use uuid::Uuid;
use zeno_core::config::Config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let config = Config::from_env("0.0.0.0:8082")?;

    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/live", post(schedule_class))
        .route("/v1/live/:id", get(get_class))
        .route("/v1/live/:id/index.m3u8", get(get_playlist))
        .route("/v1/live/stream-key-check", post(check_stream_key));

    let listener = tokio::net::TcpListener::bind(&config.bind_addr).await?;
    tracing::info!("live listening on {}", config.bind_addr);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health() -> &'static str {
    "ok"
}

async fn schedule_class() -> (StatusCode, Json<Value>) {
    not_implemented()
}

async fn get_class(Path(_id): Path<Uuid>) -> (StatusCode, Json<Value>) {
    not_implemented()
}

async fn get_playlist(Path(_id): Path<Uuid>) -> (StatusCode, Json<Value>) {
    not_implemented()
}

async fn check_stream_key() -> (StatusCode, Json<Value>) {
    // TODO: called by MediaMTX on publish; validate stream key against the scheduled class.
    not_implemented()
}

fn not_implemented() -> (StatusCode, Json<Value>) {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({ "error": "not implemented" })),
    )
}
