use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgExecutor, PgPool};
use uuid::Uuid;
use zeno_core::model::VideoStatus;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct VideoRow {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub title: String,
    pub original_filename: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub part_size: i64,
    pub source_key: String,
    pub upload_id: Option<String>,
    pub status: VideoStatus,
    pub error_code: Option<String>,
    pub duration_seconds: Option<i32>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub renditions: Value,
    pub created_at: DateTime<Utc>,
    pub uploaded_at: Option<DateTime<Utc>>,
    pub ready_at: Option<DateTime<Utc>>,
}

const COLUMNS: &str =
    "id, owner_id, title, original_filename, content_type, size_bytes, part_size, \
    source_key, upload_id, status, error_code, duration_seconds, width, height, renditions, \
    created_at, uploaded_at, ready_at";

pub struct NewVideoRow<'a> {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub title: &'a str,
    pub original_filename: &'a str,
    pub content_type: &'a str,
    pub size_bytes: i64,
    pub part_size: i64,
    pub source_key: &'a str,
    pub upload_id: &'a str,
}

pub async fn insert<'e>(
    db: impl PgExecutor<'e>,
    video: &NewVideoRow<'_>,
) -> Result<VideoRow, sqlx::Error> {
    sqlx::query_as(&format!(
        "insert into videos (id, owner_id, title, original_filename, content_type, size_bytes, part_size, source_key, upload_id) \
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9) returning {COLUMNS}"
    ))
    .bind(video.id)
    .bind(video.owner_id)
    .bind(video.title)
    .bind(video.original_filename)
    .bind(video.content_type)
    .bind(video.size_bytes)
    .bind(video.part_size)
    .bind(video.source_key)
    .bind(video.upload_id)
    .fetch_one(db)
    .await
}

pub async fn find<'e>(db: impl PgExecutor<'e>, id: Uuid) -> Result<Option<VideoRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "select {COLUMNS} from videos where id = $1 and deleted_at is null"
    ))
    .bind(id)
    .fetch_optional(db)
    .await
}

pub async fn count_uploading<'e>(
    db: impl PgExecutor<'e>,
    owner_id: Uuid,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("select count(*) from videos where owner_id = $1 and status = 'uploading'")
        .bind(owner_id)
        .fetch_one(db)
        .await
}

/// Moves an upload to processing. `None` means it was no longer uploading (someone else
/// completed it first), which callers treat as success.
pub async fn mark_processing<'e>(
    db: impl PgExecutor<'e>,
    id: Uuid,
) -> Result<Option<VideoRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "update videos set status = 'processing', uploaded_at = now(), upload_id = null, updated_at = now() \
         where id = $1 and status = 'uploading' returning {COLUMNS}"
    ))
    .bind(id)
    .fetch_optional(db)
    .await
}

pub async fn mark_failed<'e>(
    db: impl PgExecutor<'e>,
    id: Uuid,
    error_code: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "update videos set status = 'failed', error_code = $2, upload_id = null, updated_at = now() \
         where id = $1 and status <> 'ready'",
    )
    .bind(id)
    .bind(error_code)
    .execute(db)
    .await?;
    Ok(())
}

/// Deletes an upload that has not been completed. Returns whether a row was removed.
pub async fn delete_uploading<'e>(db: impl PgExecutor<'e>, id: Uuid) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("delete from videos where id = $1 and status = 'uploading'")
        .bind(id)
        .execute(db)
        .await?;
    Ok(result.rows_affected() == 1)
}

pub async fn stale_uploads<'e>(
    db: impl PgExecutor<'e>,
    older_than_hours: i32,
    limit: i64,
) -> Result<Vec<VideoRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "select {COLUMNS} from videos where status = 'uploading' \
         and created_at < now() - make_interval(hours => $1) order by created_at limit $2"
    ))
    .bind(older_than_hours)
    .bind(limit)
    .fetch_all(db)
    .await
}

/// Marks a video deleted so it stops being served. Returns whether it was live.
pub async fn soft_delete<'e>(db: impl PgExecutor<'e>, id: Uuid) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "update videos set deleted_at = now(), updated_at = now() \
         where id = $1 and deleted_at is null",
    )
    .bind(id)
    .execute(db)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// A deleted video makes every active lesson that references it unpublishable. Draft all affected
