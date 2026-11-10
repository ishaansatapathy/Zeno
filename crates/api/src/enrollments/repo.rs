use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgExecutor, PgPool};
use uuid::Uuid;
use zeno_core::model::{CourseStatus, EnrollmentSource};

use crate::enrollments::dto::EnrollmentCursor;

#[derive(Debug, sqlx::FromRow)]
pub struct CourseGate {
    pub id: Uuid,
    pub creator_id: Uuid,
    pub price_paise: i64,
    pub status: CourseStatus,
}

pub async fn lock_course(
    db: &mut PgConnection,
    course_id: Uuid,
) -> Result<Option<CourseGate>, sqlx::Error> {
    sqlx::query_as(
        "select id, creator_id, price_paise, status from courses \
         where id=$1 and deleted_at is null for share",
    )
    .bind(course_id)
    .fetch_optional(db)
    .await
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct EnrollmentRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub course_id: Uuid,
    pub source: EnrollmentSource,
    pub granted_by: Option<Uuid>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub revoked_by: Option<Uuid>,
    pub revocation_kind: Option<String>,
    pub revoke_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const ENROLLMENT_COLUMNS: &str = "id, user_id, course_id, source, granted_by, expires_at, \
    revoked_at, revoked_by, revocation_kind, revoke_reason, created_at, updated_at";

#[derive(Debug, sqlx::FromRow)]
pub struct UpsertedEnrollment {
    #[sqlx(flatten)]
    pub enrollment: EnrollmentRow,
    pub inserted: bool,
    pub changed: bool,
}

pub async fn upsert(
    db: &mut PgConnection,
    user_id: Uuid,
    course_id: Uuid,
    source: EnrollmentSource,
    granted_by: Option<Uuid>,
    expires_at: Option<DateTime<Utc>>,
) -> Result<UpsertedEnrollment, sqlx::Error> {
    let changed: Option<UpsertedEnrollment> = sqlx::query_as(&format!(
        "insert into enrollments (user_id, course_id, source, granted_by, expires_at) \
         values ($1,$2,$3,$4,$5) \
         on conflict (user_id, course_id) do update set \
            source=case when enrollments.revoked_at is not null \
                              or enrollments.expires_at <= now() then excluded.source else enrollments.source end, \
            granted_by=case when enrollments.revoked_at is not null \
                                  or enrollments.expires_at <= now() then excluded.granted_by else enrollments.granted_by end, \
            expires_at=case when enrollments.revoked_at is not null \
                                 or enrollments.expires_at <= now() then excluded.expires_at else enrollments.expires_at end, \
            revoked_at=null, revoked_by=null, revocation_kind=null, revoke_reason=null, updated_at=now() \
         where (enrollments.revoked_at is not null or enrollments.expires_at <= now()) \
           and ($3 <> 'free'::enrollment_source \
                or (enrollments.revoked_at is null and enrollments.source='free')) \
         returning {ENROLLMENT_COLUMNS}, (xmax = 0) as inserted, true as changed"
    ))
    .bind(user_id)
    .bind(course_id)
    .bind(source)
    .bind(granted_by)
    .bind(expires_at)
    .fetch_optional(&mut *db)
    .await?;
    if let Some(row) = changed {
        return Ok(row);
    }
    let enrollment = sqlx::query_as(&format!(
        "select {ENROLLMENT_COLUMNS} from enrollments where user_id=$1 and course_id=$2"
    ))
    .bind(user_id)
    .bind(course_id)
    .fetch_one(db)
    .await?;
    Ok(UpsertedEnrollment {
        enrollment,
        inserted: false,
        changed: false,
    })
}

pub async fn find<'e>(
    db: impl PgExecutor<'e>,
    user_id: Uuid,
    course_id: Uuid,
) -> Result<Option<EnrollmentRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "select {ENROLLMENT_COLUMNS} from enrollments where user_id=$1 and course_id=$2"
    ))
    .bind(user_id)
    .bind(course_id)
    .fetch_optional(db)
    .await
}

