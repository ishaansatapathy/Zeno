use axum::http::StatusCode;
use uuid::Uuid;
use zeno_core::model::Role;

use crate::auth::extractor::AuthUser;
use crate::cursor::{paginate, TimeCursor};
use crate::error::ApiError;
use crate::ratelimit::{enforce_user, policies};
use crate::recordings::dto::{NewRecording, RecordingChanges};
use crate::recordings::repo::{self, RecordingRow};
use crate::state::AppState;
use crate::videos;

/// How long a deleted recording's row is kept before the hourly sweep removes it for good. The
/// person's own view of it ended at once; this is the window in which support can still explain
/// what happened to it.
pub const KEEP_DELETED_DAYS: i32 = 30;
/// Rows removed per statement, and statements per sweep: a backlog is worked off over several
/// sweeps instead of in one long transaction.
const PURGE_BATCH: i64 = 500;
const PURGE_BATCHES: usize = 20;

/// Who may keep a library: the same people who may upload.
const ROLES: &[Role] = &[Role::Creator, Role::Admin];

fn not_found() -> ApiError {
    ApiError::Http {
        status: StatusCode::NOT_FOUND,
        code: "not_found",
        message: "Recording not found.".into(),
    }
}

fn conflict(code: &'static str, message: &'static str) -> ApiError {
    ApiError::Http {
        status: StatusCode::CONFLICT,
        code,
        message: message.into(),
    }
}

fn unique_violation(err: &sqlx::Error) -> bool {
    matches!(err, sqlx::Error::Database(db) if db.is_unique_violation())
}

/// The caller's own upload, or `video_not_found`: a video that is someone else's looks exactly
/// like one that does not exist.
async fn own_video(state: &AppState, user: &AuthUser, id: Uuid) -> Result<(), ApiError> {
    match videos::repo::find(&state.db, id).await? {
        Some(video) if video.owner_id == user.id => Ok(()),
        _ => Err(ApiError::Http {
            status: StatusCode::NOT_FOUND,
            code: "video_not_found",
            message: "Video not found.".into(),
        }),
    }
}

async fn load(state: &AppState, user: &AuthUser, id: Uuid) -> Result<RecordingRow, ApiError> {
    repo::find_owned(&state.db, id, user.id)
        .await?
        .ok_or_else(not_found)
}

/// Makes a recording, or finds the one an earlier attempt with the same id made. The bool says
/// whether this call created it.
pub async fn create(
    state: &AppState,
    user: &AuthUser,
    input: NewRecording,
) -> Result<(RecordingRow, bool), ApiError> {
    user.require_role(ROLES)?;
    enforce_user(state, &policies::RECORDING_WRITE, user.id).await?;
    if let Some(video) = input.video_id {
        own_video(state, user, video).await?;
    }

    let mut tx = state.db.begin().await?;
    let created = repo::insert(&mut tx, input.id, user.id, input.source, &input.title).await?;
    if created {
        if let Some(video) = input.video_id {
            let attached = repo::attach_video(&mut tx, input.id, user.id, video).await;
            match attached {
                Ok(_) => {}
                Err(err) if unique_violation(&err) => {
                    return Err(conflict(
                        "video_in_use",
                        "That video already belongs to another recording.",
                    ));
                }
                Err(err) => return Err(err.into()),
            }
        }
        tx.commit().await?;
    } else {
        tx.rollback().await?;
        // The id is taken. If it is this person's own earlier attempt, that is the answer; a
        // different person's, a deleted one, or one that differs in what it is, is a conflict.
        let theirs = repo::existing(&state.db, input.id).await?;
        let same = theirs.is_some_and(|row| {
            row.owner_id == user.id
                && row.deleted_at.is_none()
                && row.source == input.source
                && input
                    .video_id
                    .is_none_or(|video| row.video_id == Some(video))
        });
        if !same {
            return Err(conflict(
                "id_conflict",
                "That recording id is already in use.",
            ));
        }
    }
    Ok((load(state, user, input.id).await?, created))
}

pub async fn get(state: &AppState, user: &AuthUser, id: Uuid) -> Result<RecordingRow, ApiError> {
    user.require_role(ROLES)?;
    load(state, user, id).await
}

