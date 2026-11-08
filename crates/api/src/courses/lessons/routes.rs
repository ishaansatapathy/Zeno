use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, put};
use axum::{Json, Router};
use uuid::Uuid;

use crate::auth::extractor::{ApiJson, AuthUser};
use crate::courses::lessons::dto::{
    CreateLessonRequest, LessonEnvelope, LessonListEnvelope, LessonOrderEnvelope, LessonResponse,
    ReorderLessonsRequest, UpdateLessonRequest,
};
use crate::courses::lessons::service;
use crate::error::ApiError;
use crate::state::AppState;

pub fn course_router() -> Router<AppState> {
    Router::new()
        .route("/:id/lessons", get(list).post(create))
        .route("/:id/lessons/order", put(reorder))
}

pub fn item_router() -> Router<AppState> {
    Router::new().route("/:id", axum::routing::patch(update).delete(remove))
}

fn parse_id(raw: &str, subject: &'static str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(raw).map_err(|_| ApiError::Http {
        status: StatusCode::NOT_FOUND,
        code: "not_found",
        message: format!("{subject} not found."),
    })
}

async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<CreateLessonRequest>,
) -> Result<(StatusCode, Json<LessonEnvelope>), ApiError> {
    let (lesson, course_version) =
        service::create(&state, &user, parse_id(&id, "Course")?, body.validate()?).await?;
    Ok((
        StatusCode::CREATED,
        Json(LessonEnvelope {
            lesson: LessonResponse::from(&lesson),
            course_version,
        }),
    ))
}

async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<LessonListEnvelope>, ApiError> {
    let (lessons, course_version) = service::list(&state, &user, parse_id(&id, "Course")?).await?;
    let total_duration_seconds = lessons
        .iter()
        .filter_map(|lesson| lesson.duration_seconds)
        .map(i64::from)
        .sum();
    Ok(Json(LessonListEnvelope {
        lessons: lessons.iter().map(LessonResponse::from).collect(),
        total_duration_seconds,
        course_version,
    }))
}

async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<UpdateLessonRequest>,
) -> Result<Json<LessonEnvelope>, ApiError> {
    let (lesson, course_version) =
        service::update(&state, &user, parse_id(&id, "Lesson")?, body.validate()?).await?;
    Ok(Json(LessonEnvelope {
        lesson: LessonResponse::from(&lesson),
        course_version,
    }))
}

async fn remove(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    service::delete(&state, &user, parse_id(&id, "Lesson")?).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn reorder(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<ReorderLessonsRequest>,
) -> Result<Json<LessonOrderEnvelope>, ApiError> {
    let (lessons, course_version) =
        service::reorder(&state, &user, parse_id(&id, "Course")?, body.lesson_ids).await?;
    Ok(Json(LessonOrderEnvelope {
        lessons: lessons.iter().map(LessonResponse::from).collect(),
        course_version,
    }))
}
