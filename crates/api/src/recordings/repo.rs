use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgExecutor};
use uuid::Uuid;
use zeno_core::model::VideoStatus;

use crate::cursor::TimeCursor;

/// A recording as the library shows it: the row, plus the state of the video underneath.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RecordingRow {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub video_id: Option<Uuid>,
    pub source: String,
    pub title: String,
    pub visibility: String,
    /// Whether people watching may leave comments and reactions.
    pub comments_enabled: bool,
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub video_status: Option<VideoStatus>,
    pub duration_seconds: Option<i32>,
    /// All-time views, as of the last flush.
    pub view_count: i64,
}

/// One join on the video's primary key; no stored status to drift out of step with it.
const SELECT: &str = "select r.id, r.owner_id, r.video_id, r.source, r.title, r.visibility, \
    r.comments_enabled, r.version, r.created_at, r.updated_at, r.view_count, v.status as video_status, \
    v.duration_seconds \
    from recordings r left join videos v on v.id = r.video_id";

/// Takes the recording's row lock until the transaction ends, so that whatever is decided about
/// the recording (how many links it has) cannot be decided twice at once. `false`: it is gone.
/// `no key update` is enough to exclude other writers and does not block anything that merely
/// points at the recording.
pub async fn lock(db: &mut PgConnection, id: Uuid) -> Result<bool, sqlx::Error> {
    let row: Option<(Uuid,)> = sqlx::query_as(
        "select id from recordings where id = $1 and deleted_at is null for no key update",
    )
    .bind(id)
    .fetch_optional(db)
    .await?;
    Ok(row.is_some())
}

/// What a retried create needs to know about the row that already has its id.
#[derive(Debug, sqlx::FromRow)]
pub struct Existing {
    pub owner_id: Uuid,
    pub source: String,
    pub video_id: Option<Uuid>,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Inserts unless the id is taken; says whether this call made the row.
pub async fn insert(
    db: &mut PgConnection,
    id: Uuid,
    owner_id: Uuid,
    source: &str,
    title: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "insert into recordings (id, owner_id, source, title) values ($1, $2, $3, $4) \
         on conflict (id) do nothing",
    )
    .bind(id)
    .bind(owner_id)
    .bind(source)
    .bind(title)
    .execute(db)
    .await?;
    Ok(result.rows_affected() == 1)
}

pub async fn existing<'e>(
    db: impl PgExecutor<'e>,
    id: Uuid,
) -> Result<Option<Existing>, sqlx::Error> {
    sqlx::query_as("select owner_id, source, video_id, deleted_at from recordings where id = $1")
        .bind(id)
        .fetch_optional(db)
        .await
}

pub async fn find_owned<'e>(
    db: impl PgExecutor<'e>,
    id: Uuid,
    owner_id: Uuid,
) -> Result<Option<RecordingRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "{SELECT} where r.id = $1 and r.owner_id = $2 and r.deleted_at is null"
    ))
    .bind(id)
    .bind(owner_id)
    .fetch_optional(db)
    .await
}

pub async fn list_owned<'e>(
    db: impl PgExecutor<'e>,
    owner_id: Uuid,
    source: Option<&str>,
    cursor: Option<TimeCursor>,
    limit: i64,
) -> Result<Vec<RecordingRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "{SELECT} where r.owner_id = $1 and r.deleted_at is null \
           and ($2::text is null or r.source = $2) \
           and ($3::timestamptz is null or (r.created_at, r.id) < ($3, $4)) \
         order by r.created_at desc, r.id desc limit $5"
    ))
    .bind(owner_id)
    .bind(source)
    .bind(cursor.map(|c| c.created_at))
    .bind(cursor.map(|c| c.id))
    .bind(limit)
    .fetch_all(db)
    .await
}

/// Changes the title and/or visibility, if the caller read the current version. `false` means no
/// row matched: it is gone, someone else's, or has moved on to another version.
pub async fn update<'e>(
    db: impl PgExecutor<'e>,
    id: Uuid,
    owner_id: Uuid,
    version: i32,
    title: Option<&str>,
    visibility: Option<&str>,
    comments_enabled: Option<bool>,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "update recordings set title = coalesce($4, title), visibility = coalesce($5, visibility), \
                comments_enabled = coalesce($6, comments_enabled), \
                version = version + 1, updated_at = now() \
         where id = $1 and owner_id = $2 and deleted_at is null and version = $3",
    )
    .bind(id)
    .bind(owner_id)
    .bind(version)
    .bind(title)
    .bind(visibility)
    .bind(comments_enabled)
    .execute(db)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Sets who may watch, bumping the version so an editor holding an older copy conflicts.
pub async fn set_visibility<'e>(
    db: impl PgExecutor<'e>,
    id: Uuid,
    owner_id: Uuid,
    visibility: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "update recordings set visibility = $3, version = version + 1, updated_at = now() \
         where id = $1 and owner_id = $2 and deleted_at is null and visibility <> $3",
    )
    .bind(id)
    .bind(owner_id)
    .bind(visibility)
    .execute(db)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Sets the video of a recording that has none. `false`: no such recording, or it already has one.
pub async fn attach_video(
    db: &mut PgConnection,
    id: Uuid,
    owner_id: Uuid,
    video_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "update recordings set video_id = $3, version = version + 1, updated_at = now() \
         where id = $1 and owner_id = $2 and deleted_at is null and video_id is null",
    )
    .bind(id)
    .bind(owner_id)
    .bind(video_id)
    .execute(db)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Marks a recording deleted. `None`: nothing to delete. `Some(video)`: done, and this was its
/// video (if it had one).
pub async fn soft_delete<'e>(
    db: impl PgExecutor<'e>,
    id: Uuid,
    owner_id: Uuid,
) -> Result<Option<Option<Uuid>>, sqlx::Error> {
    let row: Option<(Option<Uuid>,)> = sqlx::query_as(
        "update recordings set deleted_at = now(), updated_at = now(), version = version + 1 \
         where id = $1 and owner_id = $2 and deleted_at is null returning video_id",
    )
    .bind(id)
    .bind(owner_id)
    .fetch_optional(db)
    .await?;
    Ok(row.map(|(video,)| video))
}

/// A recording follows its video: when the video is deleted, so are the recordings made of it.
pub async fn soft_delete_for_video<'e>(
    db: impl PgExecutor<'e>,
    video_id: Uuid,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "update recordings set deleted_at = now(), updated_at = now(), version = version + 1 \
         where video_id = $1 and deleted_at is null",
    )
    .bind(video_id)
    .execute(db)
    .await?;
    Ok(result.rows_affected())
}

/// Removes up to `limit` recordings that were deleted more than `keep_days` days ago, oldest
/// first. Their links, view counts and link viewings go with them (`on delete cascade`). Returns how
/// many went. Rows another instance is already removing are skipped, so every instance can run this.
pub async fn purge_deleted<'e>(
    db: impl PgExecutor<'e>,
    keep_days: i32,
    limit: i64,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "delete from recordings where id in ( \
             select id from recordings \
             where deleted_at is not null and deleted_at < now() - make_interval(days => $1) \
             order by deleted_at limit $2 for update skip locked)",
    )
    .bind(keep_days)
    .bind(limit)
    .execute(db)
    .await?;
    Ok(result.rows_affected())
}

/// Whether a course still uses the video: if so, deleting a recording must not take it away.
pub async fn video_in_use<'e>(
    db: impl PgExecutor<'e>,
    video_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "select exists (select 1 from lessons where video_id = $1 and deleted_at is null)",
    )
    .bind(video_id)
    .fetch_one(db)
    .await
}
