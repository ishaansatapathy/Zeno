use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get};
use axum::{Json, Router};
use uuid::Uuid;

use crate::auth::extractor::{ApiJson, AuthUser, Client, MaybeUser};
use crate::comments::dto::{
    CommentEnvelope, CommentListEnvelope, CreateCommentRequest, ListCommentsQuery, TokenQuery,
};
use crate::comments::service::{self, Page};
use crate::cursor::TimeCursor;
use crate::error::{ApiError, ErrorBody};
use crate::state::AppState;

/// What the people watching through a link see and say, merged into `/v1/s`.
pub fn viewer_router() -> Router<AppState> {
    Router::new().route("/:code/comments", get(list_for_link).post(create))
}

/// The owner's view of their own recording's conversation, merged into `/v1/recordings`.
pub fn recording_router() -> Router<AppState> {
    Router::new().route("/:id/comments", get(list_for_owner))
}

/// Removing one comment, mounted at `/v1/comments`.
pub fn router() -> Router<AppState> {
    Router::new().route("/:id", delete(remove))
}

fn parse(raw: &str, what: &'static str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(raw).map_err(|_| ApiError::Http {
        status: StatusCode::NOT_FOUND,
        code: "not_found",
        message: format!("{what} not found."),
    })
}

fn envelope(page: Page) -> Json<CommentListEnvelope> {
    Json(CommentListEnvelope {
        comments: page.comments,
        next_cursor: page.next_cursor,
        comments_enabled: page.comments_enabled,
    })
}

fn cursor_of(query: &ListCommentsQuery) -> Result<Option<TimeCursor>, ApiError> {
    query.cursor.as_deref().map(TimeCursor::decode).transpose()
}

/// The conversation on a shared recording, newest first, a page at a time.
///
/// Open to anyone who can watch: no account is needed to read. For a link with a password, only a viewing that
/// unlocked it may (pass the `t` of its playlist address). Never cached: it carries what the asker may delete.
#[utoipa::path(
    get,
    path = "/v1/s/{code}/comments",
    operation_id = "share_comments_list",
    tag = "comments",
    params(
        ("code" = String, Path, description = "The 22 characters after `/s/` in the link."),
        ListCommentsQuery,
    ),
    responses(
        (status = 200, description = "One page. With comments turned off it is empty and says so.", body = CommentListEnvelope),
        (status = 400, description = "`invalid_cursor`", body = ErrorBody),
        (status = 403, description = "`password_required`: the link has a password and this is not a viewing that gave it.", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 410, description = "`link_unavailable`: the link has closed.", body = ErrorBody),
        (status = 422, description = "`validation_failed`: `limit` is not from 1 to 200.", body = ErrorBody),
        (status = 429, description = "`too_many_requests`", body = ErrorBody, headers(("Retry-After" = u64, description = "Seconds until the budget is available again."))),
    )
)]
pub(crate) async fn list_for_link(
    State(state): State<AppState>,
    Client(ctx): Client,
    MaybeUser(viewer): MaybeUser,
    Path(code): Path<String>,
    Query(query): Query<ListCommentsQuery>,
) -> Result<Response, ApiError> {
    let limit = query.page_size()?;
    let cursor = cursor_of(&query)?;
    let page = service::list_for_link(
        &state,
        ctx.ip,
        &code,
        viewer.as_ref(),
        query.t.as_deref(),
        cursor,
        limit,
    )
    .await?;
    Ok((
        [(header::CACHE_CONTROL, "private, no-store")],
        envelope(page),
    )
        .into_response())
}

/// Leaves a comment or a reaction on a shared recording.
///
/// Needs a signed-in account (any), and a link that is open to the asker, as for reading. Sending the same
/// `id` and content again returns the comment that was left instead of a second one.
#[utoipa::path(
    post,
    path = "/v1/s/{code}/comments",
    operation_id = "share_comments_create",
    tag = "comments",
    security(("session" = [])),
    params(
        ("code" = String, Path, description = "The 22 characters after `/s/` in the link."),
        TokenQuery,
    ),
    request_body = CreateCommentRequest,
    responses(
        (status = 201, description = "Left (or already left, for a repeated `id`).", body = CommentEnvelope),
        (status = 401, description = "`unauthenticated`", body = ErrorBody),
        (status = 403, description = "`password_required` or `forbidden_origin`.", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`comments_off`, `comments_full`, `too_many_comments` (200 a person, 2000 a video) or `comment_id_conflict`.", body = ErrorBody),
        (status = 410, description = "`link_unavailable`: the link has closed.", body = ErrorBody),
        (status = 422, description = "`validation_failed` (see `fields`) or `invalid_request`.", body = ErrorBody),
        (status = 429, description = "`too_many_requests`", body = ErrorBody, headers(("Retry-After" = u64, description = "Seconds until the budget is available again."))),
    )
)]
pub(crate) async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Path(code): Path<String>,
    Query(query): Query<TokenQuery>,
    ApiJson(body): ApiJson<CreateCommentRequest>,
) -> Result<(StatusCode, Json<CommentEnvelope>), ApiError> {
    let comment = service::create(&state, &user, &code, query.t.as_deref(), body).await?;
    Ok((StatusCode::CREATED, Json(CommentEnvelope { comment })))
}

/// The conversation on one of the caller's own recordings, newest first, whether or not it is open to others.
#[utoipa::path(
    get,
    path = "/v1/recordings/{id}/comments",
    operation_id = "recordings_comments_list",
    tag = "comments",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "The recording."),
        ListCommentsQuery,
    ),
    responses(
        (status = 200, description = "One page. `comments_enabled` says whether others can see and add to it.", body = CommentListEnvelope),
        (status = 400, description = "`invalid_cursor`", body = ErrorBody),
        (status = 401, description = "`unauthenticated`", body = ErrorBody),
        (status = 403, description = "`forbidden`: only creator and admin accounts keep a library.", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 422, description = "`validation_failed`: `limit` is not from 1 to 200.", body = ErrorBody),
        (status = 429, description = "`too_many_requests`", body = ErrorBody, headers(("Retry-After" = u64, description = "Seconds until the budget is available again."))),
    )
)]
pub(crate) async fn list_for_owner(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<ListCommentsQuery>,
) -> Result<Json<CommentListEnvelope>, ApiError> {
    let limit = query.page_size()?;
    let cursor = cursor_of(&query)?;
    let page =
        service::list_for_owner(&state, &user, parse(&id, "Recording")?, cursor, limit).await?;
    Ok(envelope(page))
}

/// Removes a comment or a reaction: its author may, and so may the owner of the recording it is on.
/// For anyone else it does not exist.
#[utoipa::path(
    delete,
    path = "/v1/comments/{id}",
    operation_id = "comments_delete",
    tag = "comments",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "The comment.")),
    responses(
        (status = 204, description = "Gone."),
        (status = 401, description = "`unauthenticated`", body = ErrorBody),
        (status = 403, description = "`forbidden_origin`", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 429, description = "`too_many_requests`", body = ErrorBody, headers(("Retry-After" = u64, description = "Seconds until the budget is available again."))),
    )
)]
pub(crate) async fn remove(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    service::delete(&state, &user, parse(&id, "Comment")?).await?;
    Ok(StatusCode::NO_CONTENT)
}
