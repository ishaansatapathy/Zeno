use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, put};
use axum::{Json, Router};
use uuid::Uuid;

use crate::auth::extractor::{ApiJson, AuthUser};
use crate::cursor::TimeCursor;
use crate::error::ApiError;
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

/// `201` when this call made the recording; `200` when it found the one an earlier attempt with
/// the same `id` made.
async fn create(
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

async fn list(
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

async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<RecordingEnvelope>, ApiError> {
    Ok(envelope(service::get(&state, &user, parse_id(&id)?).await?))
}

async fn update(
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

async fn remove(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    service::delete(&state, &user, parse_id(&id)?).await?;
    Ok(StatusCode::NO_CONTENT)
}
