use axum::http::StatusCode;
use zeno_core::model::Role;

use crate::auth::extractor::AuthUser;
use crate::error::{ApiError, FieldError};
use crate::images::{self, ImageKind};
use crate::profiles::dto::ProfileInput;
use crate::profiles::repo::{self, ProfileRow};
use crate::state::AppState;

const ROLES: &[Role] = &[Role::Creator, Role::Admin];

fn conflict(code: &'static str, message: &'static str) -> ApiError {
    ApiError::Http {
        status: StatusCode::CONFLICT,
        code,
        message: message.into(),
    }
}

fn profile_required() -> ApiError {
    conflict(
        "profile_required",
        "Create your profile before adding a picture.",
    )
}

pub async fn get(state: &AppState, user: &AuthUser) -> Result<Option<ProfileRow>, ApiError> {
    user.require_role(ROLES)?;
    Ok(repo::find(&state.db, user.id).await?)
}

pub async fn put(
    state: &AppState,
    user: &AuthUser,
    input: ProfileInput,
) -> Result<ProfileRow, ApiError> {
    user.require_role(ROLES)?;
    if let Some(current) = repo::find(&state.db, user.id).await? {
        if current.handle != input.handle {
            return Err(conflict(
                "handle_locked",
                "A handle cannot be changed once it is set, because shared links use it.",
            ));
        }
    }
    match repo::upsert(&state.db, user.id, &input).await {
        Ok(row) => Ok(row),
        Err(sqlx::Error::Database(db)) if db.is_unique_violation() => Err(ApiError::validation(
            vec![FieldError::new("handle", "That handle is already taken.")],
        )),
        Err(err) => Err(err.into()),
    }
}

pub async fn set_avatar(
    state: &AppState,
    user: &AuthUser,
    bytes: Vec<u8>,
) -> Result<ProfileRow, ApiError> {
    user.require_role(ROLES)?;
    if repo::find(&state.db, user.id).await?.is_none() {
        return Err(profile_required());
    }
    let processed = images::process_async(bytes, ImageKind::Avatar).await?;
    let key = images::store(state, ImageKind::Avatar, user.id, processed).await?;
    let mut tx = state.db.begin().await?;
    let Some(old) = repo::lock_avatar(&mut tx, user.id).await? else {
        drop(tx);
        images::discard(state, &key).await;
        return Err(profile_required());
    };
    let row = repo::set_avatar(&mut tx, user.id, Some(&key)).await?;
    tx.commit().await?;
    // The same picture uploaded twice has the same key; only a different old one is removed.
    if let Some(old) = old.filter(|old| *old != key) {
        images::discard(state, &old).await;
    }
    Ok(row)
}

pub async fn clear_avatar(state: &AppState, user: &AuthUser) -> Result<ProfileRow, ApiError> {
    user.require_role(ROLES)?;
    let mut tx = state.db.begin().await?;
    let Some(old) = repo::lock_avatar(&mut tx, user.id).await? else {
        return Err(profile_required());
    };
    let row = repo::set_avatar(&mut tx, user.id, None).await?;
    tx.commit().await?;
    if let Some(old) = old {
        images::discard(state, &old).await;
    }
    Ok(row)
}
