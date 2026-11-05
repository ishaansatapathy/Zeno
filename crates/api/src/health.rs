use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::error::ApiError;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
}

/// Liveness: the process is up. Deliberately touches no dependency, so an outage in the
/// database doesn't make the orchestrator restart healthy API pods.
async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

/// Readiness: can this instance actually serve traffic right now?
async fn ready(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    sqlx::query("select 1")
        .execute(&state.db)
        .await
        .map_err(|err| {
            tracing::error!(error = ?err, "readiness: database check failed");
            ApiError::Unavailable("database unavailable")
        })?;
    state.counters.ping().await.map_err(|err| {
        tracing::error!(error = ?err, "readiness: redis check failed");
        ApiError::Unavailable("redis unavailable")
    })?;
    Ok(Json(json!({ "status": "ready" })))
}
