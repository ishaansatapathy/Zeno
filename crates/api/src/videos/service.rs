use std::time::Duration;

use axum::http::StatusCode;
use chrono::Utc;
use serde_json::json;
use uuid::Uuid;
use zeno_core::jobs;
use zeno_core::model::{Role, VideoStatus};
use zeno_core::storage::StorageError;

use crate::auth::extractor::AuthUser;
use crate::auth::limits::{self, HOUR};
use crate::error::ApiError;
use crate::state::AppState;
use crate::videos::dto::{NewVideo, PartUrl, UploadResponse};
use crate::videos::plan::PartPlan;
use crate::videos::repo::{self, NewVideoRow, VideoRow};

/// Upload URLs stay valid this long; a client that needs more asks for fresh ones.
const URL_TTL: Duration = Duration::from_secs(60 * 60);
const MAX_UPLOADS_PER_HOUR: u64 = 30;
const MAX_UNFINISHED_UPLOADS: i64 = 10;
/// An upload nobody finished within this long is abandoned and cleaned up.
pub const STALE_UPLOAD_HOURS: i32 = 24;
/// Source objects are never served, so they carry a type no browser will render.
const SOURCE_CONTENT_TYPE: &str = "application/octet-stream";
/// How long a request that lost a completion race waits for the winner to commit.
const COMPLETION_WAIT: Duration = Duration::from_secs(20);
const CREATOR_ROLES: &[Role] = &[Role::Creator, Role::Admin];

fn not_found() -> ApiError {
    ApiError::Http {
        status: StatusCode::NOT_FOUND,
        code: "not_found",
        message: "Video not found.".into(),
    }
}

fn conflict(code: &'static str, message: impl Into<String>) -> ApiError {
    ApiError::Http {
        status: StatusCode::CONFLICT,
        code,
        message: message.into(),
    }
}

fn storage_error(err: StorageError) -> ApiError {
    match err {
        StorageError::Transport(_)
        | StorageError::Http {
            status: 500..=599, ..
        } => {
            tracing::error!(error = ?err, "storage unavailable");
            ApiError::Unavailable("storage unavailable")
        }
        other => ApiError::Internal(other.into()),
    }
}

fn source_key(video_id: Uuid) -> String {
    format!("uploads/{video_id}/source")
}

/// Someone else's video looks exactly like one that doesn't exist.
async fn load_owned(state: &AppState, user: &AuthUser, id: Uuid) -> Result<VideoRow, ApiError> {
    user.require_role(CREATOR_ROLES)?;
    repo::find(&state.db, id)
        .await?
        .filter(|video| video.owner_id == user.id)
        .ok_or_else(not_found)
}

fn plan_of(video: &VideoRow) -> PartPlan {
    PartPlan::from_parts(video.size_bytes as u64, video.part_size as u64)
}

fn part_urls(state: &AppState, video: &VideoRow, upload_id: &str, numbers: &[u16]) -> Vec<PartUrl> {
    let plan = plan_of(video);
    numbers
        .iter()
        .filter_map(|&number| {
            let size = plan.size_of(number)?;
            let url = state
                .store
                .presign_part(&video.source_key, upload_id, number, size, URL_TTL);
            Some(PartUrl {
                number,
                size,
                url: url.to_string(),
            })
        })
        .collect()
}

fn upload_response(
    state: &AppState,
    video: &VideoRow,
    upload_id: &str,
    uploaded: Vec<u16>,
    missing: &[u16],
) -> UploadResponse {
    let plan = plan_of(video);
    UploadResponse {
        part_size: plan.part_size,
        total_parts: plan.count,
        uploaded,
        parts: part_urls(state, video, upload_id, missing),
        expires_at: Utc::now() + chrono::Duration::from_std(URL_TTL).unwrap_or_default(),
    }
}

