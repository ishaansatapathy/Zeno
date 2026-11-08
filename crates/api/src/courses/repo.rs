use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgExecutor, PgPool};
use uuid::Uuid;
use zeno_core::model::CourseStatus;

use crate::courses::dto::CourseCursor;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CourseRow {
    pub id: Uuid,
    pub creator_id: Uuid,
    pub slug: String,
    pub title: String,
    pub description: String,
    pub short_description: String,
    pub price_paise: i64,
    pub currency: String,
    pub status: CourseStatus,
    pub thumbnail_key: Option<String>,
    pub language: Option<String>,
    pub level: String,
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub published_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

const COLUMNS: &str = "id, creator_id, slug, title, description, short_description, \
    price_paise, currency, status, thumbnail_key, language, level, version, created_at, \
    updated_at, published_at, deleted_at";

pub struct NewCourseRow<'a> {
    pub id: Uuid,
    pub creator_id: Uuid,
    pub slug: &'a str,
    pub title: &'a str,
    pub description: &'a str,
    pub short_description: &'a str,
    pub price_paise: i64,
    pub language: Option<&'a str>,
    pub level: &'a str,
}

pub async fn insert(db: &PgPool, course: &NewCourseRow<'_>) -> Result<CourseRow, sqlx::Error> {
    sqlx::query_as(&format!(
        "insert into courses (id, creator_id, slug, title, description, short_description, price_paise, language, level) \
         values ($1,$2,$3,$4,$5,$6,$7,$8,$9) returning {COLUMNS}"
    ))
    .bind(course.id)
    .bind(course.creator_id)
    .bind(course.slug)
    .bind(course.title)
    .bind(course.description)
    .bind(course.short_description)
    .bind(course.price_paise)
    .bind(course.language)
    .bind(course.level)
    .fetch_one(db)
    .await
}

pub async fn find_owned<'e>(
    db: impl PgExecutor<'e>,
    id: Uuid,
    creator_id: Uuid,
) -> Result<Option<CourseRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "select {COLUMNS} from courses where id = $1 and creator_id = $2 and deleted_at is null"
    ))
    .bind(id)
    .bind(creator_id)
    .fetch_optional(db)
    .await
}

pub async fn find_any<'e>(
    db: impl PgExecutor<'e>,
    id: Uuid,
) -> Result<Option<CourseRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "select {COLUMNS} from courses where id = $1 and deleted_at is null"
    ))
    .bind(id)
    .fetch_optional(db)
    .await
}

pub async fn lock_visible(
    db: &mut PgConnection,
    id: Uuid,
    actor_id: Uuid,
    admin: bool,
) -> Result<Option<CourseRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "select {COLUMNS} from courses \
         where id=$1 and deleted_at is null and ($2 or creator_id=$3) for update"
    ))
    .bind(id)
    .bind(admin)
    .bind(actor_id)
    .fetch_optional(db)
    .await
}

pub async fn list_owned(
    db: &PgPool,
    creator_id: Uuid,
    cursor: Option<CourseCursor>,
    limit: i64,
) -> Result<Vec<CourseRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "select {COLUMNS} from courses \
         where creator_id = $1 and deleted_at is null \
           and ($2::timestamptz is null or (created_at, id) < ($2, $3)) \
         order by created_at desc, id desc limit $4"
    ))
    .bind(creator_id)
    .bind(cursor.map(|c| c.created_at))
    .bind(cursor.map(|c| c.id))
    .bind(limit)
    .fetch_all(db)
    .await
}

pub struct CourseValues<'a> {
    pub title: &'a str,
    pub description: &'a str,
    pub short_description: &'a str,
    pub price_paise: i64,
    pub language: Option<&'a str>,
    pub level: &'a str,
}

