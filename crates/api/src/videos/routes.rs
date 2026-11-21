use axum::body::Body;
use axum::extract::{FromRequest, Path, Request, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use uuid::Uuid;

use crate::auth::extractor::{ApiJson, AuthUser};
use crate::error::{ApiError, ErrorBody};
use crate::state::AppState;
use crate::videos::dto::{
    CompleteRequest, CreateStreamingRequest, CreateVideoRequest, CreateVideoResponse, PartEnvelope,
    PartRequest, UploadResponse, VideoEnvelope, VideoResponse,
};
use crate::videos::service;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", post(create))
        .route("/streaming", post(create_streaming))
        .route("/:id", get(show).delete(remove))
        .route("/:id/upload", get(resume))
        .route("/:id/parts", post(part))
        .route("/:id/complete", post(complete))
        .route("/:id/abort", post(abort))
        .route("/:id/play", post(crate::playback::routes::start_video))
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

/// Starts the upload of a recording that is still being made.
///
/// There is no size to give. Parts are asked for one at a time as they fill (`POST /v1/videos/{id}/parts`),
/// each URL signed for exactly the length said, and the size is stated at the end
/// (`POST /v1/videos/{id}/complete`). Nothing the client sends reaches this API: the bytes go to storage.
#[utoipa::path(
    post,
    path = "/v1/videos/streaming",
    operation_id = "videos_create_streaming",
    tag = "videos",
    security(("session" = [])),
    request_body = CreateStreamingRequest,
    responses(
        (status = 201, description = "Started. `upload.parts` is empty: ask for each part as it fills.", body = CreateVideoResponse),
        (status = 401, description = "`unauthenticated`", body = ErrorBody),
        (status = 403, description = "`forbidden` (only creator and admin accounts upload) or `forbidden_origin`.", body = ErrorBody),
        (status = 422, description = "`validation_failed` (see `fields`) or `invalid_request`.", body = ErrorBody),
        (status = 429, description = "`too_many_uploads` (ten are unfinished: finish or abort one) or `too_many_requests` (which carries `Retry-After`).", body = ErrorBody),
    )
)]
pub(crate) async fn create_streaming(
    State(state): State<AppState>,
    user: AuthUser,
    ApiJson(body): ApiJson<CreateStreamingRequest>,
) -> Result<(StatusCode, Json<CreateVideoResponse>), ApiError> {
    let input = body.validate()?;
    let (video, upload) = service::create_streaming(&state, &user, input).await?;
    Ok((
        StatusCode::CREATED,
        Json(CreateVideoResponse {
            video: VideoResponse::from(&video),
            upload,
        }),
    ))
}

/// The URL for one more part of a recording's upload, signed for exactly its length.
///
/// Asking again for a part gives a fresh URL: a client that lost the answer, or whose URL expired,
/// just asks. Every part but the last is exactly the part size; the last is 1 up to the part size.
#[utoipa::path(
    post,
    path = "/v1/videos/{id}/parts",
    operation_id = "videos_part",
    tag = "videos",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "The video made by `POST /v1/videos/streaming`.")),
    request_body = PartRequest,
    responses(
        (status = 200, description = "Where to `PUT` the part.", body = PartEnvelope),
        (status = 401, description = "`unauthenticated`", body = ErrorBody),
        (status = 403, description = "`forbidden` or `forbidden_origin`.", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`not_streaming`: this upload is finished, or was started with a size.", body = ErrorBody),
        (status = 422, description = "`validation_failed` (see `fields`: the number or the size) or `invalid_request`.", body = ErrorBody),
        (status = 429, description = "`too_many_requests`", body = ErrorBody, headers(("Retry-After" = u64, description = "Seconds until the budget is available again."))),
    )
)]
pub(crate) async fn part(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<PartRequest>,
) -> Result<Json<PartEnvelope>, ApiError> {
    let (part, expires_at) = service::part_url(
        &state,
        &user,
        parse_id(&id)?,
        body.number,
        body.size,
        body.last,
    )
    .await?;
    Ok(Json(PartEnvelope { part, expires_at }))
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

/// Finishes an upload. A recording's upload says how big the recording came to; a file's has nothing to say
/// (its size was given at the start), so its request has no body.
///
/// Storage is asked what it holds before anything is assembled: if a part is missing the answer is
/// `409 upload_incomplete` and nothing is queued. Completing twice is not an error.
#[utoipa::path(
    post,
    path = "/v1/videos/{id}/complete",
    operation_id = "videos_complete",
    tag = "videos",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "The video.")),
    request_body(content = Option<CompleteRequest>, description = "Required for a recording's upload; not sent for a file's."),
    responses(
        (status = 200, description = "Assembled, and queued for processing (or already was).", body = VideoEnvelope),
        (status = 401, description = "`unauthenticated`", body = ErrorBody),
        (status = 403, description = "`forbidden` or `forbidden_origin`.", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`upload_incomplete` (the message says which parts are missing), `upload_expired`, `not_uploading`, `video_failed` or `size_mismatch`.", body = ErrorBody),
        (status = 422, description = "`validation_failed`: `size_bytes` is missing, zero, over the limit, or not the size a file's upload was started with.", body = ErrorBody),
    )
)]
pub(crate) async fn complete(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    request: Request,
) -> Result<Json<VideoEnvelope>, ApiError> {
    let declared = declared_size(request, &state).await?;
    let video = service::complete(&state, &user, parse_id(&id)?, declared).await?;
    Ok(Json(VideoEnvelope {
        video: VideoResponse::from(&video),
    }))
}

/// The size a request states, if it has a body at all. A file's upload sends none; a recording's sends JSON,
/// and then it is read exactly as every other JSON body is, with the same errors.
async fn declared_size(request: Request, state: &AppState) -> Result<Option<u64>, ApiError> {
    /// As much as any JSON body may be (axum's own default).
    const LIMIT: usize = 2 * 1024 * 1024;
    let (parts, body) = request.into_parts();
    let bytes = axum::body::to_bytes(body, LIMIT)
        .await
        .map_err(|_| ApiError::Http {
            status: StatusCode::PAYLOAD_TOO_LARGE,
            code: "payload_too_large",
            message: "The body is too large.".into(),
        })?;
    if bytes.is_empty() {
        return Ok(None);
    }
    let request = Request::from_parts(parts, Body::from(bytes));
    let ApiJson(CompleteRequest { size_bytes }) =
        ApiJson::<CompleteRequest>::from_request(request, state).await?;
    Ok(Some(size_bytes))
}

async fn abort(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    service::abort(&state, &user, parse_id(&id)?).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    service::delete(&state, &user, parse_id(&id)?).await?;
    Ok(StatusCode::NO_CONTENT)
}