pub async fn create(
    state: &AppState,
    user: &AuthUser,
    input: NewVideo,
) -> Result<(VideoRow, UploadResponse), ApiError> {
    user.require_role(CREATOR_ROLES)?;
    limits::enforce(
        &*state.counters,
        &format!("video:create:{}", user.id),
        MAX_UPLOADS_PER_HOUR,
        HOUR,
    )
    .await?;
    if repo::count_uploading(&state.db, user.id).await? >= MAX_UNFINISHED_UPLOADS {
        return Err(ApiError::Http {
            status: StatusCode::TOO_MANY_REQUESTS,
            code: "too_many_uploads",
            message: "Finish or cancel your unfinished uploads before starting another.".into(),
        });
    }

    let id = Uuid::new_v4();
    let key = source_key(id);
    let plan = PartPlan::new(input.size_bytes);
    let upload_id = state
        .store
        .create_multipart(&key, SOURCE_CONTENT_TYPE)
        .await
        .map_err(storage_error)?;

    let row = NewVideoRow {
        id,
        owner_id: user.id,
        title: &input.title,
        original_filename: &input.filename,
        content_type: &input.content_type,
        size_bytes: input.size_bytes as i64,
        part_size: plan.part_size as i64,
        source_key: &key,
        upload_id: &upload_id,
    };
    let video = match repo::insert(&state.db, &row).await {
        Ok(video) => video,
        Err(err) => {
            // Don't leave a multipart upload (which storage bills for) with no row pointing at it.
            if let Err(abort_err) = state.store.abort_multipart(&key, &upload_id).await {
                tracing::error!(error = ?abort_err, "could not abort orphaned upload");
            }
            return Err(err.into());
        }
    };
    let numbers: Vec<u16> = plan.numbers().collect();
    let upload = upload_response(state, &video, &upload_id, Vec::new(), &numbers);
    Ok((video, upload))
}

pub async fn get(state: &AppState, user: &AuthUser, id: Uuid) -> Result<VideoRow, ApiError> {
    let video = repo::find(&state.db, id).await?.ok_or_else(not_found)?;
    if video.owner_id != user.id && user.role != Role::Admin {
        return Err(not_found());
    }
    Ok(video)
}

/// For a client that was interrupted or whose URLs expired: what storage already has, and
/// fresh URLs for only the parts that are missing (or have the wrong size and must be redone).
pub async fn resume(
    state: &AppState,
    user: &AuthUser,
    id: Uuid,
) -> Result<(VideoRow, UploadResponse), ApiError> {
    let video = load_owned(state, user, id).await?;
    let (Some(upload_id), VideoStatus::Uploading) = (video.upload_id.clone(), video.status) else {
        return Err(conflict(
            "not_uploading",
            "This video is no longer accepting uploads.",
        ));
    };
    let plan = plan_of(&video);
    let held = match state.store.list_parts(&video.source_key, &upload_id).await {
        Ok(parts) => parts,
        Err(StorageError::NotFound) => Vec::new(),
        Err(err) => return Err(storage_error(err)),
    };
    let (uploaded, missing) = split_parts(&plan, &held);
    let response = upload_response(state, &video, &upload_id, uploaded, &missing);
    Ok((video, response))
}

/// Splits the plan into parts storage holds correctly and parts still needed.
fn split_parts(plan: &PartPlan, held: &[zeno_core::storage::PartInfo]) -> (Vec<u16>, Vec<u16>) {
    let mut uploaded = Vec::new();
    let mut missing = Vec::new();
    for number in plan.numbers() {
        let complete = held
            .iter()
            .any(|part| part.number == number && Some(part.size) == plan.size_of(number));
        if complete {
            uploaded.push(number);
        } else {
            missing.push(number);
        }
    }
    (uploaded, missing)
}

