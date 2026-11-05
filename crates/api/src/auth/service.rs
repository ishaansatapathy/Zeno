use std::net::IpAddr;

use chrono::{DateTime, Duration, Utc};
use sqlx::types::ipnetwork::IpNetwork;
use uuid::Uuid;
use zeno_core::model::Role;

use crate::auth::repo::{self, NewSession, TokenPurpose, UserRow};
use crate::auth::{emails, limits, tokens};
use crate::error::ApiError;
use crate::state::AppState;

const REFRESH_TTL_DAYS: i64 = 30;
/// A session family dies after this long no matter how often it is refreshed.
const SESSION_ABSOLUTE_TTL_DAYS: i64 = 90;
/// Two browser tabs can race to refresh with the same cookie. Presenting a just-rotated
/// token inside this window is treated as that race, not as theft.
const ROTATION_GRACE_SECS: i64 = 10;
const VERIFY_TTL_HOURS: i64 = 24;
const RESET_TTL_HOURS: i64 = 1;
const MAX_LOGIN_PASSWORD_BYTES: usize = 1024;
const MAX_USER_AGENT_CHARS: usize = 512;

pub struct ClientContext {
    pub ip: Option<IpAddr>,
    pub user_agent: Option<String>,
}

pub struct RegisterInput {
    pub name: String,
    pub email: String,
    pub password: String,
    pub role: Role,
}

impl std::fmt::Debug for RegisterInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegisterInput")
            .field("email", &self.email)
            .field("role", &self.role)
            .field("password", &"<redacted>")
            .finish_non_exhaustive()
    }
}

pub struct Authenticated {
    pub user: UserRow,
    pub access_token: String,
    pub access_expires_at: DateTime<Utc>,
    pub refresh_token: String,
    pub refresh_expires_at: DateTime<Utc>,
}

pub enum RefreshFailure {
    /// The session is invalid, expired, revoked or was reused: clear the cookies.
    Dead,
    /// Another request is rotating this token right now: leave the cookies alone, because
    /// that request is about to set fresh ones.
    Conflict,
    Other(ApiError),
}

impl<E: Into<anyhow::Error>> From<E> for RefreshFailure {
    fn from(err: E) -> Self {
        RefreshFailure::Other(ApiError::Internal(err.into()))
    }
}

fn invalid_token() -> ApiError {
    ApiError::bad_request(
        "invalid_token",
        "This link is invalid or has expired. Request a new one.",
    )
}

fn link(state: &AppState, path: &str, token: &str) -> String {
    format!("{}/{path}/{token}", state.config.client_origin)
}

async fn issue_token(
    state: &AppState,
    user_id: Uuid,
    purpose: TokenPurpose,
    ttl: Duration,
) -> Result<String, ApiError> {
    let raw = tokens::generate_opaque_token();
    let hash = tokens::hash_opaque_token(&raw);
    let mut tx = state.db.begin().await?;
    repo::invalidate_auth_tokens(&mut *tx, user_id, purpose).await?;
    repo::insert_auth_token(&mut *tx, user_id, purpose, &hash, Utc::now() + ttl).await?;
    tx.commit().await?;
    Ok(raw)
}

async fn send_verification(state: &AppState, user: &UserRow) -> Result<(), ApiError> {
    let Some(to) = &user.email else {
        return Ok(());
    };
    let raw = issue_token(
        state,
        user.id,
        TokenPurpose::EmailVerification,
        Duration::hours(VERIFY_TTL_HOURS),
    )
    .await?;
    state.send_email(emails::verification(to, &link(state, "verify-email", &raw)));
    Ok(())
}

/// Always completes the same work (including the expensive hash) whether or not the email
/// is already registered, and the caller always gives the same answer, so sign-up can't be
/// used to discover who has an account.
pub async fn register(state: &AppState, input: RegisterInput) -> Result<(), ApiError> {
    let hash = state.passwords.hash(input.password).await?;
    match repo::insert_user(&state.db, &input.name, &input.email, &hash, input.role).await? {
        Some(user) => send_verification(state, &user).await?,
        None => {
            let Some(existing) = repo::find_user_by_email(&state.db, &input.email).await? else {
                return Ok(());
            };
            if existing.email_verified_at.is_none() {
                send_verification(state, &existing).await?;
            } else if let Some(to) = &existing.email {
                let origin = &state.config.client_origin;
                state.send_email(emails::already_registered(
                    to,
                    &format!("{origin}/login"),
                    &format!("{origin}/forgot-password"),
                ));
            }
        }
    }
    Ok(())
}

