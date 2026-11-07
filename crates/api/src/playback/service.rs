//! Playback sessions: who may press play, how many screens one account may use at once, and
//! the per-viewer playlists that hand out short-lived segment URLs and the key server's URL.

use std::time::Duration as StdDuration;

use axum::http::StatusCode;
use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;
use zeno_core::model::{Role, VideoStatus};
use zeno_core::playback::PlaybackClaims;
use zeno_core::storage::StorageError;

use crate::auth::extractor::AuthUser;
use crate::auth::service::ClientContext;
use crate::error::ApiError;
use crate::playback::playlist::{self, RewriteError, STORED_PLAYLIST_NAME};
use crate::playback::repo::{self, NewSession};
use crate::state::AppState;
use crate::videos::repo::{self as videos, VideoRow};

/// How often the player should report in.
pub const HEARTBEAT_SECS: u64 = 30;
/// A session not heard from for this long stops holding a slot: the next play on another device
/// ends it. Generous next to the heartbeat because background tabs throttle timers.
pub const IDLE_AFTER_SECS: i64 = 180;
/// Ended sessions are kept this long (resume positions, abuse investigations), then deleted.
pub const KEEP_ENDED_DAYS: i32 = 90;
const MIN_SESSION: Duration = Duration::hours(2);
const MAX_SESSION: Duration = Duration::hours(12);
const MAX_PLAYLIST_BYTES: u64 = 4 * 1024 * 1024;
const MAX_DEVICE_LEN: usize = 200;
/// Resuming within this many seconds of the end starts over instead.
const FINISHED_MARGIN_SECS: i32 = 15;

fn not_found() -> ApiError {
    ApiError::Http {
        status: StatusCode::NOT_FOUND,
        code: "not_found",
        message: "Video not found.".into(),
    }
}

/// One answer for every reason a playlist request is refused, so a token cannot be probed for
/// which check it fails.
fn denied() -> ApiError {
    ApiError::forbidden(
        "playback_denied",
        "This playback session is not valid. Start playback again.",
    )
}

fn conflict(code: &'static str, message: impl Into<String>) -> ApiError {
    ApiError::Http {
        status: StatusCode::CONFLICT,
        code,
        message: message.into(),
    }
}

/// Who may watch a video. For now its creator and admins (to preview); students gain access
/// through course enrollment once courses exist.
fn can_watch(user: &AuthUser, video: &VideoRow) -> bool {
    video.owner_id == user.id || user.role == Role::Admin
}

/// Long enough to watch the whole video with pauses, never shorter than two hours nor longer
/// than twelve.
fn session_length(duration_seconds: Option<i32>) -> Duration {
    let duration = Duration::seconds(i64::from(duration_seconds.unwrap_or(0).max(0)));
    (duration * 2 + Duration::hours(1)).clamp(MIN_SESSION, MAX_SESSION)
}

fn resume_point(position: Option<i32>, duration: Option<i32>) -> i32 {
    match (position, duration) {
        (Some(position), Some(duration)) if position < duration - FINISHED_MARGIN_SECS => {
            position.max(0)
        }
        (Some(position), None) => position.max(0),
        _ => 0,
    }
}

