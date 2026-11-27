use axum::http::StatusCode;
use chrono::{Duration, Utc};
use uuid::Uuid;

use crate::auth::extractor::AuthUser;
use crate::auth::limits::{self, HOUR};
use crate::auth::tokens;
use crate::auth::validation;
use crate::error::ApiError;
use crate::state::AppState;
use crate::workspaces::dto::{InviteResponse, MemberResponse, WorkspaceResponse};
use crate::workspaces::emails;
use crate::workspaces::permissions::{self as rules, WorkspaceRole};
use crate::workspaces::repo;

/// How many workspaces one person may own. Belonging to others' is not limited.
pub const MAX_OWNED: i64 = 5;
/// Members and open invitations together, so a workspace cannot grow past this by inviting faster than people join.
pub const MAX_PEOPLE: i64 = 50;
pub const INVITE_DAYS: i64 = 7;
const MAX_TOKEN_LEN: usize = 128;

fn not_found() -> ApiError {
    ApiError::Http {
        status: StatusCode::NOT_FOUND,
        code: "not_found",
        message: "Workspace not found.".into(),
    }
}

fn forbidden() -> ApiError {
    ApiError::forbidden("forbidden", "You don't have access to this resource.")
}

fn conflict(code: &'static str, message: &'static str) -> ApiError {
    ApiError::Http {
        status: StatusCode::CONFLICT,
        code,
        message: message.into(),
    }
}

fn invalid_token() -> ApiError {
    ApiError::BadRequest {
        code: "invalid_token",
        message: "This invitation link is not valid any more. Ask for a new one.".into(),
    }
}

fn unique_violation(err: &sqlx::Error) -> bool {
    err.as_database_error()
        .is_some_and(|db| db.code().as_deref() == Some("23505"))
}

/// The person's role in a workspace, or "not found": being refused and not existing look the same.
async fn my_role(
    state: &AppState,
    user: &AuthUser,
    workspace: Uuid,
) -> Result<WorkspaceRole, ApiError> {
    repo::role_of(&state.db, workspace, user.id)
        .await?
        .ok_or_else(not_found)
}

pub async fn create(
    state: &AppState,
    user: &AuthUser,
    name: &str,
) -> Result<WorkspaceResponse, ApiError> {
    let name = validation::name(name).map_err(|e| ApiError::validation(vec![e]))?;
    limits::enforce(
        &*state.counters,
        &format!("workspace:create:{}", user.id),
        10,
        HOUR,
    )
    .await?;
    let id = Uuid::new_v4();
    let mut tx = state.db.begin().await?;
    // One person's count of owned workspaces is read and added to under their own lock, so two at once cannot
    // both be the fifth.
    crate::quota::repo::lock(&mut *tx, user.id).await?;
    if repo::count_owned(&mut *tx, user.id).await? >= MAX_OWNED {
        return Err(conflict(
            "too_many_workspaces",
            "You own as many workspaces as an account may. Delete one, or hand it to someone else, to start another.",
        ));
    }
    repo::insert_workspace(&mut *tx, id, &name).await?;
    repo::insert_member(&mut *tx, id, user.id, WorkspaceRole::Owner).await?;
    tx.commit().await?;
    summary(state, user, id).await
}

async fn summary(
    state: &AppState,
    user: &AuthUser,
    id: Uuid,
) -> Result<WorkspaceResponse, ApiError> {
    repo::summary(&state.db, id, user.id)
        .await?
        .map(WorkspaceResponse::from)
        .ok_or_else(not_found)
}

pub async fn list(state: &AppState, user: &AuthUser) -> Result<Vec<WorkspaceResponse>, ApiError> {
    Ok(repo::list_for_user(&state.db, user.id)
        .await?
        .into_iter()
        .map(WorkspaceResponse::from)
        .collect())
}

pub async fn detail(
    state: &AppState,
    user: &AuthUser,
    id: Uuid,
) -> Result<(WorkspaceResponse, Vec<MemberResponse>), ApiError> {
    let workspace = summary(state, user, id).await?;
    let sees_emails = rules::may_manage_people(workspace.role);
    let members = repo::members(&state.db, id)
        .await?
        .into_iter()
        .map(|row| MemberResponse::build(row, user.id, sees_emails))
        .collect();
    Ok((workspace, members))
}

