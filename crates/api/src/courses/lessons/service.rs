use std::collections::HashSet;

use axum::http::StatusCode;
use uuid::Uuid;
use zeno_core::model::{Role, VideoStatus};

use crate::auth::extractor::AuthUser;
use crate::courses::lessons::dto::{LessonChanges, NewLesson};
use crate::courses::lessons::repo::{self, LessonRow, LessonValues, NewLessonRow};
use crate::error::{ApiError, FieldError};
use crate::state::AppState;

const COURSE_ROLES: &[Role] = &[Role::Creator, Role::Admin];

fn admin(user: &AuthUser) -> bool {
    user.role == Role::Admin
}

fn not_found(subject: &'static str) -> ApiError {
    ApiError::Http {
        status: StatusCode::NOT_FOUND,
        code: "not_found",
        message: format!("{subject} not found."),
    }
}

fn validation(field: &'static str, message: &'static str) -> ApiError {
    ApiError::validation(vec![FieldError::new(field, message)])
}

fn validate_video_status(status: VideoStatus) -> Result<(), ApiError> {
    if matches!(status, VideoStatus::Processing | VideoStatus::Ready) {
        Ok(())
    } else {
        Err(validation(
            "video_id",
            "Video must be processing or ready before it can be attached.",
        ))
    }
}

pub async fn create(
    state: &AppState,
    user: &AuthUser,
    course_id: Uuid,
    input: NewLesson,
) -> Result<(LessonRow, i32), ApiError> {
    user.require_role(COURSE_ROLES)?;
    let mut tx = state.db.begin().await?;
    let course = repo::lock_course(&mut tx, course_id, user.id, admin(user))
        .await?
        .ok_or_else(|| not_found("Course"))?;
    let video = repo::find_video(&mut tx, input.video_id, course.creator_id)
        .await?
        .ok_or_else(|| not_found("Video"))?;
    validate_video_status(video.status)?;

    let count = repo::active_count(&mut tx, course_id).await?;
    let last_position = i32::try_from(count + 1)
        .map_err(|_| ApiError::Internal(anyhow::anyhow!("lesson position overflow")))?;
    let position = input.position.unwrap_or(last_position);
    if position > last_position {
        return Err(validation(
            "position",
            "Position cannot be greater than the next available position.",
        ));
    }
    repo::defer_positions(&mut tx).await?;
    repo::shift_for_insert(&mut tx, course_id, position).await?;
    let lesson_id = repo::insert(
        &mut tx,
        &NewLessonRow {
            course_id,
            title: &input.title,
            description: &input.description,
            position,
            video_id: video.id,
            is_free_preview: input.is_free_preview,
        },
    )
    .await?;
    let version = repo::bump_course_version(&mut tx, course_id).await?;
    let lesson = repo::find_one(&mut tx, lesson_id).await?;
    tx.commit().await?;
    Ok((lesson, version))
}

pub async fn list(
    state: &AppState,
    user: &AuthUser,
    course_id: Uuid,
) -> Result<(Vec<LessonRow>, i32), ApiError> {
    user.require_role(COURSE_ROLES)?;
    let mut tx = state.db.begin().await?;
    sqlx::query("set transaction isolation level repeatable read read only")
        .execute(&mut *tx)
        .await?;
    let course = repo::find_course(&mut tx, course_id, user.id, admin(user))
        .await?
        .ok_or_else(|| not_found("Course"))?;
    let lessons = repo::list_in_transaction(&mut tx, course_id).await?;
    tx.commit().await?;
    Ok((lessons, course.version))
}

pub async fn update(
    state: &AppState,
    user: &AuthUser,
    lesson_id: Uuid,
    changes: LessonChanges,
) -> Result<(LessonRow, i32), ApiError> {
    user.require_role(COURSE_ROLES)?;
    let mut tx = state.db.begin().await?;
    let course = repo::lock_course_for_lesson(&mut tx, lesson_id, user.id, admin(user))
        .await?
        .ok_or_else(|| not_found("Lesson"))?;
    let current = repo::lock_lesson(&mut tx, lesson_id, course.id)
        .await?
        .ok_or_else(|| not_found("Lesson"))?;
    let video_id = changes.video_id.unwrap_or(current.video_id);
    if changes.video_id.is_some() {
        let video = repo::find_video(&mut tx, video_id, course.creator_id)
            .await?
            .ok_or_else(|| not_found("Video"))?;
        validate_video_status(video.status)?;
    }
    repo::update(
        &mut tx,
        lesson_id,
        &LessonValues {
            title: changes.title.as_deref().unwrap_or(&current.title),
            description: changes
                .description
                .as_deref()
                .unwrap_or(&current.description),
            video_id,
            is_free_preview: changes.is_free_preview.unwrap_or(current.is_free_preview),
        },
    )
    .await?;
    let version = repo::bump_course_version(&mut tx, course.id).await?;
    let lesson = repo::find_one(&mut tx, lesson_id).await?;
    tx.commit().await?;
    Ok((lesson, version))
}

pub async fn delete(state: &AppState, user: &AuthUser, lesson_id: Uuid) -> Result<(), ApiError> {
    user.require_role(COURSE_ROLES)?;
    let mut tx = state.db.begin().await?;
    let course = repo::lock_course_for_lesson(&mut tx, lesson_id, user.id, admin(user))
        .await?
        .ok_or_else(|| not_found("Lesson"))?;
    let current = repo::lock_lesson(&mut tx, lesson_id, course.id)
        .await?
        .ok_or_else(|| not_found("Lesson"))?;
    repo::defer_positions(&mut tx).await?;
    repo::soft_delete(&mut tx, lesson_id, course.id, current.position).await?;
    repo::bump_course_version(&mut tx, course.id).await?;
    tx.commit().await?;
    Ok(())
}

pub async fn reorder(
    state: &AppState,
    user: &AuthUser,
    course_id: Uuid,
    lesson_ids: Vec<Uuid>,
) -> Result<(Vec<LessonRow>, i32), ApiError> {
    user.require_role(COURSE_ROLES)?;
    let mut tx = state.db.begin().await?;
    let course = repo::lock_course(&mut tx, course_id, user.id, admin(user))
        .await?
        .ok_or_else(|| not_found("Course"))?;
    let current = repo::active_ids(&mut tx, course_id).await?;
    let requested: HashSet<_> = lesson_ids.iter().copied().collect();
    let existing: HashSet<_> = current.iter().copied().collect();
    if requested.len() != lesson_ids.len()
        || lesson_ids.len() != current.len()
        || requested != existing
    {
        return Err(validation(
            "lesson_ids",
            "Send every active lesson ID exactly once.",
        ));
    }
    if lesson_ids == current {
        let lessons = repo::list_in_transaction(&mut tx, course_id).await?;
        tx.commit().await?;
        return Ok((lessons, course.version));
    }
    repo::defer_positions(&mut tx).await?;
    repo::reorder(&mut tx, course_id, &lesson_ids).await?;
    let version = repo::bump_course_version(&mut tx, course_id).await?;
    let lessons = repo::list_in_transaction(&mut tx, course_id).await?;
    tx.commit().await?;
    Ok((lessons, version))
}
