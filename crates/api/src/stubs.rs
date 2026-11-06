//! Routes from the planned REST API that are not built yet. They answer 501 rather than
//! 404 so clients can tell "not implemented" from "wrong URL".
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/courses", post(not_implemented))
        .route(
            "/v1/courses/:id",
            get(not_implemented).patch(not_implemented),
        )
        .route("/v1/courses/:id/lessons", post(not_implemented))
        .route("/v1/play/:lesson_id/master.m3u8", get(not_implemented))
        .route("/v1/play/:session_id/heartbeat", post(not_implemented))
        .route("/v1/checkout/:course_id", post(not_implemented))
        .route("/v1/webhooks/razorpay", post(not_implemented))
        .route("/v1/analytics/courses/:id", get(not_implemented))
}

async fn not_implemented() -> (StatusCode, Json<Value>) {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({ "error": { "code": "not_implemented", "message": "Not implemented yet." } })),
    )
}
