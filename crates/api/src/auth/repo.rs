use chrono::{DateTime, Utc};
use sqlx::types::ipnetwork::IpNetwork;
use sqlx::PgExecutor;
use uuid::Uuid;
use zeno_core::model::Role;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct UserRow {
    pub id: Uuid,
    pub name: String,
    pub email: Option<String>,
    pub password_hash: String,
    pub role: Role,
    pub email_verified_at: Option<DateTime<Utc>>,
}

pub async fn find_user_by_email<'e>(
    db: impl PgExecutor<'e>,
    email: &str,
) -> Result<Option<UserRow>, sqlx::Error> {
    sqlx::query_as(
        "select id, name, email, password_hash, role, email_verified_at from users where email = $1",
    )
    .bind(email)
    .fetch_optional(db)
    .await
}

pub async fn find_user_by_id<'e>(
    db: impl PgExecutor<'e>,
    id: Uuid,
) -> Result<Option<UserRow>, sqlx::Error> {
    sqlx::query_as(
        "select id, name, email, password_hash, role, email_verified_at from users where id = $1",
    )
    .bind(id)
    .fetch_optional(db)
    .await
}

/// Returns `None` when the email is already taken. Letting the database arbitrate (rather
/// than checking first) keeps concurrent sign-ups for one address race-free.
pub async fn insert_user<'e>(
    db: impl PgExecutor<'e>,
    name: &str,
    email: &str,
    password_hash: &str,
    role: Role,
) -> Result<Option<UserRow>, sqlx::Error> {
    sqlx::query_as(
        "insert into users (name, email, password_hash, role) values ($1, $2, $3, $4) \
         on conflict (email) do nothing \
         returning id, name, email, password_hash, role, email_verified_at",
    )
    .bind(name)
    .bind(email)
    .bind(password_hash)
    .bind(role)
    .fetch_optional(db)
    .await
}

pub async fn mark_email_verified<'e>(
    db: impl PgExecutor<'e>,
    user_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "update users set email_verified_at = coalesce(email_verified_at, now()), updated_at = now() where id = $1",
    )
    .bind(user_id)
    .execute(db)
    .await?;
    Ok(())
}

/// Resetting through an emailed link proves control of the mailbox, so it also verifies it.
pub async fn update_password<'e>(
    db: impl PgExecutor<'e>,
    user_id: Uuid,
    password_hash: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "update users set password_hash = $2, email_verified_at = coalesce(email_verified_at, now()), updated_at = now() where id = $1",
    )
    .bind(user_id)
    .bind(password_hash)
    .execute(db)
    .await?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "auth_token_purpose", rename_all = "snake_case")]
pub enum TokenPurpose {
    EmailVerification,
    PasswordReset,
}

pub async fn invalidate_auth_tokens<'e>(
    db: impl PgExecutor<'e>,
    user_id: Uuid,
    purpose: TokenPurpose,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "update auth_tokens set used_at = now() where user_id = $1 and purpose = $2 and used_at is null",
    )
    .bind(user_id)
    .bind(purpose)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn insert_auth_token<'e>(
    db: impl PgExecutor<'e>,
    user_id: Uuid,
    purpose: TokenPurpose,
    token_hash: &[u8],
    expires_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "insert into auth_tokens (user_id, purpose, token_hash, expires_at) values ($1, $2, $3, $4)",
    )
    .bind(user_id)
    .bind(purpose)
    .bind(token_hash)
    .bind(expires_at)
    .execute(db)
    .await?;
    Ok(())
}

/// Read-only check, so obviously bad tokens can be turned away before doing expensive work.
pub async fn auth_token_is_usable<'e>(
    db: impl PgExecutor<'e>,
    token_hash: &[u8],
    purpose: TokenPurpose,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "select exists(select 1 from auth_tokens where token_hash = $1 and purpose = $2 and used_at is null and expires_at > now())",
    )
    .bind(token_hash)
    .bind(purpose)
    .fetch_one(db)
    .await
}

/// Atomically spends a token. Exactly one concurrent caller can get `Some`.
pub async fn consume_auth_token<'e>(
    db: impl PgExecutor<'e>,
    token_hash: &[u8],
    purpose: TokenPurpose,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        "update auth_tokens set used_at = now() \
         where token_hash = $1 and purpose = $2 and used_at is null and expires_at > now() \
         returning user_id",
    )
    .bind(token_hash)
    .bind(purpose)
    .fetch_optional(db)
    .await
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SessionRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub family_id: Uuid,
    pub expires_at: DateTime<Utc>,
    pub absolute_expires_at: DateTime<Utc>,
    pub rotated_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

