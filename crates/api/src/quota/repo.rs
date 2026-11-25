use sqlx::PgExecutor;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::FromRow)]
pub struct Usage {
    pub quota_bytes: i64,
    pub used_bytes: i64,
    pub videos: i64,
}

impl Usage {
    pub fn remaining(&self) -> i64 {
        (self.quota_bytes - self.used_bytes).max(0)
    }
}

/// What the person has kept and may keep. A video counts while it exists and has not failed, whatever stage it is
/// at. `excluding` leaves one video out (the one that is asking for more room).
pub async fn usage<'e>(
    db: impl PgExecutor<'e>,
    user_id: Uuid,
    default_quota: i64,
    excluding: Option<Uuid>,
) -> Result<Option<Usage>, sqlx::Error> {
    sqlx::query_as(
        "select coalesce(u.storage_quota_bytes, $2) as quota_bytes, \
                coalesce(sum(v.size_bytes), 0)::bigint as used_bytes, \
                count(v.id) as videos \
         from users u \
         left join videos v on v.owner_id = u.id and v.deleted_at is null \
              and v.status <> 'failed' and v.id is distinct from $3 \
         where u.id = $1 \
         group by u.id",
    )
    .bind(user_id)
    .bind(default_quota)
    .bind(excluding)
    .fetch_optional(db)
    .await
}

/// Makes everyone who is about to use the same person's room wait their turn: the check and the insert that
/// uses the room must happen under this, or two uploads started together would both see room for the one.
/// `no key update` does not get in the way of anything that only refers to the user (a session, a video), and
/// two of these wait for each other, which is the point.
pub async fn lock<'e>(db: impl PgExecutor<'e>, user_id: Uuid) -> Result<bool, sqlx::Error> {
    let found: Option<i32> =
        sqlx::query_scalar("select 1 from users where id = $1 for no key update")
            .bind(user_id)
            .fetch_optional(db)
            .await?;
    Ok(found.is_some())
}
