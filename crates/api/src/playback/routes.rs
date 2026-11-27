use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::auth::extractor::{ApiJson, AuthUser, Client};
use crate::error::{ApiError, ErrorBody};
use crate::playback::service::{self, HEARTBEAT_SECS};
use crate::ratelimit::{self, policies};
use crate::state::AppState;

/// Mounted at `/v1/play`. Playlist routes are authorised by the token in `?t=` alone: players
/// fetch them without cookies. Heartbeat and stop are the signed-in viewer's own calls.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/:session", delete(stop))
        .route("/:session/master.m3u8", get(master))
        .route("/:session/heartbeat", post(heartbeat))
        .route("/:session/:file", get(media))
}

#[derive(Serialize, ToSchema)]
pub struct WatermarkResponse {
    /// Draw this over the video, moving it around, for the whole viewing.
    pub text: String,
    /// Short code for this viewing, also in the text. Quote it to support.
    pub session_code: String,
}

#[derive(Serialize, ToSchema)]
pub struct PlaybackResponse {
    pub session_id: Uuid,
    /// Relative to the API's origin; hand it to the HLS player as is.
    pub playlist_url: String,
    /// When the viewing runs out; after that the keys stop.
    pub expires_at: DateTime<Utc>,
    /// Send `POST /v1/play/{session}/heartbeat` this often while it plays.
    pub heartbeat_interval_seconds: u64,
    /// Where the viewer left off, to start from.
    pub resume_from_seconds: i32,
    /// Draw this over the video, moving it around, for the whole session.
    pub watermark: WatermarkResponse,
}

#[derive(Serialize, ToSchema)]
pub struct PlaybackEnvelope {
    pub playback: PlaybackResponse,
}

fn started_response(started: service::Started) -> (StatusCode, Json<PlaybackEnvelope>) {
    (
        StatusCode::CREATED,
        Json(PlaybackEnvelope {
            playback: PlaybackResponse {
                session_id: started.session_id,
                playlist_url: format!(
                    "/v1/play/{}/master.m3u8?t={}",
                    started.session_id, started.token
                ),
                expires_at: started.expires_at,
                heartbeat_interval_seconds: HEARTBEAT_SECS,
                resume_from_seconds: started.resume_from_seconds,
                watermark: WatermarkResponse {
                    text: started.watermark.text,
                    session_code: started.watermark.session_code,
                },
            },
        }),
    )
}

/// Starts a viewing of a video: for its owner, an admin, or anyone in a workspace that holds a recording
/// made of it.
///
/// Gives back the playlist address and what the player needs to keep the viewing alive. A student who is
/// enrolled in a course plays through the lesson instead, never through a video's own id. An account may
/// watch on a few screens at once; starting one more is `409 stream_limit`.
#[utoipa::path(
    post,
    path = "/v1/videos/{id}/play",
    operation_id = "videos_play",
    tag = "playback",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "The video.")),
    responses(
        (status = 201, description = "A viewing started.", body = PlaybackEnvelope),
        (status = 401, description = "`unauthenticated`", body = ErrorBody),
        (status = 403, description = "`playback_denied`: not yours, and not in a workspace that holds it; or `forbidden_origin`.", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`not_ready` (still being processed) or `stream_limit` (already watching on as many screens as an account may).", body = ErrorBody),
        (status = 429, description = "`too_many_requests`", body = ErrorBody, headers(("Retry-After" = u64, description = "Seconds until the budget is available again."))),
    )
)]
pub async fn start_video(
    State(state): State<AppState>,
    user: AuthUser,
    Client(ctx): Client,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<PlaybackEnvelope>), ApiError> {
    let id = Uuid::parse_str(&id).map_err(|_| ApiError::Http {
        status: StatusCode::NOT_FOUND,
        code: "not_found",
        message: "Video not found.".into(),
    })?;
    ratelimit::enforce_user(&state, &policies::PLAY_START, user.id).await?;
    Ok(started_response(
        service::start_video(&state, &user, id, &ctx).await?,
    ))
}

/// `POST /v1/lessons/:id/play` — owner/admin, preview, or active enrollment.
pub async fn start_lesson(
    State(state): State<AppState>,
    user: AuthUser,
    Client(ctx): Client,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<PlaybackEnvelope>), ApiError> {
    let id = Uuid::parse_str(&id).map_err(|_| ApiError::Http {
        status: StatusCode::NOT_FOUND,
        code: "not_found",
        message: "Lesson not found.".into(),
    })?;
    ratelimit::enforce_user(&state, &policies::PLAY_START, user.id).await?;
    Ok(started_response(
        service::start_lesson(&state, &user, id, &ctx).await?,
    ))
}

