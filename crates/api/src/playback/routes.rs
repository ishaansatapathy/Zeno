use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::extractor::{ApiJson, AuthUser, Client};
use crate::error::ApiError;
use crate::playback::service::{self, HEARTBEAT_SECS};
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

#[derive(Serialize)]
pub struct WatermarkResponse {
    pub text: String,
    pub session_code: String,
}

#[derive(Serialize)]
pub struct PlaybackResponse {
    pub session_id: Uuid,
    /// Relative to the API's origin; hand it to the HLS player as is.
    pub playlist_url: String,
    pub expires_at: DateTime<Utc>,
    pub heartbeat_interval_seconds: u64,
    pub resume_from_seconds: i32,
    /// Draw this over the video, moving it around, for the whole session.
    pub watermark: WatermarkResponse,
}

#[derive(Serialize)]
pub struct PlaybackEnvelope {
    pub playback: PlaybackResponse,
}

/// `POST /v1/videos/:id/play`
pub async fn start(
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
    let started = service::start(&state, &user, id, &ctx).await?;
    Ok((
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HeartbeatRequest {
    position_seconds: Option<u32>,
}

async fn heartbeat(
    State(state): State<AppState>,
    user: AuthUser,
    Path(session): Path<String>,
    ApiJson(body): ApiJson<HeartbeatRequest>,
) -> Result<StatusCode, ApiError> {
    service::heartbeat(&state, &user, session_id(&session)?, body.position_seconds).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn stop(
    State(state): State<AppState>,
    user: AuthUser,
    Path(session): Path<String>,
) -> Result<StatusCode, ApiError> {
    service::stop(&state, &user, session_id(&session)?).await?;
    Ok(StatusCode::NO_CONTENT)
}
