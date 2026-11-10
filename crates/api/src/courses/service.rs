use axum::http::StatusCode;
use uuid::Uuid;
use zeno_core::model::{CourseStatus, Role};

use crate::auth::extractor::AuthUser;
use crate::courses::dto::{slug_base, CourseChanges, CourseCursor, NewCourse};
use crate::courses::repo::{self, CourseRow, CourseValues, NewCourseRow};
use crate::error::ApiError;
use crate::state::AppState;

const COURSE_ROLES: &[Role] = &[Role::Creator, Role::Admin];

fn not_found() -> ApiError {
    ApiError::Http {
        status: StatusCode::NOT_FOUND,
        code: "not_found",
        message: "Course not found.".into(),
    }
}

fn version_conflict() -> ApiError {
    ApiError::Http {
        status: StatusCode::CONFLICT,
        code: "version_conflict",
        message: "This course changed elsewhere. Reload it and try again.".into(),
    }
}

fn conflict(code: &'static str, message: impl Into<String>) -> ApiError {
    ApiError::Http {
        status: StatusCode::CONFLICT,
        code,
        message: message.into(),
    }
}

fn require_publishable(readiness: &repo::PublishReadiness) -> Result<(), ApiError> {
    if readiness.lessons == 0 {
        return Err(conflict(
            "course_incomplete",
            "Add at least one lesson before publishing.",
        ));
    }
    if readiness.ready_lessons != readiness.lessons {
        return Err(conflict(
            "course_incomplete",
            "Every lesson needs a ready video before publishing.",
        ));
    }
    Ok(())
}

fn admin(user: &AuthUser) -> bool {
    user.role == Role::Admin
}

async fn load_visible(state: &AppState, user: &AuthUser, id: Uuid) -> Result<CourseRow, ApiError> {
    let course = if admin(user) {
        repo::find_any(&state.db, id).await?
    } else {
        repo::find_owned(&state.db, id, user.id).await?
    };
    course.ok_or_else(not_found)
}

fn unique_violation(err: &sqlx::Error) -> bool {
    matches!(err, sqlx::Error::Database(db) if db.is_unique_violation())
}

pub async fn create(
    state: &AppState,
    user: &AuthUser,
    input: NewCourse,
) -> Result<CourseRow, ApiError> {
    user.require_role(COURSE_ROLES)?;
    let id = Uuid::new_v4();
    let base = {
        let slug = slug_base(&input.title);
        if slug.is_empty() {
            "course".to_owned()
        } else {
            slug
        }
    };
    for attempt in 0..10 {
        let slug = if attempt == 0 {
            base.clone()
        } else {
            format!("{base}-{}", &Uuid::new_v4().simple().to_string()[..8])
        };
        let row = NewCourseRow {
            id,
            creator_id: user.id,
            slug: &slug,
            title: &input.title,
            description: &input.description,
            short_description: &input.short_description,
            price_paise: input.price_paise,
            language: input.language.as_deref(),
            level: &input.level,
        };
        match repo::insert(&state.db, &row).await {
            Ok(course) => return Ok(course),
            Err(err) if unique_violation(&err) => continue,
            Err(err) => return Err(err.into()),
        }
    }
    Err(ApiError::Internal(anyhow::anyhow!(
        "could not allocate a unique course slug"
    )))
}

pub async fn get(state: &AppState, user: &AuthUser, id: Uuid) -> Result<CourseRow, ApiError> {
    user.require_role(COURSE_ROLES)?;
    load_visible(state, user, id).await
}

pub async fn list(
    state: &AppState,
    user: &AuthUser,
    cursor: Option<CourseCursor>,
    requested_limit: Option<u32>,
) -> Result<(Vec<CourseRow>, Option<CourseCursor>), ApiError> {
    user.require_role(COURSE_ROLES)?;
    let limit = requested_limit.unwrap_or(20).clamp(1, 50);
    let mut courses = repo::list_owned(&state.db, user.id, cursor, i64::from(limit) + 1).await?;
    let has_more = courses.len() > limit as usize;
    if has_more {
        courses.pop();
    }
    let next = has_more.then(|| {
        let last = courses.last().expect("a page with more rows is non-empty");
        CourseCursor {
            created_at: last.created_at,
            id: last.id,
        }
    });
    Ok((courses, next))
}

pub async fn update(
    state: &AppState,
    user: &AuthUser,
    id: Uuid,
    expected_version: i32,
    changes: CourseChanges,
) -> Result<CourseRow, ApiError> {
    user.require_role(COURSE_ROLES)?;
    let current = load_visible(state, user, id).await?;
    if current.version != expected_version {
        return Err(version_conflict());
    }
    let values = CourseValues {
        title: changes.title.as_deref().unwrap_or(&current.title),
        description: changes
            .description
            .as_deref()
            .unwrap_or(&current.description),
        short_description: changes
            .short_description
            .as_deref()
            .unwrap_or(&current.short_description),
        price_paise: changes.price_paise.unwrap_or(current.price_paise),
        language: changes
            .language
            .as_ref()
            .map_or(current.language.as_deref(), |value| value.as_deref()),
        level: changes.level.as_deref().unwrap_or(&current.level),
        refund_window_days: changes
            .refund_window_days
            .unwrap_or(current.refund_window_days),
    };
    match repo::update(
        &state.db,
        id,
        user.id,
        admin(user),
        expected_version,
        &values,
    )
    .await?
    {
        Some(course) => Ok(course),
        None => {
            if load_visible(state, user, id).await.is_ok() {
                Err(version_conflict())
            } else {
                Err(not_found())
            }
        }
    }
}

