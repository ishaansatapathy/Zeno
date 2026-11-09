use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgExecutor};
use uuid::Uuid;

use crate::progress::logic::Stored;

pub async fn still_this_video(
    db: &mut PgConnection,
    lesson_id: Uuid,
    video_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let current: Option<Uuid> = sqlx::query_scalar(
        "select video_id from lessons where id=$1 and deleted_at is null for share",
    )
    .bind(lesson_id)
    .fetch_optional(db)
    .await?;
    Ok(current == Some(video_id))
}

/// Where the viewer last was in this lesson.
pub async fn last_position<'e>(
    db: impl PgExecutor<'e>,
    user_id: Uuid,
    lesson_id: Uuid,
) -> Result<Option<i32>, sqlx::Error> {
    sqlx::query_scalar(
        "select last_position_seconds from lesson_progress where user_id=$1 and lesson_id=$2",
    )
    .bind(user_id)
    .bind(lesson_id)
    .fetch_optional(db)
    .await
}

#[derive(sqlx::FromRow)]
struct StoredRow {
    last_position_seconds: i32,
    furthest_seconds: i32,
    watched_seconds: i32,
    completed: bool,
    elapsed_secs: i64,
}

/// Creates the row for a first sighting. Returns false if it already existed (another heartbeat
/// got there first), in which case the caller folds into the existing row.
pub async fn insert_first(
    db: &mut PgConnection,
    user_id: Uuid,
    lesson_id: Uuid,
    first: Stored,
) -> Result<bool, sqlx::Error> {
    let inserted = sqlx::query(
        "insert into lesson_progress \
            (user_id, lesson_id, last_position_seconds, furthest_seconds, watched_seconds) \
         values ($1,$2,$3,$4,$5) on conflict do nothing",
    )
    .bind(user_id)
    .bind(lesson_id)
    .bind(first.last_position)
    .bind(first.furthest)
    .bind(first.watched)
    .execute(db)
    .await?;
    Ok(inserted.rows_affected() == 1)
}

/// Locks the row (two screens heartbeating at once take turns) and returns it with the seconds
/// since it was last written.
pub async fn lock(
    db: &mut PgConnection,
    user_id: Uuid,
    lesson_id: Uuid,
) -> Result<Option<(Stored, i64)>, sqlx::Error> {
    let row: Option<StoredRow> = sqlx::query_as(
        "select last_position_seconds, furthest_seconds, watched_seconds, \
                completed_at is not null as completed, \
                extract(epoch from now() - last_watched_at)::bigint as elapsed_secs \
         from lesson_progress where user_id=$1 and lesson_id=$2 for update",
    )
    .bind(user_id)
    .bind(lesson_id)
    .fetch_optional(db)
    .await?;
    Ok(row.map(|row| {
        (
            Stored {
                last_position: row.last_position_seconds,
                furthest: row.furthest_seconds,
                watched: row.watched_seconds,
                completed: row.completed,
            },
            row.elapsed_secs,
        )
    }))
}

pub async fn save(
    db: &mut PgConnection,
    user_id: Uuid,
    lesson_id: Uuid,
    state: Stored,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "update lesson_progress set last_position_seconds=$3, furthest_seconds=$4, \
            watched_seconds=$5, last_watched_at=now(), \
            completed_at=case when $6 then coalesce(completed_at, now()) else completed_at end \
         where user_id=$1 and lesson_id=$2",
    )
    .bind(user_id)
    .bind(lesson_id)
    .bind(state.last_position)
    .bind(state.furthest)
    .bind(state.watched)
    .bind(state.completed)
    .execute(db)
    .await?;
    Ok(())
}

#[derive(Debug, sqlx::FromRow)]
pub struct ContinueRow {
    pub course_slug: String,
    pub course_title: String,
    pub thumbnail_key: Option<String>,
    pub lesson_id: Uuid,
    pub lesson_title: String,
    pub lesson_position: i32,
    pub duration_seconds: Option<i32>,
    pub last_position_seconds: i32,
    pub watched_seconds: i32,
    pub last_watched_at: DateTime<Utc>,
}

/// The unfinished lesson each course was last played in, newest first. A lesson the viewer can
/// no longer open (revoked, expired, video removed) is left out rather than offered and refused.
pub async fn continue_watching<'e>(
    db: impl PgExecutor<'e>,
    user_id: Uuid,
    limit: i64,
) -> Result<Vec<ContinueRow>, sqlx::Error> {
    sqlx::query_as(
        "select * from ( \
            select distinct on (c.id) \
                c.slug as course_slug, c.title as course_title, c.thumbnail_key, \
                l.id as lesson_id, l.title as lesson_title, l.position as lesson_position, \
                v.duration_seconds, p.last_position_seconds, p.watched_seconds, p.last_watched_at \
            from lesson_progress p \
            join lessons l on l.id = p.lesson_id and l.deleted_at is null \
            join courses c on c.id = l.course_id and c.deleted_at is null \
            join videos v on v.id = l.video_id and v.deleted_at is null \
            where p.user_id = $1 and p.completed_at is null \
              and video_access_state($1, l.video_id) = 'allowed' \
            order by c.id, p.last_watched_at desc \
         ) latest order by last_watched_at desc limit $2",
    )
    .bind(user_id)
    .bind(limit)
    .fetch_all(db)
    .await
}

#[derive(Debug, sqlx::FromRow)]
pub struct CourseLessonProgress {
    pub lesson_id: Uuid,
    pub position: i32,
    pub title: String,
    pub is_free_preview: bool,
    pub duration_seconds: Option<i32>,
    pub last_position_seconds: Option<i32>,
    pub watched_seconds: Option<i32>,
    pub completed: bool,
}

/// Whether the user holds a live enrollment in the course.
pub async fn enrolled<'e>(
    db: impl PgExecutor<'e>,
    user_id: Uuid,
    course_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "select exists ( \
            select 1 from enrollments e join courses c on c.id = e.course_id \
            where e.user_id=$1 and e.course_id=$2 and e.revoked_at is null \
              and (e.expires_at is null or e.expires_at > now()) \
              and (c.deleted_at is null or c.deleted_at > now() - interval '30 days'))",
    )
    .bind(user_id)
    .bind(course_id)
    .fetch_one(db)
    .await
}

pub async fn course_lessons<'e>(
    db: impl PgExecutor<'e>,
    user_id: Uuid,
    course_id: Uuid,
) -> Result<Vec<CourseLessonProgress>, sqlx::Error> {
    sqlx::query_as(
        "select l.id as lesson_id, l.position, l.title, l.is_free_preview, v.duration_seconds, \
                p.last_position_seconds, p.watched_seconds, \
                coalesce(p.completed_at is not null, false) as completed \
         from lessons l \
         join videos v on v.id = l.video_id \
         left join lesson_progress p on p.lesson_id = l.id and p.user_id = $1 \
         where l.course_id = $2 and l.deleted_at is null \
         order by l.position",
    )
    .bind(user_id)
    .bind(course_id)
    .fetch_all(db)
    .await
}
