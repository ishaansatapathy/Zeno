use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use uuid::Uuid;

use crate::auth::extractor::{ApiJson, AuthUser};
use crate::enrollments::dto::{
    EnrollmentCursor, EnrollmentEnvelope, EnrollmentResponse, GrantRequest, GrantResponse,
    ListQuery, MyCourseResponse, MyCoursesEnvelope, RevokeQuery, StudentResponse, StudentsEnvelope,
};
use crate::enrollments::service::{self, GrantResult};
use crate::error::ApiError;
use crate::state::AppState;

pub fn course_router() -> Router<AppState> {
    Router::new()
        .route("/:id/enroll", post(enroll))
        .route("/:id/grants", post(grant))
        .route("/:id/grants/:user", delete(revoke))
        .route("/:id/students", get(students))
}

pub fn me_router() -> Router<AppState> {
    Router::new().route("/courses", get(mine))
}

fn course_id(raw: &str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(raw).map_err(|_| ApiError::Http {
        status: StatusCode::NOT_FOUND,
        code: "not_found",
        message: "Course not found.".into(),
    })
}

async fn enroll(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<EnrollmentEnvelope>), ApiError> {
    crate::ratelimit::enforce_user(&state, &crate::ratelimit::policies::ENROLL, user.id).await?;
    let enrollment = service::enroll_free(&state, &user, course_id(&id)?).await?;
    Ok((
        StatusCode::CREATED,
        Json(EnrollmentEnvelope {
            enrollment: EnrollmentResponse::from(&enrollment),
        }),
    ))
}

async fn grant(
    State(state): State<AppState>,
    actor: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<GrantRequest>,
) -> Result<(StatusCode, Json<GrantResponse>), ApiError> {
    match service::grant(&state, &actor, course_id(&id)?, body.validate()?).await? {
        GrantResult::Enrolled(enrollment) => Ok((
            StatusCode::CREATED,
            Json(GrantResponse {
                status: "enrolled",
                enrollment: Some(EnrollmentResponse::from(&enrollment)),
                invited_email: None,
            }),
        )),
        GrantResult::Invited(email) => Ok((
            StatusCode::ACCEPTED,
            Json(GrantResponse {
                status: "invited",
                enrollment: None,
                invited_email: Some(email),
            }),
        )),
    }
}

async fn revoke(
    State(state): State<AppState>,
    actor: AuthUser,
    Path((id, user)): Path<(String, String)>,
    Query(query): Query<RevokeQuery>,
) -> Result<StatusCode, ApiError> {
    service::revoke(&state, &actor, course_id(&id)?, &user, &query.validate()?).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn mine(
    State(state): State<AppState>,
    user: AuthUser,
    Query(query): Query<ListQuery>,
) -> Result<Json<MyCoursesEnvelope>, ApiError> {
    let cursor = query
        .cursor
        .as_deref()
        .map(EnrollmentCursor::decode)
        .transpose()?;
    let (courses, next) = service::list_mine(&state, &user, cursor, query.limit).await?;
    Ok(Json(MyCoursesEnvelope {
        courses: courses.iter().map(MyCourseResponse::from).collect(),
        next_cursor: next.map(EnrollmentCursor::encode),
    }))
}

async fn students(
    State(state): State<AppState>,
    actor: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Json<StudentsEnvelope>, ApiError> {
    let cursor = query
        .cursor
        .as_deref()
        .map(EnrollmentCursor::decode)
        .transpose()?;
    let (students, next) =
        service::list_students(&state, &actor, course_id(&id)?, cursor, query.limit).await?;
    Ok(Json(StudentsEnvelope {
        students: students.iter().map(StudentResponse::from).collect(),
        next_cursor: next.map(EnrollmentCursor::encode),
    }))
}