pub async fn record_event(
    db: &mut PgConnection,
    enrollment_id: Uuid,
    actor_id: Option<Uuid>,
    action: &str,
    source: EnrollmentSource,
    reason: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "insert into enrollment_events (enrollment_id, actor_id, action, source, reason) \
         values ($1,$2,$3,$4,$5)",
    )
    .bind(enrollment_id)
    .bind(actor_id)
    .bind(action)
    .bind(source)
    .bind(reason)
    .execute(db)
    .await?;
    Ok(())
}

#[derive(Debug, sqlx::FromRow)]
pub struct UserContact {
    pub id: Uuid,
    pub email: Option<String>,
}

pub async fn user_by_id<'e>(
    db: impl PgExecutor<'e>,
    id: Uuid,
) -> Result<Option<UserContact>, sqlx::Error> {
    sqlx::query_as("select id, email from users where id=$1")
        .bind(id)
        .fetch_optional(db)
        .await
}

pub async fn user_by_email<'e>(
    db: impl PgExecutor<'e>,
    email: &str,
) -> Result<Option<UserContact>, sqlx::Error> {
    sqlx::query_as("select id, email from users where email=$1")
        .bind(email)
        .fetch_optional(db)
        .await
}

pub async fn user_by_phone<'e>(
    db: impl PgExecutor<'e>,
    phone: &str,
) -> Result<Option<UserContact>, sqlx::Error> {
    sqlx::query_as("select id, email from users where phone=$1")
        .bind(phone)
        .fetch_optional(db)
        .await
}

pub async fn upsert_invite(
    db: &mut PgConnection,
    course_id: Uuid,
    email: &str,
    granted_by: Uuid,
    access_expires_at: Option<DateTime<Utc>>,
    token_expires_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "insert into course_invites \
            (course_id, email, granted_by, access_expires_at, token_expires_at) \
         values ($1,$2,$3,$4,$5) \
         on conflict (course_id, email) where accepted_at is null and revoked_at is null \
         do update set token_hash=null, granted_by=excluded.granted_by, \
            access_expires_at=excluded.access_expires_at, token_expires_at=excluded.token_expires_at, \
            updated_at=now()",
    )
    .bind(course_id)
    .bind(email)
    .bind(granted_by)
    .bind(access_expires_at)
    .bind(token_expires_at)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn lock_invite_email(db: &mut PgConnection, email: &str) -> Result<(), sqlx::Error> {
    sqlx::query("select pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(email)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn revoke_invite(
    db: &mut PgConnection,
    course_id: Uuid,
    email: &str,
) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query(
        "update course_invites set revoked_at=now(), updated_at=now() \
         where course_id=$1 and email=$2 and accepted_at is null and revoked_at is null",
    )
    .bind(course_id)
    .bind(email)
    .execute(db)
    .await?
    .rows_affected())
}

pub async fn revoke(
    db: &mut PgConnection,
    course_id: Uuid,
    user_id: Uuid,
    reason: &str,
    actor_id: Uuid,
    kind: &str,
) -> Result<Option<EnrollmentRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "update enrollments set revoked_at=now(), revoke_reason=$3, revoked_by=$4, \
            revocation_kind=$5, updated_at=now() \
         where course_id=$1 and user_id=$2 and revoked_at is null \
         returning {ENROLLMENT_COLUMNS}"
    ))
    .bind(course_id)
    .bind(user_id)
    .bind(reason)
    .bind(actor_id)
    .bind(kind)
    .fetch_optional(db)
    .await
}

pub async fn end_sessions_without_access(
    db: &mut PgConnection,
    user_id: Uuid,
) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query(
        "update playback_sessions s set ended_at=now(), end_reason='access_revoked' \
         where s.user_id=$1 and s.ended_at is null \
           and video_access_state(s.user_id, s.video_id) <> 'allowed'",
    )
    .bind(user_id)
    .execute(db)
    .await?
    .rows_affected())
}

#[derive(Debug, sqlx::FromRow)]
pub struct MyCourseRow {
    #[sqlx(flatten)]
    pub enrollment: EnrollmentRow,
    pub slug: String,
    pub title: String,
    pub status: CourseStatus,
    pub price_paise: i64,
}

