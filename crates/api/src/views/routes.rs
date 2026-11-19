use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::extractor::AuthUser;
use crate::error::ApiError;
use crate::state::AppState;
use crate::views::service::{self, ViewsEnvelope};

/// The owner's side, merged into `/v1/recordings`.
pub fn recording_router() -> Router<AppState> {
    Router::new().route("/:id/views", get(show))
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct ViewsQuery {
    days: Option<u32>,
}

async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<ViewsQuery>,
) -> Result<Json<ViewsEnvelope>, ApiError> {
    // An id that is not a UUID names nothing.
    let id = Uuid::parse_str(&id).map_err(|_| ApiError::Http {
        status: StatusCode::NOT_FOUND,
        code: "not_found",
        message: "Recording not found.".into(),
    })?;
    let days = service::validate_days(query.days)?;
    Ok(Json(ViewsEnvelope {
        views: service::show(&state, &user, id, days).await?,
    }))
}
