use axum::http::StatusCode;
use chrono::{Duration, Utc};
use uuid::Uuid;
use zeno_core::model::{CourseStatus, EnrollmentSource, Role};

use crate::auth::extractor::AuthUser;
use crate::enrollments::dto::{Contact, EnrollmentCursor, GrantInput};
use crate::enrollments::repo::{self, EnrollmentRow, MyCourseRow, StudentRow};
use crate::error::ApiError;
use crate::mail::OutgoingEmail;
use crate::outbox;
use crate::state::AppState;

const COURSE_ROLES: &[Role] = &[Role::Creator, Role::Admin];
const INVITE_TTL_DAYS: i64 = 7;

fn not_found(subject: &'static str) -> ApiError {
    ApiError::Http {
        status: StatusCode::NOT_FOUND,
        code: "not_found",
        message: format!("{subject} not found."),
    }
}

fn admin(user: &AuthUser) -> bool {
    user.role == Role::Admin
}

fn ensure_owner(user: &AuthUser, creator_id: Uuid) -> Result<(), ApiError> {
    user.require_role(COURSE_ROLES)?;
    if admin(user) || user.id == creator_id {
        Ok(())
    } else {
        Err(not_found("Course"))
    }
}

pub async fn enroll_free(
    state: &AppState,
    user: &AuthUser,
    course_id: Uuid,
) -> Result<EnrollmentRow, ApiError> {
    let mut tx = state.db.begin().await?;
    let course = repo::lock_course(&mut tx, course_id)
        .await?
        .ok_or_else(|| not_found("Course"))?;
    if course.status != CourseStatus::Published {
        return Err(ApiError::Http {
            status: StatusCode::CONFLICT,
            code: "course_unavailable",
            message: "This course is not accepting enrollments.".into(),
        });
    }
    if course.price_paise != 0 {
        return Err(ApiError::Http {
            status: StatusCode::PAYMENT_REQUIRED,
            code: "payment_required",
            message: format!("Complete checkout at /v1/checkout/{course_id}."),
        });
    }
    let row = repo::upsert(
        &mut tx,
        user.id,
        course_id,
        EnrollmentSource::Free,
        None,
        None,
    )
    .await?;
    if row.enrollment.revoked_at.is_some() {
        let creator_revoked = matches!(
            row.enrollment.revocation_kind.as_deref(),
            Some("creator" | "admin")
        );
        return Err(ApiError::Http {
            status: StatusCode::CONFLICT,
            code: if creator_revoked {
                "access_revoked_by_creator"
            } else {
                "access_revoked"
            },
            message: "Access was revoked. Contact the course creator to restore it.".into(),
        });
    }
    if row
        .enrollment
        .expires_at
        .is_some_and(|expiry| expiry <= Utc::now())
    {
        return Err(ApiError::Http {
            status: StatusCode::CONFLICT,
            code: "access_expired",
            message: "Renew this enrollment through its original access path.".into(),
        });
    }
    if row.changed {
        repo::record_event(
            &mut tx,
            row.enrollment.id,
            Some(user.id),
            if row.inserted {
                "created"
            } else {
                "reactivated"
            },
            row.enrollment.source,
            Some("Free course claimed."),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(row.enrollment)
}

pub enum GrantResult {
    Enrolled(EnrollmentRow),
    Invited(String),
}

async fn resolve_user(
    state: &AppState,
    contact: &Contact,
) -> Result<Option<repo::UserContact>, ApiError> {
    Ok(match contact {
        Contact::UserId(id) => repo::user_by_id(&state.db, *id).await?,
        Contact::Email(email) => repo::user_by_email(&state.db, email).await?,
        Contact::Phone(phone) => repo::user_by_phone(&state.db, phone).await?,
    })
}

pub async fn grant(
    state: &AppState,
    actor: &AuthUser,
    course_id: Uuid,
    input: GrantInput,
) -> Result<GrantResult, ApiError> {
    actor.require_role(COURSE_ROLES)?;
    let mut tx = state.db.begin().await?;
    let course = repo::lock_course(&mut tx, course_id)
        .await?
        .ok_or_else(|| not_found("Course"))?;
    ensure_owner(actor, course.creator_id)?;
    let target = resolve_user(state, &input.contact).await?;
    if let Some(target) = target {
        let source = if admin(actor) {
            EnrollmentSource::Admin
        } else {
            EnrollmentSource::Grant
        };
        let row = repo::upsert(
            &mut tx,
            target.id,
            course_id,
            source,
            Some(actor.id),
            input.expires_at,
        )
        .await?;
        if row.changed {
            repo::record_event(
                &mut tx,
                row.enrollment.id,
                Some(actor.id),
                if row.inserted {
                    "created"
                } else {
                    "reactivated"
                },
                row.enrollment.source,
                Some("Manual course grant."),
            )
            .await?;
        }
        tx.commit().await?;
        return Ok(GrantResult::Enrolled(row.enrollment));
    }

    let Contact::Email(email) = input.contact else {
        return Err(not_found("User"));
    };
    repo::lock_invite_email(&mut tx, &email).await?;
    // The account may have been inserted after the first lookup. The shared advisory lock with
    // email verification closes the only window in which an invite could otherwise be stranded.
    if let Some(target) = repo::user_by_email(&mut *tx, &email).await? {
        let source = if admin(actor) {
            EnrollmentSource::Admin
        } else {
            EnrollmentSource::Grant
        };
        let row = repo::upsert(
            &mut tx,
            target.id,
            course_id,
            source,
            Some(actor.id),
            input.expires_at,
        )
        .await?;
        if row.changed {
            repo::record_event(
                &mut tx,
                row.enrollment.id,
                Some(actor.id),
                if row.inserted {
                    "created"
                } else {
                    "reactivated"
                },
                row.enrollment.source,
                Some("Manual course grant."),
            )
            .await?;
        }
        tx.commit().await?;
        return Ok(GrantResult::Enrolled(row.enrollment));
    }
    repo::upsert_invite(
        &mut tx,
        course_id,
        &email,
        actor.id,
        input.expires_at,
        Utc::now() + Duration::days(INVITE_TTL_DAYS),
    )
    .await?;
    tx.commit().await?;
    outbox::enqueue(
        state,
        OutgoingEmail {
            to: email.clone(),
            subject: "You have been invited to a Zeno course".into(),
            text: format!(
                "A course creator granted you access. Sign up and verify this email to accept:\n{}/register",
                state.config.client_origin
            ),
        },
        Duration::days(INVITE_TTL_DAYS),
    )
    .await?;
    Ok(GrantResult::Invited(email))
}

async fn resolve_revoke_user(
    state: &AppState,
    raw: &str,
) -> Result<Option<repo::UserContact>, ApiError> {
    if let Ok(id) = Uuid::parse_str(raw) {
        return Ok(repo::user_by_id(&state.db, id).await?);
    }
    if raw.contains('@') {
        let email = crate::auth::validation::email(raw)
            .map_err(|error| ApiError::validation(vec![error]))?;
        return Ok(repo::user_by_email(&state.db, &email).await?);
    }
    Ok(repo::user_by_phone(&state.db, raw).await?)
}

pub async fn revoke(
    state: &AppState,
    actor: &AuthUser,
    course_id: Uuid,
    raw_user: &str,
    reason: &str,
) -> Result<(), ApiError> {
    actor.require_role(COURSE_ROLES)?;
    let mut tx = state.db.begin().await?;
    let course = repo::lock_course(&mut tx, course_id)
        .await?
        .ok_or_else(|| not_found("Course"))?;
    ensure_owner(actor, course.creator_id)?;
    let target = resolve_revoke_user(state, raw_user).await?;
    if let Some(target) = target {
        if let Some(row) = repo::revoke(
            &mut tx,
            course_id,
            target.id,
            reason,
            actor.id,
            if admin(actor) { "admin" } else { "creator" },
        )
        .await?
        {
            repo::record_event(
                &mut tx,
                row.id,
                Some(actor.id),
                "revoked",
                row.source,
                Some(reason),
            )
            .await?;
            repo::end_sessions_without_access(&mut tx, target.id).await?;
        }
    } else if raw_user.contains('@') {
        let email = crate::auth::validation::email(raw_user)
            .map_err(|error| ApiError::validation(vec![error]))?;
        repo::revoke_invite(&mut tx, course_id, &email).await?;
    }
    tx.commit().await?;
    Ok(())
}

fn page_limit(requested: Option<u32>) -> i64 {
    i64::from(requested.unwrap_or(20).clamp(1, 50))
}

pub async fn list_mine(
    state: &AppState,
    user: &AuthUser,
    cursor: Option<EnrollmentCursor>,
    requested_limit: Option<u32>,
) -> Result<(Vec<MyCourseRow>, Option<EnrollmentCursor>), ApiError> {
    let limit = page_limit(requested_limit);
    let mut rows = repo::list_for_user(&state.db, user.id, cursor, limit + 1).await?;
    let more = rows.len() > limit as usize;
    if more {
        rows.pop();
    }
    let next = more.then(|| {
        let row = rows.last().expect("a page with more rows is non-empty");
        EnrollmentCursor {
            created_at: row.enrollment.created_at,
            id: row.enrollment.id,
        }
    });
    Ok((rows, next))
}

pub async fn list_students(
    state: &AppState,
    actor: &AuthUser,
    course_id: Uuid,
    cursor: Option<EnrollmentCursor>,
    requested_limit: Option<u32>,
) -> Result<(Vec<StudentRow>, Option<EnrollmentCursor>), ApiError> {
    actor.require_role(COURSE_ROLES)?;
    let course = repo::find_course(&state.db, course_id)
        .await?
        .ok_or_else(|| not_found("Course"))?;
    ensure_owner(actor, course.creator_id)?;
    let limit = page_limit(requested_limit);
    let mut rows = repo::list_for_course(&state.db, course_id, cursor, limit + 1).await?;
    let more = rows.len() > limit as usize;
    if more {
        rows.pop();
    }
    let next = more.then(|| {
        let row = rows.last().expect("a page with more rows is non-empty");
        EnrollmentCursor {
            created_at: row.enrollment.created_at,
            id: row.enrollment.id,
        }
    });
    Ok((rows, next))
}