/// courses and bump their editor version in the same transaction as the video tombstone.
pub async fn invalidate_courses_for_video(
    db: &mut sqlx::PgConnection,
    id: Uuid,
) -> Result<u64, sqlx::Error> {
    let course_ids: Vec<Uuid> = sqlx::query_scalar(
        "select c.id from courses c \
         where c.deleted_at is null and exists ( \
             select 1 from lessons l \
             where l.course_id=c.id and l.video_id=$1 and l.deleted_at is null \
         ) order by c.id for update of c",
    )
    .bind(id)
    .fetch_all(&mut *db)
    .await?;
    if course_ids.is_empty() {
        return Ok(0);
    }
    let result = sqlx::query(
        "update courses c set status='draft', published_at=null, version=version+1, updated_at=now() \
         where c.id=any($1::uuid[])",
    )
    .bind(&course_ids)
    .execute(db)
    .await?;
    Ok(result.rows_affected())
}

/// Deleted videos that can be purged now: none has a transcode job that is still live, because
/// such a job may yet write output for it.
pub async fn purgeable<'e>(
    db: impl PgExecutor<'e>,
    limit: i64,
) -> Result<Vec<(Uuid, String)>, sqlx::Error> {
    sqlx::query_as(
        "select v.id, v.source_key from videos v \
         where v.deleted_at is not null \
           and v.storage_purged_at is null \
           and not exists ( \
               select 1 from jobs j where j.kind = 'transcode' \
                 and j.payload ->> 'video_id' = v.id::text \
                 and j.completed_at is null and j.failed_at is null) \
         order by v.deleted_at limit $1",
    )
    .bind(limit)
    .fetch_all(db)
    .await
}

/// Finalises a storage purge. Unreferenced videos are removed completely; rows referenced by
/// lesson history remain as tombstones and are marked so the storage sweep does not retry them.
/// Both paths re-check the job condition so a newly enqueued job cannot lose its video.
pub async fn delete_purged(db: &PgPool, id: Uuid) -> Result<bool, sqlx::Error> {
    let retained = sqlx::query(
        "update videos v set storage_purged_at=now(), updated_at=now() \
         where v.id=$1 and v.deleted_at is not null and v.storage_purged_at is null \
           and exists (select 1 from lessons l where l.video_id=v.id) \
           and not exists ( \
               select 1 from jobs j where j.kind = 'transcode' \
                 and j.payload ->> 'video_id' = v.id::text \
                 and j.completed_at is null and j.failed_at is null)",
    )
    .bind(id)
    .execute(db)
    .await?;
    if retained.rows_affected() == 1 {
        return Ok(true);
    }
    let result = sqlx::query(
        "delete from videos v where v.id = $1 and v.deleted_at is not null \
           and v.storage_purged_at is null \
           and not exists (select 1 from lessons l where l.video_id=v.id) \
           and not exists ( \
               select 1 from jobs j where j.kind = 'transcode' \
                 and j.payload ->> 'video_id' = v.id::text \
                 and j.completed_at is null and j.failed_at is null)",
    )
    .bind(id)
    .execute(db)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Finished, still-live videos whose original upload is older than the retention period.
pub async fn sources_past_retention<'e>(
    db: impl PgExecutor<'e>,
    retention_days: i32,
    limit: i64,
) -> Result<Vec<(Uuid, String)>, sqlx::Error> {
    sqlx::query_as(
        "select id, source_key from videos \
         where source_purged_at is null and deleted_at is null and status in ('ready', 'failed') \
           and coalesce(ready_at, updated_at) < now() - make_interval(days => $1) \
         order by coalesce(ready_at, updated_at) limit $2",
    )
    .bind(retention_days)
    .bind(limit)
    .fetch_all(db)
    .await
}

pub async fn mark_source_purged<'e>(db: impl PgExecutor<'e>, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("update videos set source_purged_at = now() where id = $1")
        .bind(id)
        .execute(db)
        .await?;
    Ok(())
}
