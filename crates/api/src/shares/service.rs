use axum::http::StatusCode;
use chrono::Utc;
use sqlx::Acquire;
use uuid::Uuid;
use zeno_core::model::Role;

use crate::auth::extractor::AuthUser;
use crate::auth::service::ClientContext;
use crate::error::ApiError;
use crate::net::limit_bucket;
use crate::playback::{self, service::Started};
use crate::ratelimit::{enforce, enforce_user, policies};
use crate::recordings::repo as recordings;
use crate::shares::code;
use crate::shares::dto::{playable, NewLink, ShareView, MAX_LIVE_LINKS};
use crate::shares::repo::{self, LinkRow, Target};
use crate::state::AppState;
use crate::videos;

const ROLES: &[Role] = &[Role::Creator, Role::Admin];
/// How many of a recording's links the owner's list shows, live or not.
const LIST_LIMIT: i64 = 100;
/// Tries to find an unused code. One is enough with 128 random bits; the loop is for honesty.
const CODE_ATTEMPTS: usize = 3;

fn recording_not_found() -> ApiError {
    ApiError::Http {
        status: StatusCode::NOT_FOUND,
        code: "not_found",
        message: "Recording not found.".into(),
    }
}

fn link_not_found() -> ApiError {
    ApiError::Http {
        status: StatusCode::NOT_FOUND,
        code: "not_found",
        message: "Link not found.".into(),
    }
}

/// A link that was good and no longer is: revoked, expired, switched off or its recording gone.
/// `410` rather than `404`, so a visitor holding the link is told it has been closed. The code is
/// 128 random bits, so knowing it exists tells a stranger nothing.
fn unavailable() -> ApiError {
    ApiError::Http {
        status: StatusCode::GONE,
        code: "link_unavailable",
        message: "This link is no longer available.".into(),
    }
}

fn conflict(code: &'static str, message: &'static str) -> ApiError {
    ApiError::Http {
        status: StatusCode::CONFLICT,
        code,
        message: message.into(),
    }
}

fn too_many_links() -> ApiError {
    conflict(
        "too_many_links",
        "This recording already has the most links it can have. Revoke one first.",
    )
}

fn unique_violation(err: &sqlx::Error) -> bool {
    matches!(err, sqlx::Error::Database(db) if db.is_unique_violation())
}

// ---- the owner's side -------------------------------------------------------------------

/// Makes a link for one of the caller's recordings. A private recording is switched to `link`
/// in the same transaction: a link to something nobody may see would be a trap.
pub async fn create(
    state: &AppState,
    user: &AuthUser,
    recording_id: Uuid,
    input: NewLink,
) -> Result<(LinkRow, String), ApiError> {
    user.require_role(ROLES)?;
    enforce_user(state, &policies::SHARE_WRITE, user.id).await?;
    let recording = recordings::find_owned(&state.db, recording_id, user.id)
        .await?
        .ok_or_else(recording_not_found)?;
    if let Some(id) = input.id {
        if let Some(link) = repo::find_by_id(&state.db, id).await? {
            return replay_link(state, &input, recording_id, &recording.visibility, link).await;
        }
    }
    if repo::count_live(&state.db, recording_id).await? >= MAX_LIVE_LINKS {
        return Err(too_many_links());
    }
    // The hash is the expensive part, so a request that is going to be refused anyway is refused
    // before it. This count is not the check that holds: two requests can both pass it. The one
    // inside the transaction below does.
    let hash = match &input.password {
        Some(password) => Some(state.passwords.hash(password.clone()).await?),
        None => None,
    };

    let mut tx = state.db.begin().await?;
    // One creator at a time per recording: a count and an insert are only a cap if nobody else can
    // insert between them.
    if !recordings::lock(&mut tx, recording_id).await? {
        return Err(recording_not_found());
    }
    if let Some(id) = input.id {
        if let Some(link) = repo::find_by_id(&mut *tx, id).await? {
            let current = recordings::find_owned(&mut *tx, recording_id, user.id)
                .await?
                .ok_or_else(recording_not_found)?;
            return replay_link(state, &input, recording_id, &current.visibility, link).await;
        }
    }
    if repo::count_live(&mut *tx, recording_id).await? >= MAX_LIVE_LINKS {
        return Err(too_many_links());
    }
    let mut link = None;
    for _ in 0..CODE_ATTEMPTS {
        // A failed insert aborts the transaction, so each attempt gets a savepoint.
        let mut attempt = tx.begin().await?;
        match repo::insert(
            &mut attempt,
            input.id.unwrap_or_else(Uuid::new_v4),
            &code::generate(),
            recording_id,
            hash.as_deref(),
            input.expires_at,
        )
        .await
        {
            Ok(row) => {
                attempt.commit().await?;
                link = Some(row);
                break;
            }
            Err(err) if unique_violation(&err) => {
                attempt.rollback().await?;
                if let Some(id) = input.id {
                    if repo::find_by_id(&mut *tx, id).await?.is_some() {
                        return Err(link_id_conflict());
                    }
                }
            }
            Err(err) => return Err(err.into()),
        }
    }
    let link = link.ok_or_else(|| ApiError::Internal(anyhow::anyhow!("no free share code")))?;
    let visibility = if recording.visibility == "private" {
        recordings::set_visibility(&mut *tx, recording_id, user.id, "link").await?;
        "link".to_owned()
    } else {
        recording.visibility
    };
    tx.commit().await?;
    Ok((link, visibility))
}

