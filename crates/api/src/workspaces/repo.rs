use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use uuid::Uuid;

use crate::workspaces::permissions::WorkspaceRole;

#[derive(Debug, sqlx::FromRow)]
pub struct SummaryRow {
    pub id: Uuid,
    pub name: String,
    pub role: WorkspaceRole,
    pub member_count: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct MemberRow {
    pub user_id: Uuid,
    pub name: String,
    pub email: Option<String>,
    pub role: WorkspaceRole,
    pub joined_at: DateTime<Utc>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct InviteRow {
    pub id: Uuid,
    pub email: String,
    pub role: WorkspaceRole,
    pub invited_by_name: Option<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// What a token names, before anything is decided about it.
#[derive(Debug, sqlx::FromRow)]
pub struct InviteLookup {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub email: String,
    pub role: WorkspaceRole,
}

const SUMMARY: &str = "select w.id, w.name, m.role, w.created_at, \
        (select count(*) from workspace_members c where c.workspace_id = w.id) as member_count \
     from workspaces w join workspace_members m on m.workspace_id = w.id and m.user_id = $1 \
     where w.deleted_at is null";

pub async fn insert_workspace<'e>(
    db: impl PgExecutor<'e>,
    id: Uuid,
    name: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("insert into workspaces (id, name) values ($1, $2)")
        .bind(id)
        .bind(name)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn insert_member<'e>(
    db: impl PgExecutor<'e>,
    workspace: Uuid,
    user: Uuid,
    role: WorkspaceRole,
) -> Result<(), sqlx::Error> {
    sqlx::query("insert into workspace_members (workspace_id, user_id, role) values ($1, $2, $3)")
        .bind(workspace)
        .bind(user)
        .bind(role)
        .execute(db)
        .await?;
    Ok(())
}

/// Makes everyone who is about to change who is in a workspace wait their turn: counting the owners and then
/// removing one must not be done by two people at once, or each would see another owner left. `false` when the
/// workspace does not exist (or has been ended).
pub async fn lock<'e>(db: impl PgExecutor<'e>, workspace: Uuid) -> Result<bool, sqlx::Error> {
    let found: Option<i32> = sqlx::query_scalar(
        "select 1 from workspaces where id = $1 and deleted_at is null for update",
    )
    .bind(workspace)
    .fetch_optional(db)
    .await?;
    Ok(found.is_some())
}

/// What the person is in the workspace, or `None` if they are not in it (or it is gone): the same answer, so
/// that nobody learns that a workspace exists by being refused.
pub async fn role_of<'e>(
    db: impl PgExecutor<'e>,
    workspace: Uuid,
    user: Uuid,
) -> Result<Option<WorkspaceRole>, sqlx::Error> {
    sqlx::query_scalar(
        "select m.role from workspace_members m join workspaces w on w.id = m.workspace_id \
         where m.workspace_id = $1 and m.user_id = $2 and w.deleted_at is null",
    )
    .bind(workspace)
    .bind(user)
    .fetch_optional(db)
    .await
}

pub async fn list_for_user<'e>(
    db: impl PgExecutor<'e>,
    user: Uuid,
) -> Result<Vec<SummaryRow>, sqlx::Error> {
    sqlx::query_as(&format!("{SUMMARY} order by w.created_at desc, w.id"))
        .bind(user)
        .fetch_all(db)
        .await
}

pub async fn summary<'e>(
    db: impl PgExecutor<'e>,
    workspace: Uuid,
    user: Uuid,
) -> Result<Option<SummaryRow>, sqlx::Error> {
    sqlx::query_as(&format!("{SUMMARY} and w.id = $2"))
        .bind(user)
        .bind(workspace)
        .fetch_optional(db)
        .await
}

pub async fn members<'e>(
    db: impl PgExecutor<'e>,
    workspace: Uuid,
) -> Result<Vec<MemberRow>, sqlx::Error> {
    sqlx::query_as(
        "select m.user_id, u.name, u.email, m.role, m.joined_at \
         from workspace_members m join users u on u.id = m.user_id \
         where m.workspace_id = $1 order by m.joined_at, m.user_id",
    )
    .bind(workspace)
    .fetch_all(db)
    .await
}

pub async fn count_members<'e>(
    db: impl PgExecutor<'e>,
    workspace: Uuid,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("select count(*) from workspace_members where workspace_id = $1")
        .bind(workspace)
        .fetch_one(db)
        .await
}

pub async fn count_owners<'e>(
    db: impl PgExecutor<'e>,
    workspace: Uuid,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "select count(*) from workspace_members where workspace_id = $1 and role = 'owner'",
    )
    .bind(workspace)
    .fetch_one(db)
    .await
}

/// How many workspaces the person owns (and still has).
pub async fn count_owned<'e>(db: impl PgExecutor<'e>, user: Uuid) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "select count(*) from workspace_members m join workspaces w on w.id = m.workspace_id \
         where m.user_id = $1 and m.role = 'owner' and w.deleted_at is null",
    )
    .bind(user)
    .fetch_one(db)
    .await
}