pub async fn list_for_user(
    db: &PgPool,
    user_id: Uuid,
    cursor: Option<EnrollmentCursor>,
    limit: i64,
) -> Result<Vec<MyCourseRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "select e.{}, c.slug, c.title, c.status, c.price_paise \
         from enrollments e join courses c on c.id=e.course_id \
         where e.user_id=$1 \
           and ($2::timestamptz is null or (e.created_at, e.id) < ($2,$3)) \
         order by e.created_at desc, e.id desc limit $4",
        ENROLLMENT_COLUMNS.replace(", ", ", e.")
    ))
    .bind(user_id)
    .bind(cursor.map(|value| value.created_at))
    .bind(cursor.map(|value| value.id))
    .bind(limit)
    .fetch_all(db)
    .await
}

#[derive(Debug, sqlx::FromRow)]
pub struct StudentRow {
    #[sqlx(flatten)]
    pub enrollment: EnrollmentRow,
    pub name: String,
    pub email: Option<String>,
    pub phone: Option<String>,
}

pub async fn list_for_course(
    db: &PgPool,
    course_id: Uuid,
    cursor: Option<EnrollmentCursor>,
    limit: i64,
) -> Result<Vec<StudentRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "select e.{}, u.name, u.email, u.phone \
         from enrollments e join users u on u.id=e.user_id \
         where e.course_id=$1 \
           and ($2::timestamptz is null or (e.created_at, e.id) < ($2,$3)) \
         order by e.created_at desc, e.id desc limit $4",
        ENROLLMENT_COLUMNS.replace(", ", ", e.")
    ))
    .bind(course_id)
    .bind(cursor.map(|value| value.created_at))
    .bind(cursor.map(|value| value.id))
    .bind(limit)
    .fetch_all(db)
    .await
}

pub async fn find_course<'e>(
    db: impl PgExecutor<'e>,
    course_id: Uuid,
) -> Result<Option<CourseGate>, sqlx::Error> {
    sqlx::query_as(
        "select id, creator_id, price_paise, status from courses \
         where id=$1 and deleted_at is null",
    )
    .bind(course_id)
    .fetch_optional(db)
    .await
}

/// Converts every still-live invitation for the user's verified email. Called inside the email
/// verification transaction, so login cannot observe verification without its grants.
pub async fn accept_invites_for_user(
    db: &mut PgConnection,
    user_id: Uuid,
) -> Result<u64, sqlx::Error> {
    // Serialises verification against a grant that initially observed no account for this email.
    sqlx::query("select pg_advisory_xact_lock(hashtextextended(email, 0)) from users where id=$1")
        .bind(user_id)
        .execute(&mut *db)
        .await?;
    let invites: Vec<(Uuid, Uuid, Option<DateTime<Utc>>, Uuid)> = sqlx::query_as(
        "select i.id, i.course_id, i.access_expires_at, i.granted_by \
         from course_invites i join users u on u.email=i.email \
         join courses c on c.id=i.course_id \
         where u.id=$1 and i.accepted_at is null and i.revoked_at is null \
           and i.token_expires_at > now() \
           and (i.access_expires_at is null or i.access_expires_at > now()) \
           and c.deleted_at is null \
         order by i.course_id for update of i",
    )
    .bind(user_id)
    .fetch_all(&mut *db)
    .await?;
    for (invite_id, course_id, expires_at, granted_by) in &invites {
        let row = upsert(
            &mut *db,
            user_id,
            *course_id,
            EnrollmentSource::Grant,
            Some(*granted_by),
            *expires_at,
        )
        .await?;
        if row.changed {
            record_event(
                &mut *db,
                row.enrollment.id,
                Some(*granted_by),
                if row.inserted {
                    "created"
                } else {
                    "reactivated"
                },
                row.enrollment.source,
                Some("Accepted pending course invitation."),
            )
            .await?;
        }
        sqlx::query("update course_invites set accepted_at=now(), updated_at=now() where id=$1")
            .bind(invite_id)
            .execute(&mut *db)
            .await?;
    }
    Ok(invites.len() as u64)
}
