use std::net::IpAddr;

use chrono::{DateTime, Duration, Utc};
use sqlx::types::ipnetwork::IpNetwork;
use uuid::Uuid;
use zeno_core::model::Role;

use crate::auth::audit::{self, Event, Kind};
use crate::auth::repo::{self, NewSession, TokenPurpose, UserRow};
use crate::auth::tokens::AccessClaims;
use crate::auth::{emails, limits, revocation, tokens};
use crate::error::ApiError;
use crate::outbox;
use crate::secret::Secret;
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

#[derive(Clone)]
pub struct ClientContext {
    pub ip: Option<IpAddr>,
    pub user_agent: Option<String>,
}

#[derive(Debug)]
pub struct RegisterInput {
    pub name: String,
    pub email: String,
    pub password: Secret,
    pub role: Role,
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

/// Revokes in Postgres (the source of truth) and cuts off access tokens already in flight.
async fn revoke_family(state: &AppState, family_id: Uuid) -> Result<(), sqlx::Error> {
    repo::revoke_family(&state.db, family_id).await?;
    revocation::mark_revoked(&*state.counters, family_id).await;
    Ok(())
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
    outbox::enqueue(
        state,
        emails::verification(to, &link(state, "verify-email", &raw)),
        Duration::hours(VERIFY_TTL_HOURS),
    )
    .await?;
    Ok(())
}

/// Runs `job` after the response has been sent. Anything whose duration depends on whether
/// an account exists (token writes, mail) goes through here, so response time can't be used
/// to discover registered emails.
pub fn defer<F, Fut>(state: &AppState, what: &'static str, job: F)
where
    F: FnOnce(AppState) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<(), ApiError>> + Send + 'static,
{
    let owned = state.clone();
    state.background.spawn(async move {
        if let Err(err) = job(owned).await {
            tracing::error!(error = ?err, job = what, "background job failed");
        }
    });
}

async fn notify_existing_account(state: &AppState, email: &str) -> Result<(), ApiError> {
    let Some(existing) = repo::find_user_by_email(&state.db, email).await? else {
        return Ok(());
    };
    if existing.email_verified_at.is_none() {
        send_verification(state, &existing).await
    } else if let Some(to) = &existing.email {
        let origin = &state.config.client_origin;
        outbox::enqueue(
            state,
            emails::already_registered(
                to,
                &format!("{origin}/login"),
                &format!("{origin}/forgot-password"),
            ),
            Duration::days(1),
        )
        .await?;
        Ok(())
    } else {
        Ok(())
    }
}

/// Both outcomes (new address, taken address) cost the same inline: one hash and one
/// insert. Everything that differs happens in the background, and the caller always gives
/// the same answer, so sign-up can't be used to discover who has an account.
pub async fn register(
    state: &AppState,
    input: RegisterInput,
    ctx: &ClientContext,
) -> Result<(), ApiError> {
    let hash = state.passwords.hash(input.password.into_inner()).await?;
    let inserted =
        repo::insert_user(&state.db, &input.name, &input.email, &hash, input.role).await?;
    let email = input.email;
    if let Some(user) = &inserted {
        audit::record(
            state,
            Event::new(Kind::SignedUp, ctx).user(user.id).email(&email),
        );
    }
    defer(state, "register follow-up", move |state| async move {
        match inserted {
            Some(user) => send_verification(&state, &user).await,
            None => notify_existing_account(&state, &email).await,
        }
    });
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
    crate::enrollments::repo::accept_invites_for_user(&mut tx, user_id).await?;
    tx.commit().await?;

    let user = repo::find_user_by_id(&state.db, user_id)
        .await?
        .ok_or_else(invalid_token)?;
    audit::record(state, Event::new(Kind::EmailVerified, ctx).user(user.id));
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
    if let Err(err) = limits::ensure_login_allowed(&*state.counters, &email, ctx.ip).await {
        audit::record(state, Event::new(Kind::LoginBlocked, ctx).email(&email));
        return Err(err);
    }

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

    let known_user = user.as_ref().map(|u| u.id);
    let Some(user) = user.filter(|_| valid) else {
        let counter_result = limits::record_login_failure(&*state.counters, &email, ctx.ip).await;
        audit::record(
            state,
            Event::new(Kind::LoginFailed, ctx)
                .maybe_user(known_user)
                .email(&email),
        );
        counter_result?;
        return Err(ApiError::unauthorized(
            "invalid_credentials",
            "Invalid email or password.",
        ));
    };
    if user.email_verified_at.is_none() {
        audit::record(state, Event::new(Kind::LoginUnverified, ctx).user(user.id));
        return Err(ApiError::forbidden(
            "email_not_verified",
            "Please verify your email before signing in.",
        ));
    }

    limits::clear_login_failures(&*state.counters, &email, ctx.ip).await;
    audit::record(state, Event::new(Kind::LoginSucceeded, ctx).user(user.id));
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

    let family_id = Uuid::new_v4();
    let session_id = repo::insert_session(
        &state.db,
        &NewSession {
            user_id: user.id,
            family_id,
            token_hash: &token_hash,
            user_agent: user_agent.as_deref(),
            ip: ctx.ip.map(IpNetwork::from),
            expires_at,
            absolute_expires_at: now + Duration::days(SESSION_ABSOLUTE_TTL_DAYS),
        },
    )
    .await?;

    issue_tokens(
        state,
        user,
        session_id,
        family_id,
        refresh_token,
        expires_at,
        now,
    )
}

fn issue_tokens(
    state: &AppState,
    user: UserRow,
    session_id: Uuid,
    family_id: Uuid,
    refresh_token: String,
    refresh_expires_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<Authenticated, ApiError> {
    let (access_token, access_expires_at) = state
        .tokens
        .issue_access(user.id, user.role, session_id, family_id, now)?;
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
        revoke_family(state, session.family_id).await?;
        audit::record(
            state,
            Event::new(Kind::RefreshReuseDetected, ctx)
                .user(session.user_id)
                .family(session.family_id),
        );
        return Err(RefreshFailure::Dead);
    }

    let user = repo::find_user_by_id(&state.db, session.user_id)
        .await?
        .filter(|u| u.email_verified_at.is_some());
    let Some(user) = user else {
        revoke_family(state, session.family_id).await?;
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

    issue_tokens(
        state,
        user,
        new_id,
        session.family_id,
        new_token,
        expires_at,
        now,
    )
    .map_err(RefreshFailure::Other)
}

/// Ends the whole session family, identified by the refresh cookie and/or the family in a
/// still-valid access token (so logout works even if the refresh cookie went missing).
/// Access tokens already issued are cut off immediately through the revocation marker.
pub async fn logout(
    state: &AppState,
    raw_refresh_token: Option<&str>,
    access: Option<&AccessClaims>,
    ctx: &ClientContext,
) -> Result<(), ApiError> {
    let mut families = Vec::new();
    let mut user_id = access.map(|claims| claims.sub);
    if let Some(raw) = raw_refresh_token {
        let hash = tokens::hash_opaque_token(raw);
        if let Some(session) = repo::find_session_by_token_hash(&state.db, &hash).await? {
            families.push(session.family_id);
            user_id = user_id.or(Some(session.user_id));
        }
    }
    families.extend(access.map(|claims| claims.fid));
    families.sort_unstable();
    families.dedup();
    for family_id in &families {
        revoke_family(state, *family_id).await?;
    }
    if let Some(user_id) = user_id {
        audit::record(state, Event::new(Kind::LoggedOut, ctx).user(user_id));
    }
    Ok(())
}

pub async fn forgot_password(
    state: &AppState,
    email: &str,
    ctx: &ClientContext,
) -> Result<(), ApiError> {
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
    audit::record(
        state,
        Event::new(Kind::PasswordResetRequested, ctx).user(user.id),
    );
    outbox::enqueue(
        state,
        emails::password_reset(to, &link(state, "reset-password", &raw)),
        Duration::hours(RESET_TTL_HOURS),
    )
    .await?;
    Ok(())
}

pub async fn reset_password(
    state: &AppState,
    raw_token: &str,
    new_password: String,
    ctx: &ClientContext,
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
    let families = repo::revoke_user_sessions(&mut *tx, user_id).await?;
    // Whoever had the password may be watching too: their playback stops with the next request.
    crate::playback::repo::end_all_for_user(&mut *tx, user_id, "password_reset").await?;
    tx.commit().await?;
    for family_id in families {
        revocation::mark_revoked(&*state.counters, family_id).await;
    }
    audit::record(
        state,
        Event::new(Kind::PasswordResetCompleted, ctx).user(user_id),
    );

    // The password is already changed; failing to queue the courtesy notice must not
    // turn that success into an error.
    if let Some(user) = repo::find_user_by_id(&state.db, user_id).await? {
        if let Some(to) = &user.email {
            if let Err(err) =
                outbox::enqueue(state, emails::password_changed(to), Duration::days(1)).await
            {
                tracing::error!(error = ?err, "could not queue password-changed notice");
            }
        }
    }
    Ok(())
}

pub async fn me(state: &AppState, user_id: Uuid) -> Result<UserRow, ApiError> {
    repo::find_user_by_id(&state.db, user_id)
        .await?
        .ok_or_else(|| ApiError::unauthorized("unauthenticated", "Sign in to continue."))
}