pub async fn set_role<'e>(
    db: impl PgExecutor<'e>,
    workspace: Uuid,
    user: Uuid,
    role: WorkspaceRole,
) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query(
        "update workspace_members set role = $3 where workspace_id = $1 and user_id = $2",
    )
    .bind(workspace)
    .bind(user)
    .bind(role)
    .execute(db)
    .await?
    .rows_affected()
        == 1)
}

pub async fn remove_member<'e>(
    db: impl PgExecutor<'e>,
    workspace: Uuid,
    user: Uuid,
) -> Result<bool, sqlx::Error> {
    Ok(
        sqlx::query("delete from workspace_members where workspace_id = $1 and user_id = $2")
            .bind(workspace)
            .bind(user)
            .execute(db)
            .await?
            .rows_affected()
            == 1,
    )
}

pub async fn rename<'e>(
    db: impl PgExecutor<'e>,
    workspace: Uuid,
    name: &str,
) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query(
        "update workspaces set name = $2, updated_at = now() where id = $1 and deleted_at is null",
    )
    .bind(workspace)
    .bind(name)
    .execute(db)
    .await?
    .rows_affected()
        == 1)
}

pub async fn soft_delete<'e>(
    db: impl PgExecutor<'e>,
    workspace: Uuid,
) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query("update workspaces set deleted_at = now(), updated_at = now() where id = $1 and deleted_at is null")
        .bind(workspace)
        .execute(db)
        .await?
        .rows_affected()
        == 1)
}

// ---- invitations -----------------------------------------------------------------------------------------

pub async fn email_is_member<'e>(
    db: impl PgExecutor<'e>,
    workspace: Uuid,
    email: &str,
) -> Result<bool, sqlx::Error> {
    let found: Option<i32> = sqlx::query_scalar(
        "select 1 from workspace_members m join users u on u.id = m.user_id \
         where m.workspace_id = $1 and lower(u.email) = $2",
    )
    .bind(workspace)
    .bind(email)
    .fetch_optional(db)
    .await?;
    Ok(found.is_some())
}

/// Invitations nobody answered in time stop counting as out: they no longer stand in the way of a new one.
pub async fn retire_expired_invites<'e>(
    db: impl PgExecutor<'e>,
    workspace: Uuid,
    email: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "update workspace_invites set revoked_at = now() \
         where workspace_id = $1 and email = $2 and accepted_at is null and revoked_at is null \
           and expires_at <= now()",
    )
    .bind(workspace)
    .bind(email)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn count_live_invites<'e>(
    db: impl PgExecutor<'e>,
    workspace: Uuid,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "select count(*) from workspace_invites \
         where workspace_id = $1 and accepted_at is null and revoked_at is null and expires_at > now()",
    )
    .bind(workspace)
    .fetch_one(db)
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_invite<'e>(
    db: impl PgExecutor<'e>,
    id: Uuid,
    workspace: Uuid,
    email: &str,
    role: WorkspaceRole,
    token_hash: &[u8],
    invited_by: Uuid,
    expires_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "insert into workspace_invites (id, workspace_id, email, role, token_hash, invited_by, expires_at) \
         values ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(id)
    .bind(workspace)
    .bind(email)
    .bind(role)
    .bind(token_hash)
    .bind(invited_by)
    .bind(expires_at)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn live_invites<'e>(
    db: impl PgExecutor<'e>,
    workspace: Uuid,
) -> Result<Vec<InviteRow>, sqlx::Error> {
    sqlx::query_as(
        "select i.id, i.email, i.role, u.name as invited_by_name, i.created_at, i.expires_at \
         from workspace_invites i left join users u on u.id = i.invited_by \
         where i.workspace_id = $1 and i.accepted_at is null and i.revoked_at is null and i.expires_at > now() \
         order by i.created_at desc, i.id",
    )
    .bind(workspace)
    .fetch_all(db)
    .await
}

pub async fn revoke_invite<'e>(
    db: impl PgExecutor<'e>,
    workspace: Uuid,
    invite: Uuid,
) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query(
        "update workspace_invites set revoked_at = now() \
         where id = $1 and workspace_id = $2 and accepted_at is null and revoked_at is null",
    )
    .bind(invite)
    .bind(workspace)
    .execute(db)
    .await?
    .rows_affected()
        == 1)
}

/// The invitation a token names, if it is still good: not answered, not revoked, not out of time, and its
/// workspace not ended. Every way of not being good is the same `None`.
pub async fn find_live_invite<'e>(
    db: impl PgExecutor<'e>,
    token_hash: &[u8],
) -> Result<Option<InviteLookup>, sqlx::Error> {
    sqlx::query_as(
        "select i.id, i.workspace_id, i.email, i.role from workspace_invites i \
         join workspaces w on w.id = i.workspace_id \
         where i.token_hash = $1 and i.accepted_at is null and i.revoked_at is null \
           and i.expires_at > now() and w.deleted_at is null",
    )
    .bind(token_hash)
    .fetch_optional(db)
    .await
}

/// Uses the invitation up. `false` if somebody else already did (or it was revoked a moment ago).
pub async fn claim_invite<'e>(db: impl PgExecutor<'e>, invite: Uuid) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query(
        "update workspace_invites set accepted_at = now() \
         where id = $1 and accepted_at is null and revoked_at is null and expires_at > now()",
    )
    .bind(invite)
    .execute(db)
    .await?
    .rows_affected()
        == 1)
}