pub async fn verify_email(
    state: &AppState,
    raw_token: &str,
    ctx: &ClientContext,
) -> Result<Authenticated, ApiError> {
    let hash = tokens::hash_opaque_token(raw_token);
    let mut tx = state.db.begin().await?;
    let user_id = repo::consume_auth_token(&mut *tx, &hash, TokenPurpose::EmailVerification)
        .await?
        .ok_or_else(invalid_token)?;
    repo::mark_email_verified(&mut *tx, user_id).await?;
    tx.commit().await?;

    let user = repo::find_user_by_id(&state.db, user_id)
        .await?
        .ok_or_else(invalid_token)?;
    start_session(state, user, ctx).await
}

pub async fn resend_verification(state: &AppState, email: &str) -> Result<(), ApiError> {
    if let Some(user) = repo::find_user_by_email(&state.db, email).await? {
        if user.email_verified_at.is_none() {
            send_verification(state, &user).await?;
        }
    }
    Ok(())
}

pub async fn login(
    state: &AppState,
    raw_email: &str,
    password: String,
    ctx: &ClientContext,
) -> Result<Authenticated, ApiError> {
    let email = crate::auth::validation::normalize_email(raw_email);
    limits::ensure_login_allowed(&*state.counters, &email, ctx.ip).await?;

    let user = if email.len() <= 254 {
        repo::find_user_by_email(&state.db, &email).await?
    } else {
        None
    };
    // An absurdly long password can't be right; skip feeding it to the hasher.
    let password = if password.len() > MAX_LOGIN_PASSWORD_BYTES {
        String::new()
    } else {
        password
    };
    let valid = state
        .passwords
        .verify(password, user.as_ref().map(|u| u.password_hash.clone()))
        .await?;

    let Some(user) = user.filter(|_| valid) else {
        limits::record_login_failure(&*state.counters, &email, ctx.ip).await;
        return Err(ApiError::unauthorized(
            "invalid_credentials",
            "Invalid email or password.",
        ));
    };
    if user.email_verified_at.is_none() {
        return Err(ApiError::forbidden(
            "email_not_verified",
            "Please verify your email before signing in.",
        ));
    }

    limits::clear_login_failures(&*state.counters, &email, ctx.ip).await;
    start_session(state, user, ctx).await
}

async fn start_session(
    state: &AppState,
    user: UserRow,
    ctx: &ClientContext,
) -> Result<Authenticated, ApiError> {
    let now = Utc::now();
    let refresh_token = tokens::generate_opaque_token();
    let token_hash = tokens::hash_opaque_token(&refresh_token);
    let expires_at = now + Duration::days(REFRESH_TTL_DAYS);
    let user_agent: Option<String> = ctx
        .user_agent
        .as_ref()
        .map(|ua| ua.chars().take(MAX_USER_AGENT_CHARS).collect());

    let session_id = repo::insert_session(
        &state.db,
        &NewSession {
            user_id: user.id,
            family_id: Uuid::new_v4(),
            token_hash: &token_hash,
            user_agent: user_agent.as_deref(),
            ip: ctx.ip.map(IpNetwork::from),
            expires_at,
            absolute_expires_at: now + Duration::days(SESSION_ABSOLUTE_TTL_DAYS),
        },
    )
    .await?;

    issue_tokens(state, user, session_id, refresh_token, expires_at, now)
}

