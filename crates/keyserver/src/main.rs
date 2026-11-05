//! Token validation and AES-128 key delivery for signed playback requests.
use axum::{extract::Path, http::StatusCode, routing::get, Json, Router};
use serde_json::{json, Value};
use uuid::Uuid;
use zeno_core::config::Config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let config = Config::from_env("0.0.0.0:8081")?;

    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/keys/:video_id", get(get_key));

    let listener = tokio::net::TcpListener::bind(&config.bind_addr).await?;
    tracing::info!("keyserver listening on {}", config.bind_addr);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health() -> &'static str {
    "ok"
}

async fn get_key(Path(_video_id): Path<Uuid>) -> (StatusCode, Json<Value>) {
    // TODO: validate the signed token, confirm enrollment and session limits, return the key.
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({ "error": "not implemented" })),
    )
}
