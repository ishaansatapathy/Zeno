use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use chrono::Utc;
use uuid::Uuid;

use crate::auth::extractor::{ApiJson, AuthUser, Client};
use crate::error::{ApiError, ErrorBody};
use crate::net::limit_bucket;
use crate::ratelimit::{self, policies};
use crate::shares::dto::{
    CreateLinkRequest, LinkEnvelope, LinkListEnvelope, LinkResponse, PlayRequest, ShareEnvelope,
    SharedPlayback, SharedPlaybackEnvelope, SharedWatermark,
};
use crate::shares::service;
use crate::state::AppState;

/// A page may be reused for a few seconds and served stale for a little longer while a fresh
/// one is fetched. Short, because the page shows state (processing, ready); the lifetime does
/// not decide who may watch. Every playlist and key request asks the database again, so a link
/// revoked a second ago cannot be watched from a cached page.
const CACHE_CONTROL: &str = "public, max-age=15, stale-while-revalidate=45";

/// The owner's side, merged into `/v1/recordings`.
pub fn recording_router() -> Router<AppState> {
    Router::new()
        .route("/:id/links", post(create).get(list))
        .route("/:id/links/:link", delete(revoke))
}

/// The visitor's side, mounted at `/v1/s`. Nothing here reads cookies or identifies the caller.
pub fn public_router() -> Router<AppState> {
    Router::new()
        .route("/:code", get(resolve))
        .route("/:code/play", post(play))
}

fn not_found(what: &'static str) -> ApiError {
    ApiError::Http {
        status: StatusCode::NOT_FOUND,
        code: "not_found",
        message: format!("{what} not found."),
    }
}

fn parse(raw: &str, what: &'static str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(raw).map_err(|_| not_found(what))
}

/// Makes a share link for a recording. A private recording is switched to `link` in the same
/// call: a link to something nobody may see would be a trap.
#[utoipa::path(
    post,
    path = "/v1/recordings/{id}/links",
    operation_id = "links_create",
    tag = "recordings",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "The recording.")),
    request_body = CreateLinkRequest,
    responses(
        (status = 201, description = "Made. `link.url` is what to hand out.", body = LinkEnvelope),
        (status = 401, description = "`unauthenticated`", body = ErrorBody),
        (status = 403, description = "`forbidden` or `forbidden_origin`.", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`too_many_links`: a recording may have 10 live links; revoke one first.", body = ErrorBody),
        (status = 422, description = "`validation_failed` (the password or the expiry) or `invalid_request`.", body = ErrorBody),
        (status = 429, description = "`too_many_requests`", body = ErrorBody, headers(("Retry-After" = u64, description = "Seconds until the budget is available again."))),
    )
)]
pub(crate) async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<CreateLinkRequest>,
) -> Result<(StatusCode, Json<LinkEnvelope>), ApiError> {
    let now = Utc::now();
    let input = body.validate(now)?;
    let (link, visibility) =
        service::create(&state, &user, parse(&id, "Recording")?, input).await?;
    Ok((
        StatusCode::CREATED,
        Json(LinkEnvelope {
            link: LinkResponse::build(&state.config.client_origin, link, now),
            recording_visibility: visibility,
        }),
    ))
}

/// The recording's links, newest first, whether they still work or not. Never carries a password.
#[utoipa::path(
    get,
    path = "/v1/recordings/{id}/links",
    operation_id = "links_list",
    tag = "recordings",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "The recording.")),
    responses(
        (status = 200, description = "Up to 100 links.", body = LinkListEnvelope),
        (status = 401, description = "`unauthenticated`", body = ErrorBody),
        (status = 403, description = "`forbidden`", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
    )
)]
pub(crate) async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<LinkListEnvelope>, ApiError> {
    let links = service::list(&state, &user, parse(&id, "Recording")?).await?;
    let now = Utc::now();
    Ok(Json(LinkListEnvelope {
        links: links
            .into_iter()
            .map(|link| LinkResponse::build(&state.config.client_origin, link, now))
            .collect(),
    }))
}

/// Revokes a link and ends every viewing under way through it. Safe to repeat.
#[utoipa::path(
    delete,
    path = "/v1/recordings/{id}/links/{link}",
    operation_id = "links_revoke",
    tag = "recordings",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "The recording."), ("link" = Uuid, Path, description = "The link.")),
    responses(
        (status = 204, description = "Revoked."),
        (status = 401, description = "`unauthenticated`", body = ErrorBody),
        (status = 403, description = "`forbidden` or `forbidden_origin`.", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 429, description = "`too_many_requests`", body = ErrorBody, headers(("Retry-After" = u64, description = "Seconds until the budget is available again."))),
    )
)]
pub(crate) async fn revoke(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, link)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    service::revoke(
        &state,
        &user,
        parse(&id, "Recording")?,
        parse(&link, "Link")?,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /v1/s/:code`: what the page needs before anyone presses play.
async fn resolve(
    State(state): State<AppState>,
    request: HeaderMap,
    Path(code): Path<String>,
) -> Result<Response, ApiError> {
    let share = service::resolve(&state, &code).await?;
    Ok(crate::httpcache::json_with_etag(
        &request,
        &ShareEnvelope { share },
        CACHE_CONTROL,
    ))
}

/// `POST /v1/s/:code/play`: start watching. Needs the password if the link has one.
async fn play(
    State(state): State<AppState>,
    Client(ctx): Client,
    Path(code): Path<String>,
    ApiJson(body): ApiJson<PlayRequest>,
) -> Result<(StatusCode, Json<SharedPlaybackEnvelope>), ApiError> {
    ratelimit::enforce(&state, &policies::SHARE_PLAY, &limit_bucket(ctx.ip)).await?;
    let started = service::play(&state, &code, body.password, &ctx).await?;
    Ok((
        StatusCode::CREATED,
        Json(SharedPlaybackEnvelope {
            playback: SharedPlayback {
                session_id: started.session_id,
                playlist_url: format!(
                    "/v1/play/{}/master.m3u8?t={}",
                    started.session_id, started.token
                ),
                expires_at: started.expires_at,
                watermark: SharedWatermark {
                    text: started.watermark.text,
                    session_code: started.watermark.session_code,
                },
            },
        }),
    ))
}