fn issue_tokens(
    state: &AppState,
    user: UserRow,
    session_id: Uuid,
    refresh_token: String,
    refresh_expires_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<Authenticated, ApiError> {
    let (access_token, access_expires_at) = state
        .tokens
        .issue_access(user.id, user.role, session_id, now)?;
    Ok(Authenticated {
        user,
        access_token,
        access_expires_at,
        refresh_token,
        refresh_expires_at,
    })
}

/// Rotates the refresh token. Every successful call retires the presented token and hands
/// out a new one in the same family. A retired token coming back after the grace window
/// means it was copied, so the entire family is revoked and both parties must sign in again.
pub async fn refresh(
    state: &AppState,
    raw_token: &str,
    ctx: &ClientContext,
) -> Result<Authenticated, RefreshFailure> {
    let hash = tokens::hash_opaque_token(raw_token);
    let session = repo::find_session_by_token_hash(&state.db, &hash)
        .await?
        .ok_or(RefreshFailure::Dead)?;
    let now = Utc::now();

    if session.revoked_at.is_some()
        || session.expires_at <= now
        || session.absolute_expires_at <= now
    {
        return Err(RefreshFailure::Dead);
    }
    if let Some(rotated_at) = session.rotated_at {
        if now - rotated_at <= Duration::seconds(ROTATION_GRACE_SECS) {
            return Err(RefreshFailure::Conflict);
        }
        tracing::warn!(
            user_id = %session.user_id,
            family_id = %session.family_id,
            "refresh token reuse detected; revoking the session family"
        );
        repo::revoke_family(&state.db, session.family_id).await?;
        return Err(RefreshFailure::Dead);
    }

    let user = repo::find_user_by_id(&state.db, session.user_id)
        .await?
        .filter(|u| u.email_verified_at.is_some());
    let Some(user) = user else {
        repo::revoke_family(&state.db, session.family_id).await?;
        return Err(RefreshFailure::Dead);
    };

    let new_token = tokens::generate_opaque_token();
    let new_hash = tokens::hash_opaque_token(&new_token);
    let expires_at = (now + Duration::days(REFRESH_TTL_DAYS)).min(session.absolute_expires_at);
    let user_agent: Option<String> = ctx
        .user_agent
        .as_ref()
        .map(|ua| ua.chars().take(MAX_USER_AGENT_CHARS).collect());

    let mut tx = state.db.begin().await?;
    if !repo::mark_session_rotated(&mut *tx, session.id).await? {
        return Err(RefreshFailure::Conflict);
    }
    let new_id = repo::insert_session(
        &mut *tx,
        &NewSession {
            user_id: session.user_id,
            family_id: session.family_id,
            token_hash: &new_hash,
            user_agent: user_agent.as_deref(),
            ip: ctx.ip.map(IpNetwork::from),
            expires_at,
            absolute_expires_at: session.absolute_expires_at,
        },
    )
    .await?;
    tx.commit().await?;

    issue_tokens(state, user, new_id, new_token, expires_at, now).map_err(RefreshFailure::Other)
}

/// Ends the whole session family. Access tokens already issued stay valid until they expire
/// (at most 15 minutes), which is the price of not hitting the database on every request.
pub async fn logout(state: &AppState, raw_refresh_token: Option<&str>) -> Result<(), ApiError> {
    if let Some(raw) = raw_refresh_token {
        let hash = tokens::hash_opaque_token(raw);
        if let Some(session) = repo::find_session_by_token_hash(&state.db, &hash).await? {
            repo::revoke_family(&state.db, session.family_id).await?;
        }
    }
    Ok(())
}

pub async fn forgot_password(state: &AppState, email: &str) -> Result<(), ApiError> {
    let Some(user) = repo::find_user_by_email(&state.db, email).await? else {
        return Ok(());
    };
    let Some(to) = &user.email else {
        return Ok(());
    };
    let raw = issue_token(
        state,
        user.id,
        TokenPurpose::PasswordReset,
        Duration::hours(RESET_TTL_HOURS),
    )
    .await?;
    state.send_email(emails::password_reset(
        to,
        &link(state, "reset-password", &raw),
    ));
    Ok(())
}

pub async fn reset_password(
    state: &AppState,
    raw_token: &str,
    new_password: String,
) -> Result<(), ApiError> {
    let token_hash = tokens::hash_opaque_token(raw_token);
    // Reject bad tokens before paying for a password hash.
    if !repo::auth_token_is_usable(&state.db, &token_hash, TokenPurpose::PasswordReset).await? {
        return Err(invalid_token());
    }
    let new_hash = state.passwords.hash(new_password).await?;

    let mut tx = state.db.begin().await?;
    let user_id = repo::consume_auth_token(&mut *tx, &token_hash, TokenPurpose::PasswordReset)
        .await?
        .ok_or_else(invalid_token)?;
    repo::update_password(&mut *tx, user_id, &new_hash).await?;
    repo::revoke_user_sessions(&mut *tx, user_id).await?;
    tx.commit().await?;

    if let Some(user) = repo::find_user_by_id(&state.db, user_id).await? {
        if let Some(to) = &user.email {
            state.send_email(emails::password_changed(to));
        }
    }
    Ok(())
}

pub async fn me(state: &AppState, user_id: Uuid) -> Result<UserRow, ApiError> {
    repo::find_user_by_id(&state.db, user_id)
        .await?
        .ok_or_else(|| ApiError::unauthorized("unauthenticated", "Sign in to continue."))
}
