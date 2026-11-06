use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use uuid::Uuid;

use crate::auth::extractor::{ApiJson, AuthUser};
use crate::error::ApiError;
use crate::state::AppState;
use crate::videos::dto::{
    CreateVideoRequest, CreateVideoResponse, UploadResponse, VideoEnvelope, VideoResponse,
};
use crate::videos::service;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", post(create))
        .route("/:id", get(show))
        .route("/:id/upload", get(resume))
        .route("/:id/complete", post(complete))
        .route("/:id/abort", post(abort))
}

/// A malformed id is a 404 in the API's own format, not axum's plain-text 400.
fn parse_id(raw: &str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(raw).map_err(|_| ApiError::Http {
        status: StatusCode::NOT_FOUND,
        code: "not_found",
        message: "Video not found.".into(),
    })
}

async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    ApiJson(body): ApiJson<CreateVideoRequest>,
) -> Result<(StatusCode, Json<CreateVideoResponse>), ApiError> {
    let input = body.validate(state.config.max_upload_bytes)?;
    let (video, upload) = service::create(&state, &user, input).await?;
    Ok((
        StatusCode::CREATED,
        Json(CreateVideoResponse {
            video: VideoResponse::from(&video),
            upload,
        }),
    ))
}

async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<VideoEnvelope>, ApiError> {
    let video = service::get(&state, &user, parse_id(&id)?).await?;
    Ok(Json(VideoEnvelope {
        video: VideoResponse::from(&video),
    }))
}

async fn resume(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<UploadResponse>, ApiError> {
    let (_, upload) = service::resume(&state, &user, parse_id(&id)?).await?;
    Ok(Json(upload))
}

async fn complete(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<VideoEnvelope>, ApiError> {
    let video = service::complete(&state, &user, parse_id(&id)?).await?;
    Ok(Json(VideoEnvelope {
        video: VideoResponse::from(&video),
    }))
}

async fn abort(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    service::abort(&state, &user, parse_id(&id)?).await?;
    Ok(StatusCode::NO_CONTENT)
}