#[derive(Deserialize)]
struct TokenQuery {
    t: Option<String>,
}

fn playlist_parts(session: &str, query: &TokenQuery) -> Result<(Uuid, String), ApiError> {
    let denied = || {
        ApiError::forbidden(
            "playback_denied",
            "This playback session is not valid. Start playback again.",
        )
    };
    let session = Uuid::parse_str(session).map_err(|_| denied())?;
    let token = query
        .t
        .clone()
        .filter(|t| !t.is_empty())
        .ok_or_else(denied)?;
    Ok((session, token))
}

fn playlist_response(body: String) -> Response {
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/vnd.apple.mpegurl"),
            ),
            // Each copy carries one viewer's token and signed URLs: never store or share it.
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("no-store, private"),
            ),
            (
                header::REFERRER_POLICY,
                HeaderValue::from_static("no-referrer"),
            ),
        ],
        body,
    )
        .into_response()
}

async fn master(
    State(state): State<AppState>,
    Path(session): Path<String>,
    Query(query): Query<TokenQuery>,
) -> Result<Response, ApiError> {
    let (session, token) = playlist_parts(&session, &query)?;
    Ok(playlist_response(
        service::master(&state, session, &token).await?,
    ))
}

async fn media(
    State(state): State<AppState>,
    Path((session, file)): Path<(String, String)>,
    Query(query): Query<TokenQuery>,
) -> Result<Response, ApiError> {
    let (session, token) = playlist_parts(&session, &query)?;
    Ok(playlist_response(
        service::media(&state, session, &file, &token).await?,
    ))
}

fn session_id(raw: &str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(raw).map_err(|_| ApiError::Http {
        status: StatusCode::NOT_FOUND,
        code: "not_found",
        message: "Playback session not found.".into(),
    })
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct HeartbeatRequest {
    /// Where the player is now, in whole seconds. Kept for resuming; never past the end of the video.
    position_seconds: Option<u32>,
}

/// Says the viewing is still going, and where it is. Send it every `heartbeat_interval_seconds`.
///
/// This is also where a viewing finds out it is over: the answer is `409 session_ended` when it was closed
/// (the access it rested on has gone, it was started on another screen, or it ran out), and the body says why.
#[utoipa::path(
    post,
    path = "/v1/play/{session}/heartbeat",
    operation_id = "play_heartbeat",
    tag = "playback",
    security(("session" = [])),
    params(("session" = Uuid, Path, description = "The viewing, from `session_id`.")),
    request_body = HeartbeatRequest,
    responses(
        (status = 204, description = "Noted. It is still going."),
        (status = 401, description = "`unauthenticated`", body = ErrorBody),
        (status = 404, description = "`not_found`: not your viewing, or none.", body = ErrorBody),
        (status = 409, description = "`session_ended`: it is over, and the message says why. Start another to watch more.", body = ErrorBody),
        (status = 429, description = "`too_many_requests`", body = ErrorBody, headers(("Retry-After" = u64, description = "Seconds until the budget is available again."))),
    )
)]
pub(crate) async fn heartbeat(
    State(state): State<AppState>,
    user: AuthUser,
    Path(session): Path<String>,
    ApiJson(body): ApiJson<HeartbeatRequest>,
) -> Result<StatusCode, ApiError> {
    ratelimit::enforce_user(&state, &policies::HEARTBEAT, user.id).await?;
    service::heartbeat(&state, &user, session_id(&session)?, body.position_seconds).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Ends the viewing. Safe to call on one that has already ended.
#[utoipa::path(
    delete,
    path = "/v1/play/{session}",
    operation_id = "play_stop",
    tag = "playback",
    security(("session" = [])),
    params(("session" = Uuid, Path, description = "The viewing.")),
    responses(
        (status = 204, description = "Ended."),
        (status = 401, description = "`unauthenticated`", body = ErrorBody),
        (status = 403, description = "`forbidden_origin`", body = ErrorBody),
        (status = 404, description = "`not_found`: not your viewing, or none.", body = ErrorBody),
    )
)]
pub(crate) async fn stop(
    State(state): State<AppState>,
    user: AuthUser,
    Path(session): Path<String>,
) -> Result<StatusCode, ApiError> {
    service::stop(&state, &user, session_id(&session)?).await?;
    Ok(StatusCode::NO_CONTENT)
}
