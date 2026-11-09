use axum::http::StatusCode;
use uuid::Uuid;

use crate::auth::extractor::AuthUser;
use crate::error::ApiError;
use crate::progress::logic::{self, Stored};
use crate::progress::repo::{self, ContinueRow, CourseLessonProgress};
use crate::state::AppState;

/// Folds a heartbeat into the viewer's progress for a lesson. Two screens heartbeating at once
/// take turns on the row lock, so credit is never counted twice.
pub async fn record(
    state: &AppState,
    user_id: Uuid,
    lesson_id: Uuid,
    video_id: Uuid,
    position: i32,
    duration: Option<i32>,
) -> Result<(), sqlx::Error> {
    let position = match duration {
        Some(d) => position.clamp(0, d.max(0)),
        None => position.max(0),
    };
    let mut tx = state.db.begin().await?;
    // Serialize with a creator replacing the lesson's video. Heartbeats from a session bound to
    // the old video must never recreate the progress that replacement just cleared.
    if !repo::still_this_video(&mut tx, lesson_id, video_id).await? {
        return Ok(());
    }
    if !repo::insert_first(&mut tx, user_id, lesson_id, Stored::first(position)).await? {
        if let Some((prev, elapsed)) = repo::lock(&mut tx, user_id, lesson_id).await? {
            let next = logic::advance(prev, position, elapsed, duration);
            repo::save(&mut tx, user_id, lesson_id, next).await?;
        }
    }
    tx.commit().await
}

pub async fn continue_watching(
    state: &AppState,
    user: &AuthUser,
) -> Result<Vec<ContinueRow>, ApiError> {
    Ok(repo::continue_watching(&state.db, user.id, 10).await?)
}

/// Progress through every lesson of a course the user is enrolled in. Anyone else gets the same
/// `404` as for a course that does not exist.
pub async fn course_progress(
    state: &AppState,
    user: &AuthUser,
    course_id: Uuid,
) -> Result<Vec<CourseLessonProgress>, ApiError> {
    if !repo::enrolled(&state.db, user.id, course_id).await? {
        return Err(ApiError::Http {
            status: StatusCode::NOT_FOUND,
            code: "not_found",
            message: "Course not found.".into(),
        });
    }
    Ok(repo::course_lessons(&state.db, user.id, course_id).await?)
}