pub struct NewSession<'a> {
    pub user_id: Uuid,
    pub family_id: Uuid,
    pub token_hash: &'a [u8],
    pub user_agent: Option<&'a str>,
    pub ip: Option<IpNetwork>,
    pub expires_at: DateTime<Utc>,
    pub absolute_expires_at: DateTime<Utc>,
}

pub async fn insert_session<'e>(
    db: impl PgExecutor<'e>,
    session: &NewSession<'_>,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar(
        "insert into sessions (user_id, family_id, refresh_token_hash, user_agent, ip, expires_at, absolute_expires_at) \
         values ($1, $2, $3, $4, $5, $6, $7) returning id",
    )
    .bind(session.user_id)
    .bind(session.family_id)
    .bind(session.token_hash)
    .bind(session.user_agent)
    .bind(session.ip)
    .bind(session.expires_at)
    .bind(session.absolute_expires_at)
    .fetch_one(db)
    .await
}

pub async fn find_session_by_token_hash<'e>(
    db: impl PgExecutor<'e>,
    token_hash: &[u8],
) -> Result<Option<SessionRow>, sqlx::Error> {
    sqlx::query_as(
        "select id, user_id, family_id, expires_at, absolute_expires_at, rotated_at, revoked_at \
         from sessions where refresh_token_hash = $1",
    )
    .bind(token_hash)
    .fetch_optional(db)
    .await
}

/// Returns `false` if someone else rotated (or revoked) the session first.
pub async fn mark_session_rotated<'e>(
    db: impl PgExecutor<'e>,
    session_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "update sessions set rotated_at = now() where id = $1 and rotated_at is null and revoked_at is null",
    )
    .bind(session_id)
    .execute(db)
    .await?;
    Ok(result.rows_affected() == 1)
}

pub async fn revoke_family<'e>(
    db: impl PgExecutor<'e>,
    family_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "update sessions set revoked_at = now() where family_id = $1 and revoked_at is null",
    )
    .bind(family_id)
    .execute(db)
    .await?;
    Ok(())
}

/// Returns the families that were just revoked so their access tokens can be cut off too.
pub async fn revoke_user_sessions<'e>(
    db: impl PgExecutor<'e>,
    user_id: Uuid,
) -> Result<Vec<Uuid>, sqlx::Error> {
    let mut families: Vec<Uuid> = sqlx::query_scalar(
        "update sessions set revoked_at = now() where user_id = $1 and revoked_at is null returning family_id",
    )
    .bind(user_id)
    .fetch_all(db)
    .await?;
    families.sort_unstable();
    families.dedup();
    Ok(families)
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Purged {
    pub sessions: u64,
    pub tokens: u64,
    pub events: u64,
    pub emails: u64,
}

/// Deletes rows that can no longer matter. A rotated token must stay until its own expiry so
/// reuse can still be detected; revoked and past-lifetime rows are dead either way (a
/// replay of a deleted token is refused exactly like a revoked one). Audit events are kept
/// for a year, then dropped to honour data minimisation; delivered or abandoned outbox rows
/// (already blanked) go after a week.
pub async fn purge_expired<'e, E>(db: E) -> Result<Purged, sqlx::Error>
where
    E: PgExecutor<'e> + Copy,
{
    let sessions = sqlx::query(
        "delete from sessions where absolute_expires_at < now() \
            or expires_at < now() - interval '7 days' \
            or revoked_at < now() - interval '1 day'",
    )
    .execute(db)
    .await?
    .rows_affected();
    let tokens = sqlx::query(
        "delete from auth_tokens where expires_at < now() - interval '1 day' \
            or used_at < now() - interval '1 day'",
    )
    .execute(db)
    .await?
    .rows_affected();
    let events =
        sqlx::query("delete from auth_events where created_at < now() - interval '365 days'")
            .execute(db)
            .await?
            .rows_affected();
    let emails = sqlx::query(
        "delete from email_outbox where coalesce(sent_at, failed_at) < now() - interval '7 days'",
    )
    .execute(db)
    .await?
    .rows_affected();
    Ok(Purged {
        sessions,
        tokens,
        events,
        emails,
    })
}