fn link_id_conflict() -> ApiError {
    ApiError::Http {
        status: axum::http::StatusCode::CONFLICT,
        code: "link_id_conflict",
        message: "This link id is already used by another or closed link.".into(),
    }
}

async fn replay_link(
    state: &AppState,
    input: &NewLink,
    recording_id: Uuid,
    visibility: &str,
    link: LinkRow,
) -> Result<(LinkRow, String), ApiError> {
    if link.recording_id != recording_id
        || link.revoked_at.is_some()
        || link.expires_at.is_some_and(|at| at <= Utc::now())
        || link.expires_at != input.expires_at
    {
        return Err(link_id_conflict());
    }
    let same_password = match (&input.password, &link.password_hash) {
        (None, None) => true,
        (Some(password), Some(hash)) => {
            state
                .passwords
                .verify(password.clone(), Some(hash.clone()))
                .await?
        }
        _ => false,
    };
    if !same_password {
        return Err(link_id_conflict());
    }
    Ok((link, visibility.to_owned()))
}

pub async fn list(
    state: &AppState,
    user: &AuthUser,
    recording_id: Uuid,
) -> Result<Vec<LinkRow>, ApiError> {
    user.require_role(ROLES)?;
    recordings::find_owned(&state.db, recording_id, user.id)
        .await?
        .ok_or_else(recording_not_found)?;
    Ok(repo::list_for_recording(&state.db, recording_id, LIST_LIMIT).await?)
}

/// Revokes a link and ends every viewing under way through it. Safe to repeat.
pub async fn revoke(
    state: &AppState,
    user: &AuthUser,
    recording_id: Uuid,
    link_id: Uuid,
) -> Result<(), ApiError> {
    user.require_role(ROLES)?;
    enforce_user(state, &policies::SHARE_WRITE, user.id).await?;
    recordings::find_owned(&state.db, recording_id, user.id)
        .await?
        .ok_or_else(recording_not_found)?;
    let mut tx = state.db.begin().await?;
    if !repo::revoke(&mut *tx, link_id, recording_id).await? {
        return Err(link_not_found());
    }
    playback::repo::end_for_link(&mut *tx, link_id, "access_revoked").await?;
    tx.commit().await?;
    Ok(())
}

// ---- the visitor's side -----------------------------------------------------------------

/// The link behind `code`, if it is usable. `404` for a code that never existed (including one
/// that is not even the right shape: no database call for those), `410` for one that has closed.
pub(crate) async fn live_target(state: &AppState, raw_code: &str) -> Result<Target, ApiError> {
    if !code::is_well_formed(raw_code) {
        return Err(link_not_found());
    }
    let target = repo::find_by_code(&state.db, raw_code)
        .await?
        .ok_or_else(link_not_found)?;
    if !target.is_live(Utc::now()) {
        return Err(unavailable());
    }
    Ok(target)
}

/// What the page needs to draw itself. Public, unauthenticated, cacheable.
pub async fn resolve(state: &AppState, raw_code: &str) -> Result<ShareView, ApiError> {
    let target = live_target(state, raw_code).await?;
    Ok(ShareView::of(raw_code, &target))
}

/// Starts watching through a link: checks the password if there is one, then opens a session.
pub async fn play(
    state: &AppState,
    raw_code: &str,
    password: Option<String>,
    ctx: &ClientContext,
) -> Result<Started, ApiError> {
    let target = live_target(state, raw_code).await?;
    if let Some(hash) = &target.password_hash {
        // Counted before the hash is computed: each guess costs real CPU, and an attacker
        // should not get to spend ours.
        let subject = format!("{}:{}", target.link_id, limit_bucket(ctx.ip));
        enforce(state, &policies::SHARE_PASSWORD, &subject).await?;
        let Some(password) = password else {
            return Err(ApiError::forbidden(
                "password_required",
                "This link needs a password.",
            ));
        };
        // An absurdly long password cannot be right; do not feed it to the hasher.
        let right = password.chars().count() <= crate::shares::dto::MAX_PASSWORD
            && state.passwords.verify(password, Some(hash.clone())).await?;
        if !right {
            return Err(ApiError::forbidden(
                "wrong_password",
                "That password is not right.",
            ));
        }
    }
    if !playable(target.video_status) {
        return Err(conflict(
            "not_ready",
            "This video is not ready to play yet.",
        ));
    }
    let video = match target.video_id {
        Some(id) => videos::repo::find(&state.db, id).await?,
        None => None,
    }
    .ok_or_else(|| conflict("not_ready", "This video is not ready to play yet."))?;
    // `None`: the link was closed after we read it and before the viewing could start.
    let started = playback::service::start_link(state, target.link_id, video, ctx)
        .await?
        .ok_or_else(unavailable)?;
    // Counted only once a session really started, and off the request: a counter that is down
    // must not stop anyone watching.
    crate::views::record(
        state,
        target.recording_id,
        crate::views::viewer_id(
            &state.config.jwt_access_secret,
            ctx.ip,
            ctx.user_agent.as_deref(),
        ),
    );
    Ok(started)
}
