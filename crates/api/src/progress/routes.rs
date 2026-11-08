use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use uuid::Uuid;

use crate::auth::extractor::AuthUser;
use crate::error::ApiError;
use crate::progress::dto::{
    ContinueEnvelope, ContinueItem, CourseProgress, CourseProgressEnvelope,
};
use crate::progress::service;
use crate::ratelimit::{self, policies};
use crate::state::AppState;

/// Mounted under `/v1/me`.
pub fn me_router() -> Router<AppState> {
    Router::new().route("/continue-watching", get(continue_watching))
}

/// Merged into the `/v1/courses` router.
pub fn course_router() -> Router<AppState> {
    Router::new().route("/:id/progress", get(course_progress))
}

async fn continue_watching(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<ContinueEnvelope>, ApiError> {
    ratelimit::enforce_user(&state, &policies::PROGRESS_READ, user.id).await?;
    let rows = service::continue_watching(&state, &user).await?;
    Ok(Json(ContinueEnvelope {
        items: rows
            .iter()
            .map(|row| ContinueItem::build(&state, row))
            .collect(),
    }))
}

async fn course_progress(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<CourseProgressEnvelope>, ApiError> {
    ratelimit::enforce_user(&state, &policies::PROGRESS_READ, user.id).await?;
    let id = Uuid::parse_str(&id).map_err(|_| ApiError::Http {
        status: StatusCode::NOT_FOUND,
        code: "not_found",
        message: "Course not found.".into(),
    })?;
    let rows = service::course_progress(&state, &user, id).await?;
    Ok(Json(CourseProgressEnvelope {
        progress: CourseProgress::build(&rows),
    }))
}