pub async fn list(
    state: &AppState,
    user: &AuthUser,
    source: Option<&'static str>,
    cursor: Option<TimeCursor>,
    requested_limit: Option<u32>,
) -> Result<(Vec<RecordingRow>, Option<TimeCursor>), ApiError> {
    user.require_role(ROLES)?;
    let limit = requested_limit.unwrap_or(20).clamp(1, 50) as usize;
    let rows = repo::list_owned(&state.db, user.id, source, cursor, limit as i64 + 1).await?;
    Ok(paginate(rows, limit, |row| TimeCursor {
        created_at: row.created_at,
        id: row.id,
    }))
}

pub async fn update(
    state: &AppState,
    user: &AuthUser,
    id: Uuid,
    expected_version: i32,
    changes: RecordingChanges,
) -> Result<RecordingRow, ApiError> {
    user.require_role(ROLES)?;
    let changed = repo::update(
        &state.db,
        id,
        user.id,
        expected_version,
        changes.title.as_deref(),
        changes.visibility,
    )
    .await?;
    if !changed {
        // Either it is not theirs (or gone), or someone changed it since they read it.
        load(state, user, id).await?;
        return Err(conflict(
            "version_conflict",
            "This recording changed elsewhere. Reload it and try again.",
        ));
    }
    if changes.visibility == Some("private") {
        // Every link stops working at once: viewings under way end now rather than at their
        // next request (which would refuse them anyway).
        crate::playback::repo::end_for_recording_links(&state.db, id, "access_revoked").await?;
    }
    load(state, user, id).await
}

/// Gives a recording its video. Safe to repeat with the same video; a second, different one is
/// refused rather than silently replacing the first.
pub async fn attach_video(
    state: &AppState,
    user: &AuthUser,
    id: Uuid,
    video: Uuid,
) -> Result<RecordingRow, ApiError> {
    user.require_role(ROLES)?;
    enforce_user(state, &policies::RECORDING_WRITE, user.id).await?;
    let current = load(state, user, id).await?;
    own_video(state, user, video).await?;
    if current.video_id == Some(video) {
        return Ok(current);
    }
    let mut tx = state.db.begin().await?;
    let attached = match repo::attach_video(&mut tx, id, user.id, video).await {
        Ok(attached) => attached,
        Err(err) if unique_violation(&err) => {
            return Err(conflict(
                "video_in_use",
                "That video already belongs to another recording.",
            ));
        }
        Err(err) => return Err(err.into()),
    };
    if !attached {
        // It had a video by the time we got here (a concurrent attach won).
        tx.rollback().await?;
        let now = load(state, user, id).await?;
        if now.video_id == Some(video) {
            return Ok(now);
        }
        return Err(conflict(
            "video_already_attached",
            "This recording already has a video.",
        ));
    }
    tx.commit().await?;
    load(state, user, id).await
}

/// Removes recordings deleted more than [`KEEP_DELETED_DAYS`] ago. Returns how many.
pub async fn purge_deleted(state: &AppState) -> Result<u64, ApiError> {
    let mut total = 0;
    for _ in 0..PURGE_BATCHES {
        let removed = repo::purge_deleted(&state.db, KEEP_DELETED_DAYS, PURGE_BATCH).await?;
        total += removed;
        if removed < PURGE_BATCH as u64 {
            break;
        }
    }
    Ok(total)
}

/// Deletes a recording and, unless a course still uses it, the video it was made of.
pub async fn delete(state: &AppState, user: &AuthUser, id: Uuid) -> Result<(), ApiError> {
    user.require_role(ROLES)?;
    let video = repo::soft_delete(&state.db, id, user.id)
        .await?
        .ok_or_else(not_found)?;
    crate::playback::repo::end_for_recording_links(&state.db, id, "access_revoked").await?;
    let Some(video) = video else {
        return Ok(());
    };
    // The recording is gone either way; freeing its storage is a separate, best-effort step. If it
    // fails the video is still the owner's to delete, and nothing is exposed.
    match repo::video_in_use(&state.db, video).await {
        Ok(false) => {}
        // Still in a course, or we could not tell: keep the video.
        Ok(true) => return Ok(()),
        Err(err) => {
            tracing::warn!(error = ?err, recording_id = %id, "could not tell whether a course uses the video");
            return Ok(());
        }
    }
    if let Err(err) = videos::service::delete(state, user, video).await {
        tracing::warn!(error = ?err, recording_id = %id, video_id = %video, "recording deleted but its video was not");
    }
    Ok(())
}
