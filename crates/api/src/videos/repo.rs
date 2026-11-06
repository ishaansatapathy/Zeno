use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgExecutor;
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
    sqlx::query_as(&format!("select {COLUMNS} from videos where id = $1"))
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
