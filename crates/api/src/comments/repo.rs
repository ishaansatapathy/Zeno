use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use uuid::Uuid;

use crate::cursor::TimeCursor;

/// A comment with its author's name: one join on the user's primary key.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CommentRow {
    pub id: Uuid,
    pub recording_id: Uuid,
    pub author_id: Uuid,
    pub author_name: String,
    pub kind: String,
    pub body: Option<String>,
    pub emoji: Option<String>,
    pub at_ms: Option<i32>,
    pub created_at: DateTime<Utc>,
}

const SELECT: &str = "select c.id, c.recording_id, c.author_id, u.name as author_name, c.kind, c.body, c.emoji, c.at_ms, \
    c.created_at from recording_comments c join users u on u.id = c.author_id";

/// What the comments of a recording need to know about it, besides the share link that got there.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Conversation {
    pub owner_id: Uuid,
    pub comments_enabled: bool,
}

pub async fn conversation<'e>(
    db: impl PgExecutor<'e>,
    recording_id: Uuid,
) -> Result<Option<Conversation>, sqlx::Error> {
    sqlx::query_as(
        "select owner_id, comments_enabled from recordings where id = $1 and deleted_at is null",
    )
    .bind(recording_id)
    .fetch_optional(db)
    .await
}

/// Whether the session behind a viewing token is an open viewing through this very link.
pub async fn is_open_viewing_of_link<'e>(
    db: impl PgExecutor<'e>,
    session_id: Uuid,
    link_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "select exists (select 1 from playback_sessions \
         where id = $1 and share_link_id = $2 and ended_at is null and expires_at > now())",
    )
    .bind(session_id)
    .bind(link_id)
    .fetch_one(db)
    .await
}

/// One page, newest first. Fetch `limit + 1` to know whether there is another.
pub async fn list<'e>(
    db: impl PgExecutor<'e>,
    recording_id: Uuid,
    after: Option<TimeCursor>,
    limit: i64,
) -> Result<Vec<CommentRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "{SELECT} where c.recording_id = $1 \
           and ($2::timestamptz is null or (c.created_at, c.id) < ($2, $3)) \
         order by c.created_at desc, c.id desc limit $4"
    ))
    .bind(recording_id)
    .bind(after.map(|c| c.created_at))
    .bind(after.map(|c| c.id))
    .bind(limit)
    .fetch_all(db)
    .await
}

pub async fn find<'e>(
    db: impl PgExecutor<'e>,
    id: Uuid,
) -> Result<Option<CommentRow>, sqlx::Error> {
    sqlx::query_as(&format!("{SELECT} where c.id = $1"))
        .bind(id)
        .fetch_optional(db)
        .await
}

pub struct NewRow<'a> {
    pub id: Uuid,
    pub recording_id: Uuid,
    pub author_id: Uuid,
    pub kind: &'a str,
    pub body: Option<&'a str>,
    pub emoji: Option<&'a str>,
    pub at_ms: Option<i32>,
}

/// Adds a comment. A unique violation means the id is taken.
pub async fn insert<'e>(db: impl PgExecutor<'e>, row: &NewRow<'_>) -> Result<(), sqlx::Error> {
    sqlx::query(
        "insert into recording_comments (id, recording_id, author_id, kind, body, emoji, at_ms) \
         values ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(row.id)
    .bind(row.recording_id)
    .bind(row.author_id)
    .bind(row.kind)
    .bind(row.body)
    .bind(row.emoji)
    .bind(row.at_ms)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn count_on<'e>(db: impl PgExecutor<'e>, recording_id: Uuid) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("select count(*) from recording_comments where recording_id = $1")
        .bind(recording_id)
        .fetch_one(db)
        .await
}

pub async fn count_by<'e>(
    db: impl PgExecutor<'e>,
    recording_id: Uuid,
    author_id: Uuid,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "select count(*) from recording_comments where recording_id = $1 and author_id = $2",
    )
    .bind(recording_id)
    .bind(author_id)
    .fetch_one(db)
    .await
}

pub async fn delete<'e>(db: impl PgExecutor<'e>, id: Uuid) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("delete from recording_comments where id = $1")
        .bind(id)
        .execute(db)
        .await?;
    Ok(result.rows_affected() == 1)
}
