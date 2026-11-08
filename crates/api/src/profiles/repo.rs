use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgExecutor, PgPool};
use uuid::Uuid;

use crate::profiles::dto::ProfileInput;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ProfileRow {
    pub user_id: Uuid,
    pub handle: String,
    pub display_name: String,
    pub bio: String,
    pub avatar_key: Option<String>,
    pub links: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub const COLUMNS: &str =
    "user_id, handle, display_name, bio, avatar_key, links, created_at, updated_at";

pub async fn find<'e>(
    db: impl PgExecutor<'e>,
    user_id: Uuid,
) -> Result<Option<ProfileRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "select {COLUMNS} from creator_profiles where user_id = $1"
    ))
    .bind(user_id)
    .fetch_optional(db)
    .await
}

pub async fn find_by_handle<'e>(
    db: impl PgExecutor<'e>,
    handle: &str,
) -> Result<Option<ProfileRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "select {COLUMNS} from creator_profiles where handle = $1"
    ))
    .bind(handle)
    .fetch_optional(db)
    .await
}

/// Creates the profile or updates everything except the handle, which is permanent so shared
/// links keep working.
pub async fn upsert(
    db: &PgPool,
    user_id: Uuid,
    input: &ProfileInput,
) -> Result<Option<ProfileRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "insert into creator_profiles (user_id, handle, display_name, bio, links) \
         values ($1,$2,$3,$4,$5) \
         on conflict (user_id) do update set \
            display_name = excluded.display_name, bio = excluded.bio, \
            links = excluded.links, updated_at = now() \
         where creator_profiles.handle = excluded.handle \
         returning {COLUMNS}"
    ))
    .bind(user_id)
    .bind(&input.handle)
    .bind(&input.display_name)
    .bind(&input.bio)
    .bind(serde_json::to_value(&input.links).expect("links serialise"))
    .fetch_optional(db)
    .await
}

/// Locks the profile row and returns its current avatar key (`None` row = no profile).
pub async fn lock_avatar(
    db: &mut PgConnection,
    user_id: Uuid,
) -> Result<Option<Option<String>>, sqlx::Error> {
    sqlx::query_scalar("select avatar_key from creator_profiles where user_id = $1 for update")
        .bind(user_id)
        .fetch_optional(db)
        .await
}

pub async fn set_avatar(
    db: &mut PgConnection,
    user_id: Uuid,
    key: Option<&str>,
) -> Result<ProfileRow, sqlx::Error> {
    sqlx::query_as(&format!(
        "update creator_profiles set avatar_key = $2, updated_at = now() \
         where user_id = $1 returning {COLUMNS}"
    ))
    .bind(user_id)
    .bind(key)
    .fetch_one(db)
    .await
}
