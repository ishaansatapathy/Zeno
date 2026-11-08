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
