use axum::http::StatusCode;
use uuid::Uuid;

use crate::error::ApiError;
use crate::quota::repo::{self, Usage};
use crate::state::AppState;
use crate::videos::repo::{self as videos, NewVideoRow, VideoRow};

/// The default is kept as the database keeps it, a signed number.
fn default_quota(state: &AppState) -> i64 {
    i64::try_from(state.config.default_storage_quota_bytes).unwrap_or(i64::MAX)
}

pub async fn usage(state: &AppState, user_id: Uuid) -> Result<Usage, ApiError> {
    repo::usage(&state.db, user_id, default_quota(state), None)
        .await?
        .ok_or_else(|| ApiError::unauthorized("unauthenticated", "Sign in to continue."))
}

/// 1024-based, one decimal, for a sentence a person reads.
pub fn human(bytes: i64) -> String {
    const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
    let mut value = bytes.max(0) as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", value as i64, UNITS[0])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// What is said when there is not room. The numbers are in the sentence for a person; a program asks
/// `GET /v1/me/storage`.
pub fn exceeded(usage: &Usage, needed: i64) -> ApiError {
    ApiError::Http {
        status: StatusCode::PAYLOAD_TOO_LARGE,
        code: "storage_quota_exceeded",
        message: format!(
            "Not enough storage: this needs {} and {} of your {} is left. Delete recordings you no longer need to make room.",
            human(needed),
            human(usage.remaining()),
            human(usage.quota_bytes),
        ),
    }
}

/// A look before anything is made, so a request that cannot fit does not start an upload only to abandon it.
/// Not a promise: the check that counts is the one made under the lock when the video is inserted.
pub async fn check_room(state: &AppState, user_id: Uuid, needed: i64) -> Result<(), ApiError> {
    let usage = usage(state, user_id).await?;
    if usage.used_bytes.saturating_add(needed) > usage.quota_bytes {
        return Err(exceeded(&usage, needed));
    }
    Ok(())
}

pub enum InsertError {
    /// There is not room for this video.
    Full(Usage),
    Db(sqlx::Error),
}

impl From<sqlx::Error> for InsertError {
    fn from(err: sqlx::Error) -> Self {
        InsertError::Db(err)
    }
}

/// Adds the video if the person's room holds it, and not otherwise. The room is counted and the row added in
/// one transaction that holds the person's lock, so uploads started at the same moment cannot all be told
/// there is room for the last of it. `needed` is what to hold back: the size for a file with one, one byte for
/// a recording whose size is not known yet (its parts are checked as they come, see `check_part`).
pub async fn insert_within_room(
    state: &AppState,
    video: &NewVideoRow<'_>,
    needed: i64,
) -> Result<VideoRow, InsertError> {
    let mut tx = state.db.begin().await?;
    if !repo::lock(&mut *tx, video.owner_id).await? {
        // The account is gone: the insert would fail on the foreign key all the same.
        return Err(InsertError::Db(sqlx::Error::RowNotFound));
    }
    let usage = repo::usage(&mut *tx, video.owner_id, default_quota(state), None)
        .await?
        .ok_or(InsertError::Db(sqlx::Error::RowNotFound))?;
    if usage.used_bytes.saturating_add(needed) > usage.quota_bytes {
        return Err(InsertError::Full(usage));
    }
    let row = videos::insert(&mut *tx, video).await?;
    tx.commit().await?;
    Ok(row)
}

/// A recording that is still being made asks for one more part: that part must still fit, counting everything
/// the recording is already known to hold (the parts before it) and everything else the person keeps. Checked as
/// the parts come, so the recorder is told while it still has the recording on its own disk. Several recordings at
/// once can each fit and together overshoot by a part or two: they are not serialised, which a part a second
/// could not afford.
pub async fn check_part(
    state: &AppState,
    user_id: Uuid,
    video_id: Uuid,
    end_offset: i64,
) -> Result<(), ApiError> {
    let usage = repo::usage(&state.db, user_id, default_quota(state), Some(video_id))
        .await?
        .ok_or_else(|| ApiError::unauthorized("unauthenticated", "Sign in to continue."))?;
    if usage.used_bytes.saturating_add(end_offset) > usage.quota_bytes {
        return Err(exceeded(&usage, end_offset));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::human;

    #[test]
    fn sizes_read_as_a_person_would_say_them() {
        assert_eq!(human(0), "0 bytes");
        assert_eq!(human(1023), "1023 bytes");
        assert_eq!(human(1024), "1.0 KB");
        assert_eq!(human(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(human(25 * 1024 * 1024 * 1024), "25.0 GB");
        assert_eq!(human(1536 * 1024 * 1024), "1.5 GB");
        assert_eq!(human(-5), "0 bytes");
    }
}
