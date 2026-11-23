use axum::http::StatusCode;
use chrono::Utc;
use uuid::Uuid;
use zeno_core::model::Role;

use crate::auth::extractor::AuthUser;
use crate::comments::dto::{CommentResponse, CreateCommentRequest, NewComment};
use crate::comments::repo::{self, CommentRow, NewRow};
use crate::cursor::{paginate, TimeCursor};
use crate::error::ApiError;
use crate::net::limit_bucket;
use crate::ratelimit::{enforce, enforce_user, policies};
use crate::recordings::repo as recordings;
use crate::shares::repo::Target;
use crate::state::AppState;

const ROLES: &[Role] = &[Role::Creator, Role::Admin];
/// How many one recording can hold, and how many of them one person can leave: so that neither a long
/// thread nor one account fills a page nobody can read.
const MAX_PER_RECORDING: i64 = 2000;
const MAX_PER_AUTHOR: i64 = 200;

fn conflict(code: &'static str, message: &'static str) -> ApiError {
    ApiError::Http {
        status: StatusCode::CONFLICT,
        code,
        message: message.into(),
    }
}

fn not_found(what: &'static str) -> ApiError {
    ApiError::Http {
        status: StatusCode::NOT_FOUND,
        code: "not_found",
        message: format!("{what} not found."),
    }
}

fn comment_id_conflict() -> ApiError {
    conflict(
        "comment_id_conflict",
        "This comment id is already used by something else.",
    )
}

/// A page of the conversation, and whether it is open.
pub struct Page {
    pub comments: Vec<CommentResponse>,
    pub next_cursor: Option<String>,
    pub comments_enabled: bool,
}

/// Whether the viewer may be shown anything about the recording behind `target`. A link with no password
/// shows its comments to anyone who has the link. A link with one shows them only to a viewing that
/// unlocked it: the token of such a viewing is the proof, and it is checked against the database every time
/// (a viewing that has ended, or was never of this link, proves nothing).
async fn may_see(state: &AppState, target: &Target, token: Option<&str>) -> Result<(), ApiError> {
    if target.password_hash.is_none() {
        return Ok(());
    }
    let denied = || ApiError::forbidden("password_required", "This link needs a password.");
    let claims = token
        .and_then(|token| state.config.playback.verify(token, Utc::now()).ok())
        .ok_or_else(denied)?;
    if repo::is_open_viewing_of_link(&state.db, claims.session_id, target.link_id).await? {
        Ok(())
    } else {
        Err(denied())
    }
}

/// What a person watching through `code` is shown.
pub async fn list_for_link(
    state: &AppState,
    ip: Option<std::net::IpAddr>,
    code: &str,
    viewer: Option<&AuthUser>,
    token: Option<&str>,
    cursor: Option<TimeCursor>,
    limit: usize,
) -> Result<Page, ApiError> {
    enforce(state, &policies::COMMENT_READ, &limit_bucket(ip)).await?;
    let target = crate::shares::service::live_target(state, code).await?;
    may_see(state, &target, token).await?;
    page(
        state,
        target.recording_id,
        viewer.map(|v| v.id),
        cursor,
        limit,
    )
    .await
}

async fn page(
    state: &AppState,
    recording_id: Uuid,
    viewer: Option<Uuid>,
    cursor: Option<TimeCursor>,
    limit: usize,
) -> Result<Page, ApiError> {
    let talk = repo::conversation(&state.db, recording_id)
        .await?
        .ok_or_else(|| not_found("Recording"))?;
    if !talk.comments_enabled {
        return Ok(Page {
            comments: Vec::new(),
            next_cursor: None,
            comments_enabled: false,
        });
    }
    let rows = repo::list(&state.db, recording_id, cursor, limit as i64 + 1).await?;
    let (rows, next) = paginate(rows, limit, |row: &CommentRow| TimeCursor {
        created_at: row.created_at,
        id: row.id,
    });
    Ok(Page {
        comments: rows
            .into_iter()
            .map(|row| CommentResponse::of(row, viewer, talk.owner_id))
            .collect(),
        next_cursor: next.map(TimeCursor::encode),
        comments_enabled: true,
    })
}

