use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};

use crate::auth::extractor::AuthUser;
use crate::error::{ApiError, ErrorBody};
use crate::quota::dto::StorageEnvelope;
use crate::quota::service;
use crate::ratelimit::{self, policies};
use crate::state::AppState;

/// Mounted under `/v1/me`.
pub fn me_router() -> Router<AppState> {
    Router::new().route("/storage", get(show))
}

/// How much of their storage the signed-in person has used.
///
/// Every video they have uploaded and still have counts at the size it was sent, whatever stage it has reached;
/// deleted and failed ones do not. The limit is the account's own, or the service's default. When an upload would
/// not fit it is refused with `413 storage_quota_exceeded`.
#[utoipa::path(
    get,
    path = "/v1/me/storage",
    operation_id = "storage_show",
    tag = "account",
    responses(
        (status = 200, description = "Used, allowed and left, in bytes.", body = StorageEnvelope),
        (status = 401, description = "`unauthenticated`", body = ErrorBody),
        (status = 429, description = "`too_many_requests`", body = ErrorBody, headers(("Retry-After" = u64, description = "Seconds until the budget is available again."))),
    ),
    security(("session" = []))
)]
pub(crate) async fn show(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<StorageEnvelope>, ApiError> {
    ratelimit::enforce_user(&state, &policies::STORAGE_READ, user.id).await?;
    Ok(Json(StorageEnvelope {
        storage: service::usage(&state, user.id).await?.into(),
    }))
}
