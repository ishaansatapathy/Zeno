use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use uuid::Uuid;

use crate::auth::extractor::{ApiJson, AuthUser};
use crate::courses::dto::{
    CourseCursor, CourseEnvelope, CourseListEnvelope, CourseResponse, CreateCourseRequest,
    ListCoursesQuery, UpdateCourseRequest, VersionRequest,
};
use crate::courses::service;
use crate::error::ApiError;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", post(create).get(list))
        .route("/:id", get(show).patch(update).delete(remove))
        .route("/:id/publish", post(publish))
        .route("/:id/unpublish", post(unpublish))
        .route(
            "/:id/thumbnail",
            put(set_thumbnail)
                .delete(clear_thumbnail)
                .layer(crate::images::upload_limit()),
        )
        .merge(crate::courses::lessons::routes::course_router())
        .merge(crate::enrollments::routes::course_router())
}

fn parse_id(raw: &str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(raw).map_err(|_| ApiError::Http {
        status: StatusCode::NOT_FOUND,
        code: "not_found",
        message: "Course not found.".into(),
    })
}

fn header_version(headers: &HeaderMap) -> Result<Option<i32>, ApiError> {
    let Some(raw) = headers.get(header::IF_MATCH) else {
        return Ok(None);
    };
    let raw = raw
        .to_str()
        .map_err(|_| ApiError::bad_request("invalid_version", "If-Match is invalid."))?;
    let value = raw
        .strip_prefix("W/")
        .unwrap_or(raw)
        .trim_matches('"')
        .parse::<i32>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            ApiError::bad_request(
                "invalid_version",
                "If-Match must contain a positive course version.",
            )
        })?;
    Ok(Some(value))
}

fn expected_version(headers: &HeaderMap, body: Option<i32>) -> Result<i32, ApiError> {
    let header = header_version(headers)?;
    if header.is_some() && body.is_some() && header != body {
        return Err(ApiError::bad_request(
            "version_mismatch",
            "The body version and If-Match header disagree.",
        ));
    }
    let version = header.or(body).ok_or_else(|| ApiError::Http {
        status: StatusCode::PRECONDITION_REQUIRED,
        code: "version_required",
        message: "Send the course version in If-Match or the request body.".into(),
    })?;
    if version <= 0 {
        return Err(ApiError::bad_request(
            "invalid_version",
            "The course version must be a positive integer.",
        ));
    }
    Ok(version)
}

async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    ApiJson(body): ApiJson<CreateCourseRequest>,
) -> Result<(StatusCode, Json<CourseEnvelope>), ApiError> {
    let course = service::create(&state, &user, body.validate()?).await?;
    Ok((
        StatusCode::CREATED,
        Json(CourseEnvelope {
            course: CourseResponse::build(&state, &course),
        }),
    ))
}

async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Query(query): Query<ListCoursesQuery>,
) -> Result<Json<CourseListEnvelope>, ApiError> {
    let cursor = query
        .cursor
        .as_deref()
        .map(CourseCursor::decode)
        .transpose()?;
    let (courses, next) = service::list(&state, &user, cursor, query.limit).await?;
    Ok(Json(CourseListEnvelope {
        courses: courses
            .iter()
            .map(|course| CourseResponse::build(&state, course))
            .collect(),
        next_cursor: next.map(CourseCursor::encode),
    }))
}

async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<CourseEnvelope>, ApiError> {
    let course = service::get(&state, &user, parse_id(&id)?).await?;
    Ok(Json(CourseEnvelope {
        course: CourseResponse::build(&state, &course),
    }))
}

async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<UpdateCourseRequest>,
) -> Result<Json<CourseEnvelope>, ApiError> {
    let (body_version, changes) = body.validate()?;
    let version = expected_version(&headers, body_version)?;
    let course = service::update(&state, &user, parse_id(&id)?, version, changes).await?;
    Ok(Json(CourseEnvelope {
        course: CourseResponse::build(&state, &course),
    }))
}

async fn publish(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<VersionRequest>,
) -> Result<Json<CourseEnvelope>, ApiError> {
    let version = expected_version(&headers, Some(body.version))?;
    let course = service::publish(&state, &user, parse_id(&id)?, version).await?;
    Ok(Json(CourseEnvelope {
        course: CourseResponse::build(&state, &course),
    }))
}

async fn unpublish(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<VersionRequest>,
) -> Result<Json<CourseEnvelope>, ApiError> {
    let version = expected_version(&headers, Some(body.version))?;
    let course = service::unpublish(&state, &user, parse_id(&id)?, version).await?;
    Ok(Json(CourseEnvelope {
        course: CourseResponse::build(&state, &course),
    }))
}

async fn remove(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    service::delete(&state, &user, parse_id(&id)?).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_thumbnail(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<Json<CourseEnvelope>, ApiError> {
    let id = parse_id(&id)?;
    let version = header_version(&headers)?;
    let bytes = crate::images::read_upload(&headers, body)?;
    let course = service::set_thumbnail(&state, &user, id, bytes, version).await?;
    Ok(Json(CourseEnvelope {
        course: CourseResponse::build(&state, &course),
    }))
}

async fn clear_thumbnail(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<CourseEnvelope>, ApiError> {
    let version = header_version(&headers)?;
    let course = service::clear_thumbnail(&state, &user, parse_id(&id)?, version).await?;
    Ok(Json(CourseEnvelope {
        course: CourseResponse::build(&state, &course),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_headers_are_strict() {
        let mut headers = HeaderMap::new();
        assert!(expected_version(&headers, None).is_err());
        headers.insert(header::IF_MATCH, "W/\"7\"".parse().unwrap());
        assert_eq!(expected_version(&headers, None).unwrap(), 7);
        assert!(expected_version(&headers, Some(8)).is_err());
    }
}