pub async fn update(
    db: &PgPool,
    id: Uuid,
    actor_id: Uuid,
    admin: bool,
    expected_version: i32,
    values: &CourseValues<'_>,
) -> Result<Option<CourseRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "update courses set title=$5, description=$6, short_description=$7, price_paise=$8, \
            language=$9, level=$10, version=version+1, updated_at=now() \
         where id=$1 and deleted_at is null and version=$2 and ($3 or creator_id=$4) \
         returning {COLUMNS}"
    ))
    .bind(id)
    .bind(expected_version)
    .bind(admin)
    .bind(actor_id)
    .bind(values.title)
    .bind(values.description)
    .bind(values.short_description)
    .bind(values.price_paise)
    .bind(values.language)
    .bind(values.level)
    .fetch_optional(db)
    .await
}

#[derive(Debug, sqlx::FromRow)]
pub struct PublishReadiness {
    pub lessons: i64,
    pub ready_lessons: i64,
}

pub async fn publish_readiness<'e>(
    db: impl PgExecutor<'e>,
    course_id: Uuid,
) -> Result<PublishReadiness, sqlx::Error> {
    sqlx::query_as(
        "select count(*) as lessons, \
                count(*) filter (where l.video_id is not null and v.status = 'ready' and v.deleted_at is null) as ready_lessons \
         from lessons l left join videos v on v.id = l.video_id \
         where l.course_id = $1 and l.deleted_at is null",
    )
    .bind(course_id)
    .fetch_one(db)
    .await
}

pub async fn set_status<'e>(
    db: impl PgExecutor<'e>,
    id: Uuid,
    actor_id: Uuid,
    admin: bool,
    expected_version: i32,
    status: CourseStatus,
) -> Result<Option<CourseRow>, sqlx::Error> {
    let published = status == CourseStatus::Published;
    sqlx::query_as(&format!(
        "update courses set status=$5, published_at=case when $6 then coalesce(published_at, now()) else null end, \
            version=version+1, updated_at=now() \
         where id=$1 and deleted_at is null and version=$2 and ($3 or creator_id=$4) \
           and (not $6 or ( \
             exists (select 1 from lessons l where l.course_id=$1 and l.deleted_at is null) \
             and not exists ( \
               select 1 from lessons l left join videos v on v.id=l.video_id \
               where l.course_id=$1 and l.deleted_at is null \
                 and (l.video_id is null or v.status <> 'ready' or v.deleted_at is not null) \
             ) \
           )) \
         returning {COLUMNS}"
    ))
    .bind(id)
    .bind(expected_version)
    .bind(admin)
    .bind(actor_id)
    .bind(status)
    .bind(published)
    .fetch_optional(db)
    .await
}

pub async fn soft_delete(
    db: &PgPool,
    id: Uuid,
    actor_id: Uuid,
    admin: bool,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "update courses set deleted_at=now(), updated_at=now(), version=version+1 \
         where id=$1 and deleted_at is null and ($2 or creator_id=$3)",
    )
    .bind(id)
    .bind(admin)
    .bind(actor_id)
    .execute(db)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Locks a course the actor may edit and returns its version and current thumbnail key.
pub async fn lock_for_thumbnail(
    db: &mut PgConnection,
    id: Uuid,
    actor_id: Uuid,
    admin: bool,
) -> Result<Option<(i32, Option<String>)>, sqlx::Error> {
    sqlx::query_as(
        "select version, thumbnail_key from courses          where id=$1 and deleted_at is null and ($2 or creator_id=$3) for update",
    )
    .bind(id)
    .bind(admin)
    .bind(actor_id)
    .fetch_optional(db)
    .await
}

pub async fn set_thumbnail(
    db: &mut PgConnection,
    id: Uuid,
    key: Option<&str>,
) -> Result<CourseRow, sqlx::Error> {
    sqlx::query_as(&format!(
        "update courses set thumbnail_key=$2, version=version+1, updated_at=now()          where id=$1 returning {COLUMNS}"
    ))
    .bind(id)
    .bind(key)
    .fetch_one(db)
    .await
}