fn truncate(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

pub struct Watermark {
    pub text: String,
    pub session_code: String,
}

fn watermark(viewer: &repo::Viewer, session_id: Uuid) -> Watermark {
    let contact = viewer
        .phone
        .as_deref()
        .or(viewer.email.as_deref())
        .unwrap_or_default();
    let session_code = session_id.simple().to_string()[..8].to_uppercase();
    let text = [viewer.name.as_str(), contact, session_code.as_str()]
        .iter()
        .filter(|part| !part.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" · ");
    Watermark { text, session_code }
}

pub struct Started {
    pub session_id: Uuid,
    pub token: String,
    pub expires_at: DateTime<Utc>,
    pub resume_from_seconds: i32,
    pub watermark: Watermark,
}

pub async fn start(
    state: &AppState,
    user: &AuthUser,
    video_id: Uuid,
    ctx: &ClientContext,
) -> Result<Started, ApiError> {
    let video = videos::find(&state.db, video_id)
        .await?
        .filter(|video| can_watch(user, video))
        .ok_or_else(not_found)?;
    if video.status != VideoStatus::Ready {
        return Err(conflict(
            "not_ready",
            "This video is not ready to play yet.",
        ));
    }
    let viewer = repo::viewer(&state.db, user.id)
        .await?
        .ok_or_else(|| ApiError::unauthorized("unauthenticated", "Sign in to continue."))?;

    let session_id = Uuid::new_v4();
    let now = Utc::now();
    // Whole seconds, which is what the token carries.
    let expires_at = DateTime::from_timestamp(
        (now + session_length(video.duration_seconds)).timestamp(),
        0,
    )
    .unwrap_or(now + MIN_SESSION);
    let limit = i64::from(state.config.max_concurrent_streams);

    let mut tx = state.db.begin().await?;
    repo::lock_user(&mut tx, user.id).await?;
    repo::end_stale(&mut tx, user.id, IDLE_AFTER_SECS).await?;
    if repo::count_open(&mut tx, user.id).await? >= limit {
        // Keep the stale sessions ended even though this start is refused.
        tx.commit().await?;
        return Err(conflict(
            "stream_limit",
            format!(
                "You are already watching on {limit} screen{}. Stop playback on one of them to watch here.",
                if limit == 1 { "" } else { "s" }
            ),
        ));
    }
    let position = resume_point(
        repo::last_position(&mut tx, user.id, video_id).await?,
        video.duration_seconds,
    );
    repo::insert(
        &mut tx,
        &NewSession {
            id: session_id,
            user_id: user.id,
            video_id,
            device: &truncate(
                ctx.user_agent.as_deref().unwrap_or_default(),
                MAX_DEVICE_LEN,
            ),
            ip: ctx.ip,
            expires_at,
            position,
        },
    )
    .await?;
    tx.commit().await?;

    let token = state.config.playback.sign(&PlaybackClaims {
        session_id,
        video_id,
        user_id: user.id,
        expires_at,
    });
    Ok(Started {
        session_id,
        token,
        expires_at,
        resume_from_seconds: position,
        watermark: watermark(&viewer, session_id),
    })
}

/// A playlist request that passed every check.
struct Authorized {
    video_id: Uuid,
    expires_at: DateTime<Utc>,
    renditions: Vec<String>,
}

async fn authorize(
    state: &AppState,
    session_id: Uuid,
    token: &str,
) -> Result<Authorized, ApiError> {
    let claims = state
        .config
        .playback
        .verify(token, Utc::now())
        .map_err(|err| {
            tracing::debug!(%err, %session_id, "playback token refused");
            denied()
        })?;
    if claims.session_id != session_id {
        return Err(denied());
    }
    let session = repo::touch_playable(&state.db, session_id)
        .await?
        .ok_or_else(denied)?;
    if session.user_id != claims.user_id || session.video_id != claims.video_id {
        return Err(denied());
    }
    let renditions = session
        .renditions
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|r| r.get("name").and_then(|n| n.as_str()).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    Ok(Authorized {
        video_id: session.video_id,
        expires_at: session.expires_at,
        renditions,
    })
}

async fn stored_playlist(state: &AppState, key: &str) -> Result<String, ApiError> {
    let bytes = state
        .store
        .get_bytes(key, MAX_PLAYLIST_BYTES)
        .await
        .map_err(|err| match err {
            StorageError::Transport(_)
            | StorageError::Http {
                status: 500..=599, ..
            } => {
                tracing::error!(error = ?err, key, "storage unavailable");
                ApiError::Unavailable("storage unavailable")
            }
            other => ApiError::Internal(anyhow::anyhow!("reading {key}: {other}")),
        })?;
    String::from_utf8(bytes).map_err(|_| ApiError::Internal(anyhow::anyhow!("{key} is not UTF-8")))
}

fn rewrite_failed(err: RewriteError, video_id: Uuid) -> ApiError {
    // Only reachable if what is in storage is not what the worker wrote.
    tracing::error!(%err, %video_id, "refusing to serve a stored playlist");
    ApiError::Internal(anyhow::anyhow!("playlist rewrite refused: {err}"))
}

pub async fn master(state: &AppState, session_id: Uuid, token: &str) -> Result<String, ApiError> {
    let auth = authorize(state, session_id, token).await?;
    let text = stored_playlist(state, &format!("videos/{}/master.m3u8", auth.video_id)).await?;
    playlist::rewrite_master(&text, &auth.renditions, token)
        .map_err(|err| rewrite_failed(err, auth.video_id))
}

/// `file` is the last path segment, `{rendition}.m3u8`.
pub async fn media(
    state: &AppState,
    session_id: Uuid,
    file: &str,
    token: &str,
) -> Result<String, ApiError> {
    let auth = authorize(state, session_id, token).await?;
    let rendition = file
        .strip_suffix(".m3u8")
        .filter(|name| auth.renditions.iter().any(|r| r == name))
        .ok_or_else(not_found)?;
    let prefix = format!("videos/{}/{rendition}", auth.video_id);
    let text = stored_playlist(state, &format!("{prefix}/{STORED_PLAYLIST_NAME}")).await?;

    // Segment URLs live as long as the session (a player loads a VOD playlist once and fetches
    // segments for the whole watch), with a little slack for clock skew.
    let remaining = (auth.expires_at - Utc::now() + Duration::minutes(5))
        .to_std()
        .unwrap_or(StdDuration::from_secs(60))
        .max(StdDuration::from_secs(60));
    let key_url = format!(
        "{}/v1/keys/{}?t={token}",
        state.config.key_server_url, auth.video_id
    );
    playlist::rewrite_media(&text, &key_url, |segment| {
        state
            .store
            .presign_get(&format!("{prefix}/{segment}"), remaining)
            .to_string()
    })
    .map_err(|err| rewrite_failed(err, auth.video_id))
}

/// Errors for a heartbeat or stop on a session that is not open.
async fn session_problem(state: &AppState, user: &AuthUser, session_id: Uuid) -> ApiError {
    match repo::state_for_user(&state.db, session_id, user.id).await {
        Ok(Some(session)) if session.ended || session.expired => ApiError::Http {
            status: StatusCode::CONFLICT,
            code: "session_ended",
            message: match session.end_reason.as_deref() {
                Some("idle") => {
                    "Playback was started on another screen. Press play to continue here."
                }
                Some("password_reset") => "Your password was changed. Press play to continue.",
                Some("video_deleted") => "This video is no longer available.",
                _ => "This playback session has ended. Press play to continue.",
            }
            .into(),
        },
        Ok(_) => ApiError::Http {
            status: StatusCode::NOT_FOUND,
            code: "not_found",
            message: "Playback session not found.".into(),
        },
        Err(err) => err.into(),
    }
}

pub async fn heartbeat(
    state: &AppState,
    user: &AuthUser,
    session_id: Uuid,
    position_seconds: Option<u32>,
) -> Result<(), ApiError> {
    let duration = repo::state_for_user(&state.db, session_id, user.id)
        .await?
        .and_then(|session| session.duration_seconds);
    // Never record a position past the end of the video.
    let position = position_seconds.map(|p| {
        let p = i32::try_from(p).unwrap_or(i32::MAX);
        duration.map_or(p, |d| p.min(d.max(0)))
    });
    if repo::heartbeat(&state.db, session_id, user.id, position).await? {
        Ok(())
    } else {
        Err(session_problem(state, user, session_id).await)
    }
}

pub async fn stop(state: &AppState, user: &AuthUser, session_id: Uuid) -> Result<(), ApiError> {
    if repo::state_for_user(&state.db, session_id, user.id)
        .await?
        .is_none()
    {
        return Err(session_problem(state, user, session_id).await);
    }
    repo::end(&state.db, session_id, user.id, "stopped").await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_cover_the_video_twice_within_two_to_twelve_hours() {
        assert_eq!(session_length(None), MIN_SESSION);
        assert_eq!(session_length(Some(60)), MIN_SESSION);
        assert_eq!(session_length(Some(3 * 3600)), Duration::hours(7));
        assert_eq!(session_length(Some(10 * 3600)), MAX_SESSION);
        assert_eq!(session_length(Some(-5)), MIN_SESSION);
    }

    #[test]
    fn resume_starts_over_near_the_end_and_never_goes_negative() {
        assert_eq!(resume_point(None, Some(600)), 0);
        assert_eq!(resume_point(Some(42), Some(600)), 42);
        assert_eq!(resume_point(Some(590), Some(600)), 0);
        assert_eq!(resume_point(Some(600), Some(600)), 0);
        assert_eq!(resume_point(Some(-3), Some(600)), 0);
        assert_eq!(resume_point(Some(42), None), 42);
    }

    #[test]
    fn the_watermark_names_the_viewer_and_the_session() {
        let id = Uuid::parse_str("0123abcd-0000-0000-0000-000000000000").unwrap();
        let mut viewer = repo::Viewer {
            name: "Asha Rao".into(),
            email: Some("asha@x.in".into()),
            phone: Some("+91 98765 43210".into()),
        };
        let mark = watermark(&viewer, id);
        assert_eq!(mark.text, "Asha Rao · +91 98765 43210 · 0123ABCD");
        assert_eq!(mark.session_code, "0123ABCD");
        viewer.phone = None;
        assert_eq!(
            watermark(&viewer, id).text,
            "Asha Rao · asha@x.in · 0123ABCD"
        );
        viewer.email = None;
        assert_eq!(watermark(&viewer, id).text, "Asha Rao · 0123ABCD");
    }

    #[test]
    fn device_names_are_truncated_on_character_boundaries() {
        assert_eq!(truncate("ab✓cd", 3), "ab✓");
    }
}