pub async fn publish(
    state: &AppState,
    user: &AuthUser,
    id: Uuid,
    expected_version: i32,
) -> Result<CourseRow, ApiError> {
    user.require_role(COURSE_ROLES)?;
    let mut tx = state.db.begin().await?;
    let current = repo::lock_visible(&mut tx, id, user.id, admin(user))
        .await?
        .ok_or_else(not_found)?;
    let readiness = repo::publish_readiness(&mut *tx, id).await?;
    require_publishable(&readiness)?;
    if current.version != expected_version {
        return Err(version_conflict());
    }
    if current.status == CourseStatus::Published {
        tx.commit().await?;
        return Ok(current);
    }
    let published = repo::set_status(
        &mut *tx,
        id,
        user.id,
        admin(user),
        expected_version,
        CourseStatus::Published,
    )
    .await?;
    if let Some(course) = published {
        tx.commit().await?;
        return Ok(course);
    }

    // The SQL readiness guard is defence in depth for video state changes that do not take the
    // course lock. If it fires, preserve the domain error rather than calling it a stale editor.
    let readiness = repo::publish_readiness(&mut *tx, id).await?;
    require_publishable(&readiness)?;
    Err(version_conflict())
}

pub async fn unpublish(
    state: &AppState,
    user: &AuthUser,
    id: Uuid,
    expected_version: i32,
) -> Result<CourseRow, ApiError> {
    user.require_role(COURSE_ROLES)?;
    let current = load_visible(state, user, id).await?;
    if current.version != expected_version {
        return Err(version_conflict());
    }
    if current.status == CourseStatus::Draft {
        return Ok(current);
    }
    repo::set_status(
        &state.db,
        id,
        user.id,
        admin(user),
        expected_version,
        CourseStatus::Draft,
    )
    .await?
    .ok_or_else(version_conflict)
}

pub async fn delete(state: &AppState, user: &AuthUser, id: Uuid) -> Result<(), ApiError> {
    user.require_role(COURSE_ROLES)?;
    let mut tx = state.db.begin().await?;
    let (_, thumbnail) = repo::lock_for_thumbnail(&mut tx, id, user.id, admin(user))
        .await?
        .ok_or_else(not_found)?;
    if !repo::soft_delete(&mut *tx, id, user.id, admin(user)).await? {
        return Err(not_found());
    }
    if let Some(key) = thumbnail {
        crate::images::queue_cleanup(&mut *tx, &key).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Replaces the course thumbnail. `expected_version` is optional here: a picture swap rarely
/// conflicts with text edits, but a caller that sends one gets the usual `409` on a mismatch.
pub async fn set_thumbnail(
    state: &AppState,
    user: &AuthUser,
    id: Uuid,
    bytes: Vec<u8>,
    expected_version: Option<i32>,
) -> Result<CourseRow, ApiError> {
    user.require_role(COURSE_ROLES)?;
    // Fail on ownership before spending CPU on a decode.
    load_visible(state, user, id).await?;
    let processed =
        crate::images::process_async(bytes, crate::images::ImageKind::CourseThumbnail).await?;
    let key = crate::images::store(
        state,
        crate::images::ImageKind::CourseThumbnail,
        id,
        processed,
    )
    .await?;
    replace_thumbnail(state, user, id, Some(&key), expected_version).await
}

pub async fn clear_thumbnail(
    state: &AppState,
    user: &AuthUser,
    id: Uuid,
    expected_version: Option<i32>,
) -> Result<CourseRow, ApiError> {
    user.require_role(COURSE_ROLES)?;
    replace_thumbnail(state, user, id, None, expected_version).await
}

async fn replace_thumbnail(
    state: &AppState,
    user: &AuthUser,
    id: Uuid,
    new_key: Option<&str>,
    expected_version: Option<i32>,
) -> Result<CourseRow, ApiError> {
    let mut tx = state.db.begin().await?;
    let locked = repo::lock_for_thumbnail(&mut tx, id, user.id, admin(user)).await?;
    let outcome = match locked {
        None => Err(not_found()),
        Some((version, _)) if expected_version.is_some_and(|expected| expected != version) => {
            Err(version_conflict())
        }
        Some((_, old)) => Ok(old),
    };
    let old = match outcome {
        Ok(old) => old,
        Err(err) => {
            drop(tx);
            return Err(err);
        }
    };
    let row = repo::set_thumbnail(&mut tx, id, new_key).await?;
    if let Some(old) = old.filter(|old| Some(old.as_str()) != new_key) {
        crate::images::queue_cleanup(&mut *tx, &old).await?;
    }
    tx.commit().await?;
    Ok(row)
}