pub async fn rename(
    state: &AppState,
    user: &AuthUser,
    id: Uuid,
    name: &str,
) -> Result<WorkspaceResponse, ApiError> {
    let name = validation::name(name).map_err(|e| ApiError::validation(vec![e]))?;
    let role = my_role(state, user, id).await?;
    if !rules::may_rename(role) {
        return Err(forbidden());
    }
    if !repo::rename(&state.db, id, &name).await? {
        return Err(not_found());
    }
    summary(state, user, id).await
}

pub async fn delete(state: &AppState, user: &AuthUser, id: Uuid) -> Result<(), ApiError> {
    let mut tx = state.db.begin().await?;
    if !repo::lock(&mut *tx, id).await? {
        return Err(not_found());
    }
    let role = repo::role_of(&mut *tx, id, user.id)
        .await?
        .ok_or_else(not_found)?;
    if !rules::may_delete(role) {
        return Err(forbidden());
    }
    repo::soft_delete(&mut *tx, id).await?;
    // Nobody can accept an invitation to a workspace that is gone.
    sqlx::query("update workspace_invites set revoked_at = now() where workspace_id = $1 and accepted_at is null and revoked_at is null")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn change_role(
    state: &AppState,
    user: &AuthUser,
    id: Uuid,
    target: Uuid,
    new_role: WorkspaceRole,
) -> Result<(), ApiError> {
    let mut tx = state.db.begin().await?;
    if !repo::lock(&mut *tx, id).await? {
        return Err(not_found());
    }
    let actor = repo::role_of(&mut *tx, id, user.id)
        .await?
        .ok_or_else(not_found)?;
    if !rules::may_change_role(actor) {
        return Err(forbidden());
    }
    let current = repo::role_of(&mut *tx, id, target)
        .await?
        .ok_or_else(not_found)?;
    // Counted under the lock: two owners handing their ownership away at once cannot both be told another is left.
    if current == WorkspaceRole::Owner
        && new_role != WorkspaceRole::Owner
        && repo::count_owners(&mut *tx, id).await? <= 1
    {
        return Err(conflict(
            "last_owner",
            "A workspace needs an owner. Make someone else an owner first.",
        ));
    }
    repo::set_role(&mut *tx, id, target, new_role).await?;
    tx.commit().await?;
    Ok(())
}

/// Removes somebody, or (when `target` is the caller) leaves.
pub async fn remove_member(
    state: &AppState,
    user: &AuthUser,
    id: Uuid,
    target: Uuid,
) -> Result<(), ApiError> {
    let mut tx = state.db.begin().await?;
    if !repo::lock(&mut *tx, id).await? {
        return Err(not_found());
    }
    let actor = repo::role_of(&mut *tx, id, user.id)
        .await?
        .ok_or_else(not_found)?;
    if target == user.id {
        if actor == WorkspaceRole::Owner && repo::count_owners(&mut *tx, id).await? <= 1 {
            return Err(conflict(
                "last_owner",
                "You are the only owner. Make someone else an owner, or delete the workspace, before you leave.",
            ));
        }
    } else {
        // A member may not remove anybody, and is not told who is there to be removed.
        if actor == WorkspaceRole::Member {
            return Err(forbidden());
        }
        let victim = repo::role_of(&mut *tx, id, target)
            .await?
            .ok_or_else(not_found)?;
        if !rules::may_remove(actor, victim) {
            return Err(forbidden());
        }
    }
    if !repo::remove_member(&mut *tx, id, target).await? {
        return Err(not_found());
    }
    tx.commit().await?;
    Ok(())
}

// ---- invitations -----------------------------------------------------------------------------------------

pub async fn invite(
    state: &AppState,
    user: &AuthUser,
    id: Uuid,
    email: &str,
    role: WorkspaceRole,
) -> Result<InviteResponse, ApiError> {
    let email = validation::email(email).map_err(|e| ApiError::validation(vec![e]))?;
    limits::enforce(
        &*state.counters,
        &format!("workspace:invite:{}", user.id),
        20,
        HOUR,
    )
    .await?;
    let inviter = crate::auth::repo::find_user_by_id(&state.db, user.id)
        .await?
        .ok_or_else(|| ApiError::unauthorized("unauthenticated", "Sign in to continue."))?;

    let raw = tokens::generate_opaque_token();
    let invite_id = Uuid::new_v4();
    let expires_at = Utc::now() + Duration::days(INVITE_DAYS);
    let mut tx = state.db.begin().await?;
    if !repo::lock(&mut *tx, id).await? {
        return Err(not_found());
    }
    let actor = repo::role_of(&mut *tx, id, user.id)
        .await?
        .ok_or_else(not_found)?;
    if !rules::may_invite(actor, role) {
        return Err(forbidden());
    }
    if repo::email_is_member(&mut *tx, id, &email).await? {
        return Err(conflict(
            "already_member",
            "That person is already in this workspace.",
        ));
    }
    repo::retire_expired_invites(&mut *tx, id, &email).await?;
    let people =
        repo::count_members(&mut *tx, id).await? + repo::count_live_invites(&mut *tx, id).await?;
    if people >= MAX_PEOPLE {
        return Err(conflict(
            "workspace_full",
            "This workspace has as many people, counting open invitations, as it can hold.",
        ));
    }
    if let Err(err) = repo::insert_invite(
        &mut *tx,
        invite_id,
        id,
        &email,
        role,
        &tokens::hash_opaque_token(&raw),
        user.id,
        expires_at,
    )
    .await
    {
        return Err(if unique_violation(&err) {
            conflict(
                "already_invited",
                "That address already has an invitation. Revoke it to send another.",
            )
        } else {
            err.into()
        });
    }
    let workspace = repo::summary(&mut *tx, id, user.id)
        .await?
        .ok_or_else(not_found)?;
    let link = format!("{}/invite/{raw}", state.config.client_origin);
    crate::outbox::enqueue_in(
        &mut tx,
        emails::invitation(&email, &workspace.name, &inviter.name, &link, INVITE_DAYS),
        Duration::days(INVITE_DAYS),
    )
    .await?;
    tx.commit().await?;
    crate::outbox::kick(state);
    Ok(InviteResponse {
        id: invite_id,
        email,
        role,
        invited_by: Some(inviter.name),
        created_at: Utc::now(),
        expires_at,
    })
}

pub async fn list_invites(
    state: &AppState,
    user: &AuthUser,
    id: Uuid,
) -> Result<Vec<InviteResponse>, ApiError> {
    let role = my_role(state, user, id).await?;
    if !rules::may_manage_people(role) {
        return Err(forbidden());
    }
    Ok(repo::live_invites(&state.db, id)
        .await?
        .into_iter()
        .map(InviteResponse::from)
        .collect())
}

pub async fn revoke_invite(
    state: &AppState,
    user: &AuthUser,
    id: Uuid,
    invite: Uuid,
) -> Result<(), ApiError> {
    let role = my_role(state, user, id).await?;
    if !rules::may_manage_people(role) {
        return Err(forbidden());
    }
    if !repo::revoke_invite(&state.db, id, invite).await? {
        return Err(ApiError::Http {
            status: StatusCode::NOT_FOUND,
            code: "not_found",
            message: "Invitation not found.".into(),
        });
    }
    Ok(())
}

/// Joins the workspace an invitation is for. Whoever holds the link may try; only the account with the address
/// it was sent to, verified, gets in.
pub async fn accept(
    state: &AppState,
    user: &AuthUser,
    token: &str,
) -> Result<WorkspaceResponse, ApiError> {
    if token.is_empty() || token.len() > MAX_TOKEN_LEN {
        return Err(invalid_token());
    }
    let hash = tokens::hash_opaque_token(token);
    let invite = repo::find_live_invite(&state.db, &hash)
        .await?
        .ok_or_else(invalid_token)?;
    let account = crate::auth::repo::find_user_by_id(&state.db, user.id)
        .await?
        .ok_or_else(|| ApiError::unauthorized("unauthenticated", "Sign in to continue."))?;
    let matches = account.email_verified_at.is_some()
        && account
            .email
            .as_deref()
            .is_some_and(|email| email.to_lowercase() == invite.email);
    if !matches {
        return Err(ApiError::Http {
            status: StatusCode::FORBIDDEN,
            code: "wrong_account",
            message: "This invitation was sent to a different email address. Sign in with that address to accept it.".into(),
        });
    }

    let mut tx = state.db.begin().await?;
    if !repo::lock(&mut *tx, invite.workspace_id).await? {
        return Err(invalid_token());
    }
    // Spent under the lock, in the same transaction that adds the person: two clicks on one link make one member.
    if !repo::claim_invite(&mut *tx, invite.id).await? {
        return Err(invalid_token());
    }
    if repo::role_of(&mut *tx, invite.workspace_id, user.id)
        .await?
        .is_none()
    {
        if repo::count_members(&mut *tx, invite.workspace_id).await? >= MAX_PEOPLE {
            return Err(conflict("workspace_full", "This workspace is full."));
        }
        repo::insert_member(&mut *tx, invite.workspace_id, user.id, invite.role).await?;
    }
    tx.commit().await?;
    summary(state, user, invite.workspace_id).await
}
