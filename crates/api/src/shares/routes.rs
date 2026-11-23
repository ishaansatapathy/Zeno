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

/// While the video is still being made, the page is what somebody is waiting on: it is reused for two seconds
/// at most, and never served stale, so the moment the video plays is the moment the page says so. Still cheap
/// for the origin (an edge asks it about a link about once in two seconds, however many are waiting), and
/// only for as long as the video takes to prepare.
const CACHE_CONTROL_WAITING: &str = "public, max-age=2";

/// The owner's side, merged into `/v1/recordings`.
pub fn recording_router() -> Router<AppState> {
    Router::new()
        .route("/:id/links", post(create).get(list))
        .route("/:id/links/:link", delete(revoke))
}

/// The visitor's side, mounted at `/v1/s`. Watching reads no cookies and identifies nobody; the comments
/// merged in below know who is signed in, to show them which are theirs and to let them add one.
pub fn public_router() -> Router<AppState> {
    Router::new()
        .route("/:code", get(resolve))
        .route("/:code/play", post(play))
        .merge(crate::comments::routes::viewer_router())
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
        (status = 201, description = "Made, or returned the same open link for a repeated client `id` and the same details. `link.url` is what to hand out.", body = LinkEnvelope),
        (status = 401, description = "`unauthenticated`", body = ErrorBody),
        (status = 403, description = "`forbidden` or `forbidden_origin`.", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`too_many_links` or `link_id_conflict`: a recording may have 10 live links, and an id cannot be reused for another or closed link.", body = ErrorBody),
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

/// What a visitor's page needs before anyone presses play.
///
/// Public and cacheable: an edge may reuse the answer for 15 seconds and serve it stale for 45
/// more while it fetches a fresh one, so the origin hears about a link about four times a minute
/// however many people open it. A cached answer never lets anyone watch after a revoke: playing
/// asks the database again. While the video is not ready the answer is reused for 2 seconds only (see
/// `CACHE_CONTROL_WAITING`), so a page waiting for it to play finds out at once.
#[utoipa::path(
    get,
    path = "/v1/s/{code}",
    operation_id = "share_resolve",
    tag = "share",
    params(
        ("code" = String, Path, description = "The 22 characters after `/s/` in the link."),
        ("If-None-Match" = Option<String>, Header, description = "The `ETag` of an answer already held."),
    ),
    responses(
        (status = 200, description = "What the page needs.", body = ShareEnvelope, headers(
            ("Cache-Control" = String, description = "`public, max-age=15, stale-while-revalidate=45`; `public, max-age=2` while the video is not ready yet."),
            ("ETag" = String, description = "A weak validator over the body."),
        )),
        (status = 304, description = "The caller already holds this exact answer.", headers(
            ("Cache-Control" = String, description = "As for `200`."),
            ("ETag" = String, description = "The validator the caller sent."),
        )),
        (status = 404, description = "`not_found`: no such link (a code of the wrong shape never reaches the database).", body = ErrorBody),
        (status = 410, description = "`link_unavailable`: the link was good and has closed (revoked, expired, switched off, or its recording deleted).", body = ErrorBody),
        (status = 429, description = "`too_many_requests`", body = ErrorBody, headers(("Retry-After" = u64, description = "Seconds until the budget is available again."))),
    )
)]
pub(crate) async fn resolve(
    State(state): State<AppState>,
    request: HeaderMap,
    Path(code): Path<String>,
) -> Result<Response, ApiError> {
    let share = service::resolve(&state, &code).await?;
    // A locked link shows no status, and nothing is waited for before the password is given.
    let waiting = share.status.is_some_and(|status| status != "ready");
    Ok(crate::httpcache::json_with_etag(
        &request,
        &ShareEnvelope { share },
        if waiting {
            CACHE_CONTROL_WAITING
        } else {
            CACHE_CONTROL
        },
    ))
}

/// Starts watching through a link: checks the password if there is one, then opens a session.
///
/// `playback.playlist_url` is relative to this origin and is handed to the HLS player as it is.
/// There is no heartbeat and no resume point: there is no account to remember either for.
#[utoipa::path(
    post,
    path = "/v1/s/{code}/play",
    operation_id = "share_play",
    tag = "share",
    params(("code" = String, Path, description = "The 22 characters after `/s/` in the link.")),
    request_body = PlayRequest,
    responses(
        (status = 201, description = "A viewing started.", body = SharedPlaybackEnvelope),
        (status = 403, description = "`password_required`, `wrong_password` (a guess costs a password hash, so guesses are limited per link and address), or `forbidden_origin`.", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`not_ready`: the video is still being prepared.", body = ErrorBody),
        (status = 410, description = "`link_unavailable`: the link has closed, including a moment ago.", body = ErrorBody),
        (status = 429, description = "`too_many_requests`: too many viewings started from this address, or too many password guesses.", body = ErrorBody, headers(("Retry-After" = u64, description = "Seconds until the budget is available again."))),
    )
)]
pub(crate) async fn play(
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
