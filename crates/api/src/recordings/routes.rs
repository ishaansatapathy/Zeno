use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, put};
use axum::{Json, Router};
use uuid::Uuid;

use crate::auth::extractor::{ApiJson, AuthUser};
use crate::cursor::TimeCursor;
use crate::error::{ApiError, ErrorBody};
use crate::etag::expected_version;
use crate::recordings::dto::{
    AttachVideoRequest, CreateRecordingRequest, ListRecordingsQuery, RecordingEnvelope,
    RecordingListEnvelope, RecordingResponse, UpdateRecordingRequest,
};
use crate::recordings::service;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/:id", get(show).patch(update).delete(remove))
        .route("/:id/video", put(attach_video))
        .merge(crate::shares::routes::recording_router())
        .merge(crate::views::routes::recording_router())
}

/// An id that is not a UUID names nothing: it is "not found", like any other missing recording.
fn parse_id(raw: &str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(raw).map_err(|_| ApiError::Http {
        status: StatusCode::NOT_FOUND,
        code: "not_found",
        message: "Recording not found.".into(),
    })
}

fn envelope(row: crate::recordings::repo::RecordingRow) -> Json<RecordingEnvelope> {
    Json(RecordingEnvelope {
        recording: RecordingResponse::from(row),
    })
}

/// Makes a recording in the caller's library.
///
/// `201` when this call made it; `200` when it found the one an earlier attempt with the same `id`
/// made, so a retry after a dropped connection never makes a duplicate.
#[utoipa::path(
    post,
    path = "/v1/recordings",
    operation_id = "recordings_create",
    tag = "recordings",
    security(("session" = [])),
    request_body = CreateRecordingRequest,
    responses(
        (status = 201, description = "Made.", body = RecordingEnvelope),
        (status = 200, description = "Already made by an earlier attempt with this `id`.", body = RecordingEnvelope),
        (status = 401, description = "`unauthenticated`", body = ErrorBody),
        (status = 403, description = "`forbidden` (only creator and admin accounts keep a library) or `forbidden_origin`.", body = ErrorBody),
        (status = 404, description = "`video_not_found`: `video_id` is not one of your uploads.", body = ErrorBody),
        (status = 409, description = "`id_conflict` (the id is taken by something else) or `video_in_use` (that video already belongs to another recording).", body = ErrorBody),
        (status = 422, description = "`validation_failed` (see `fields`) or `invalid_request`.", body = ErrorBody),
        (status = 429, description = "`too_many_requests`", body = ErrorBody, headers(("Retry-After" = u64, description = "Seconds until the budget is available again."))),
    )
)]
pub(crate) async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    ApiJson(body): ApiJson<CreateRecordingRequest>,
) -> Result<(StatusCode, Json<RecordingEnvelope>), ApiError> {
    let (row, created) = service::create(&state, &user, body.validate()?).await?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, envelope(row)))
}

/// The caller's library, newest first, a page at a time.
#[utoipa::path(
    get,
    path = "/v1/recordings",
    operation_id = "recordings_list",
    tag = "recordings",
    security(("session" = [])),
    params(ListRecordingsQuery),
    responses(
        (status = 200, description = "One page.", body = RecordingListEnvelope),
        (status = 400, description = "`invalid_cursor`", body = ErrorBody),
        (status = 401, description = "`unauthenticated`", body = ErrorBody),
        (status = 403, description = "`forbidden`: only creator and admin accounts keep a library.", body = ErrorBody),
        (status = 422, description = "`validation_failed`: `source` is not one the API knows.", body = ErrorBody),
    )
)]
pub(crate) async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Query(query): Query<ListRecordingsQuery>,
) -> Result<Json<RecordingListEnvelope>, ApiError> {
    let source = query.source()?;
    let cursor = query
        .cursor
        .as_deref()
        .map(TimeCursor::decode)
        .transpose()?;
    let (rows, next) = service::list(&state, &user, source, cursor, query.limit).await?;
    Ok(Json(RecordingListEnvelope {
        recordings: rows.into_iter().map(RecordingResponse::from).collect(),
        next_cursor: next.map(TimeCursor::encode),
    }))
}

/// One recording. Someone else's, a deleted one and a malformed id all answer `404`.
#[utoipa::path(
    get,
    path = "/v1/recordings/{id}",
    operation_id = "recordings_get",
    tag = "recordings",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "The recording.")),
    responses(
        (status = 200, description = "The recording.", body = RecordingEnvelope),
        (status = 401, description = "`unauthenticated`", body = ErrorBody),
        (status = 403, description = "`forbidden`: only creator and admin accounts keep a library.", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
    )
)]
pub(crate) async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<RecordingEnvelope>, ApiError> {
    Ok(envelope(service::get(&state, &user, parse_id(&id)?).await?))
}

/// Renames a recording or changes who may watch it.
///
/// Optimistic: say which `version` you read, in the body or in `If-Match`. If it has moved on, the
/// answer is `409 version_conflict` and nothing changes.
#[utoipa::path(
    patch,
    path = "/v1/recordings/{id}",
    operation_id = "recordings_update",
    tag = "recordings",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "The recording."), ("If-Match" = Option<String>, Header, description = "The version you read, instead of `version` in the body.")),
    request_body = UpdateRecordingRequest,
    responses(
        (status = 200, description = "Changed.", body = RecordingEnvelope),
        (status = 400, description = "`invalid_version`, `version_mismatch` or `invalid_request`.", body = ErrorBody),
        (status = 401, description = "`unauthenticated`", body = ErrorBody),
        (status = 403, description = "`forbidden` or `forbidden_origin`.", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`version_conflict`: it changed since you read it. Reload and try again.", body = ErrorBody),
        (status = 422, description = "`validation_failed` (see `fields`) or `invalid_request`.", body = ErrorBody),
        (status = 428, description = "`version_required`: say which version you read.", body = ErrorBody),
    )
)]
pub(crate) async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<UpdateRecordingRequest>,
) -> Result<Json<RecordingEnvelope>, ApiError> {
    let (body_version, changes) = body.validate()?;
    let version = expected_version(&headers, body_version, "recording")?;
    let row = service::update(&state, &user, parse_id(&id)?, version, changes).await?;
    Ok(envelope(row))
}

async fn attach_video(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<AttachVideoRequest>,
) -> Result<Json<RecordingEnvelope>, ApiError> {
    let row = service::attach_video(&state, &user, parse_id(&id)?, body.video_id).await?;
    Ok(envelope(row))
}

/// Deletes a recording, and its video unless a course still uses it. Every link to it stops
/// working at once.
#[utoipa::path(
    delete,
    path = "/v1/recordings/{id}",
    operation_id = "recordings_delete",
    tag = "recordings",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "The recording.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "`unauthenticated`", body = ErrorBody),
        (status = 403, description = "`forbidden` or `forbidden_origin`.", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
    )
)]
pub(crate) async fn remove(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    service::delete(&state, &user, parse_id(&id)?).await?;
    Ok(StatusCode::NO_CONTENT)
}