/// The owner's view of their own recording's conversation, whether or not it is open to others.
pub async fn list_for_owner(
    state: &AppState,
    user: &AuthUser,
    recording_id: Uuid,
    cursor: Option<TimeCursor>,
    limit: usize,
) -> Result<Page, ApiError> {
    user.require_role(ROLES)?;
    enforce_user(state, &policies::COMMENT_READ, user.id).await?;
    let recording = recordings::find_owned(&state.db, recording_id, user.id)
        .await?
        .ok_or_else(|| not_found("Recording"))?;
    let rows = repo::list(&state.db, recording.id, cursor, limit as i64 + 1).await?;
    let (rows, next) = paginate(rows, limit, |row: &CommentRow| TimeCursor {
        created_at: row.created_at,
        id: row.id,
    });
    Ok(Page {
        comments: rows
            .into_iter()
            .map(|row| CommentResponse::of(row, Some(user.id), recording.owner_id))
            .collect(),
        next_cursor: next.map(TimeCursor::encode),
        comments_enabled: recording.comments_enabled,
    })
}

/// Leaves a comment or a reaction on the recording behind `code`. Retrying with the same id and the same
/// content returns the one that was left.
pub async fn create(
    state: &AppState,
    user: &AuthUser,
    code: &str,
    token: Option<&str>,
    input: CreateCommentRequest,
) -> Result<CommentResponse, ApiError> {
    enforce_user(state, &policies::COMMENT_WRITE, user.id).await?;
    let target = crate::shares::service::live_target(state, code).await?;
    may_see(state, &target, token).await?;
    let talk = repo::conversation(&state.db, target.recording_id)
        .await?
        .ok_or_else(|| not_found("Recording"))?;
    if !talk.comments_enabled {
        return Err(conflict(
            "comments_off",
            "The owner has turned comments off for this video.",
        ));
    }
    let duration_ms = target.duration_seconds.map(|s| i64::from(s) * 1000);
    let new = input.validate(duration_ms)?;

    if let Some(id) = new.id {
        if let Some(existing) = repo::find(&state.db, id).await? {
            return replay(existing, &new, target.recording_id, user, talk.owner_id);
        }
    }
    if repo::count_on(&state.db, target.recording_id).await? >= MAX_PER_RECORDING {
        return Err(conflict(
            "comments_full",
            "This video has as many comments as it can hold.",
        ));
    }
    if repo::count_by(&state.db, target.recording_id, user.id).await? >= MAX_PER_AUTHOR {
        return Err(conflict(
            "too_many_comments",
            "You have left as many comments on this video as one person can.",
        ));
    }

    let id = new.id.unwrap_or_else(Uuid::new_v4);
    let inserted = repo::insert(
        &state.db,
        &NewRow {
            id,
            recording_id: target.recording_id,
            author_id: user.id,
            kind: new.kind,
            body: new.body.as_deref(),
            emoji: new.emoji,
            at_ms: new.at_ms,
        },
    )
    .await;
    if let Err(err) = inserted {
        // Two posts with one id at once: the second finds the first.
        if matches!(&err, sqlx::Error::Database(db) if db.is_unique_violation()) {
            if let Some(existing) = repo::find(&state.db, id).await? {
                return replay(existing, &new, target.recording_id, user, talk.owner_id);
            }
            return Err(comment_id_conflict());
        }
        return Err(err.into());
    }
    let row = repo::find(&state.db, id)
        .await?
        .ok_or_else(|| not_found("Comment"))?;
    Ok(CommentResponse::of(row, Some(user.id), talk.owner_id))
}

/// The same post, again: the same answer. A different one under the same id is refused.
fn replay(
    existing: CommentRow,
    new: &NewComment,
    recording_id: Uuid,
    user: &AuthUser,
    owner: Uuid,
) -> Result<CommentResponse, ApiError> {
    let same = existing.recording_id == recording_id
        && existing.author_id == user.id
        && existing.kind == new.kind
        && existing.body == new.body
        && existing.emoji.as_deref() == new.emoji
        && existing.at_ms == new.at_ms;
    if same {
        Ok(CommentResponse::of(existing, Some(user.id), owner))
    } else {
        Err(comment_id_conflict())
    }
}

/// Removes a comment: its author may, and so may the owner of the recording it is on. For anyone else it
/// does not exist.
pub async fn delete(state: &AppState, user: &AuthUser, id: Uuid) -> Result<(), ApiError> {
    enforce_user(state, &policies::COMMENT_WRITE, user.id).await?;
    let row = repo::find(&state.db, id)
        .await?
        .ok_or_else(|| not_found("Comment"))?;
    let owner = repo::conversation(&state.db, row.recording_id)
        .await?
        .map(|talk| talk.owner_id);
    if row.author_id != user.id && owner != Some(user.id) {
        return Err(not_found("Comment"));
    }
    repo::delete(&state.db, id).await?;
    Ok(())
}