/// Verifies against storage (never the client) that every part is present with exactly the
/// planned length, stitches them together, then queues transcoding in the same transaction
/// that marks the video as processing. Safe to call repeatedly.
pub async fn complete(state: &AppState, user: &AuthUser, id: Uuid) -> Result<VideoRow, ApiError> {
    let video = load_owned(state, user, id).await?;
    match video.status {
        VideoStatus::Processing | VideoStatus::Ready => return Ok(video),
        VideoStatus::Failed => {
            return Err(conflict(
                "video_failed",
                "This video could not be processed.",
            ))
        }
        VideoStatus::Uploading => {}
    }
    let upload_id = video.upload_id.clone().ok_or_else(|| {
        conflict(
            "not_uploading",
            "This video is no longer accepting uploads.",
        )
    })?;
    let plan = plan_of(&video);

    let held = match state.store.list_parts(&video.source_key, &upload_id).await {
        Ok(parts) => parts,
        Err(StorageError::NotFound) => {
            return already_completed_or(state, id, "upload_expired").await
        }
        Err(err) => return Err(storage_error(err)),
    };
    let (_, missing) = split_parts(&plan, &held);
    if !missing.is_empty() || held.len() != usize::from(plan.count) {
        return Err(conflict(
            "upload_incomplete",
            format!(
                "Parts {} are missing or have the wrong size. Request fresh URLs and upload them.",
                missing
                    .iter()
                    .map(u16::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));
    }

    let etags: Vec<String> = held.iter().map(|part| part.etag.clone()).collect();
    match state
        .store
        .complete_multipart(&video.source_key, &upload_id, &etags)
        .await
    {
        Ok(()) => {}
        // A concurrent call may have completed it between our read and now.
        Err(StorageError::NotFound) => {
            return already_completed_or(state, id, "upload_expired").await
        }
        Err(err) => return Err(storage_error(err)),
    }

    let stored = state
        .store
        .head(&video.source_key)
        .await
        .map_err(storage_error)?;
    if stored != Some(video.size_bytes as u64) {
        tracing::error!(video_id = %id, expected = video.size_bytes, ?stored, "assembled object has the wrong size");
        repo::mark_failed(&state.db, id, "size_mismatch").await?;
        if let Err(err) = state.store.delete(&video.source_key).await {
            tracing::error!(error = ?err, "could not delete mismatched upload");
        }
        return Err(conflict(
            "size_mismatch",
            "The uploaded file does not match its declared size.",
        ));
    }

    let mut tx = state.db.begin().await?;
    let updated = repo::mark_processing(&mut *tx, id).await?;
    if updated.is_some() {
        jobs::enqueue(&mut *tx, "transcode", &json!({ "video_id": id })).await?;
    }
    tx.commit().await?;
    match updated {
        Some(video) => Ok(video),
        None => already_completed_or(state, id, "upload_expired").await,
    }
}

/// Called when storage says the upload is already assembled (or gone). Usually that means a
/// concurrent request is completing it right now and has not committed yet, so wait briefly
/// for it to finish; only if the video never moves on is the upload really lost.
async fn already_completed_or(
    state: &AppState,
    id: Uuid,
    code: &'static str,
) -> Result<VideoRow, ApiError> {
    let deadline = tokio::time::Instant::now() + COMPLETION_WAIT;
    loop {
        let video = repo::find(&state.db, id).await?.ok_or_else(not_found)?;
        match video.status {
            VideoStatus::Processing | VideoStatus::Ready => return Ok(video),
            VideoStatus::Failed => {
                return Err(conflict(
                    "video_failed",
                    "This video could not be processed.",
                ))
            }
            VideoStatus::Uploading if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            VideoStatus::Uploading => {
                return Err(conflict(
                    code,
                    "This upload is no longer available. Start a new upload.",
                ))
            }
        }
    }
}

pub async fn abort(state: &AppState, user: &AuthUser, id: Uuid) -> Result<(), ApiError> {
    let video = load_owned(state, user, id).await?;
    let (Some(upload_id), VideoStatus::Uploading) = (video.upload_id.clone(), video.status) else {
        return Err(conflict(
            "not_abortable",
            "Only an unfinished upload can be cancelled.",
        ));
    };
    state
        .store
        .abort_multipart(&video.source_key, &upload_id)
        .await
        .map_err(storage_error)?;
    repo::delete_uploading(&state.db, id).await?;
    Ok(())
}

/// Cleans up uploads nobody finished: storage keeps (and bills for) uploaded parts until the
/// multipart upload is aborted.
pub async fn abandon_stale_uploads(state: &AppState) -> Result<usize, ApiError> {
    let stale = repo::stale_uploads(&state.db, STALE_UPLOAD_HOURS, 50).await?;
    let mut cleaned = 0;
    for video in stale {
        if let Some(upload_id) = &video.upload_id {
            if let Err(err) = state
                .store
                .abort_multipart(&video.source_key, upload_id)
                .await
            {
                tracing::error!(error = ?err, video_id = %video.id, "could not abort stale upload");
                continue;
            }
        }
        if repo::delete_uploading(&state.db, video.id).await? {
            cleaned += 1;
        }
    }
    Ok(cleaned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::videos::plan::MIB;
    use zeno_core::storage::PartInfo;

    fn part(number: u16, size: u64) -> PartInfo {
        PartInfo {
            number,
            size,
            etag: format!("\"etag{number}\""),
        }
    }

    #[test]
    fn parts_of_the_wrong_size_count_as_missing_so_they_get_redone() {
        let plan = PartPlan::new(40 * MIB);
        assert_eq!(plan.count, 3);
        let held = [part(1, 16 * MIB), part(2, 16 * MIB - 1), part(3, 8 * MIB)];
        let (uploaded, missing) = split_parts(&plan, &held);
        assert_eq!((uploaded, missing), (vec![1, 3], vec![2]));
    }

    #[test]
    fn nothing_held_means_everything_is_missing() {
        let plan = PartPlan::new(1);
        assert_eq!(split_parts(&plan, &[]), (vec![], vec![1]));
    }

    #[test]
    fn storage_keys_never_contain_user_input() {
        let id = Uuid::nil();
        assert_eq!(
            source_key(id),
            "uploads/00000000-0000-0000-0000-000000000000/source"
        );
    }
}
