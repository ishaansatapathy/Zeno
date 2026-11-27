use sqlx::PgExecutor;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessState {
    Allowed,
    Expired,
    Revoked,
}

pub async fn for_video<'e>(
    db: impl PgExecutor<'e>,
    user_id: Uuid,
    video_id: Uuid,
) -> Result<AccessState, sqlx::Error> {
    let state: String = sqlx::query_scalar("select video_access_state($1,$2)")
        .bind(user_id)
        .bind(video_id)
        .fetch_one(db)
        .await?;
    Ok(match state.as_str() {
        "allowed" => AccessState::Allowed,
        "expired" => AccessState::Expired,
        _ => AccessState::Revoked,
    })
}

pub async fn lesson_video<'e>(
    db: impl PgExecutor<'e>,
    lesson_id: Uuid,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        "select video_id from lessons where id=$1 and deleted_at is null and video_id is not null",
    )
    .bind(lesson_id)
    .fetch_optional(db)
    .await
}

/// Whether the person is in a workspace that holds a recording made of this video. The same condition as the
/// workspace branch of `video_access_state`, asked on its own because the direct play route is a preview surface
/// that must not open to every kind of entitlement (an enrolled student plays through the lesson, not this).
pub async fn via_workspace<'e>(
    db: impl PgExecutor<'e>,
    user_id: Uuid,
    video_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "select exists ( \
             select 1 from recordings r \
             join workspaces w on w.id = r.workspace_id and w.deleted_at is null \
             join workspace_members m on m.workspace_id = w.id and m.user_id = $1 \
             where r.video_id = $2 and r.deleted_at is null)",
    )
    .bind(user_id)
    .bind(video_id)
    .fetch_one(db)
    .await
}
