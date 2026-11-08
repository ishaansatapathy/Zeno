use axum::http::StatusCode;

use crate::catalog::dto::{CourseCard, CourseDetail, Filter, PublicLesson};
use crate::catalog::repo::{self, CardRow};
use crate::error::ApiError;
use crate::profiles::dto::valid_handle;
use crate::profiles::repo as profiles;
use crate::state::AppState;

fn not_found(subject: &'static str) -> ApiError {
    ApiError::Http {
        status: StatusCode::NOT_FOUND,
        code: "not_found",
        message: format!("{subject} not found."),
    }
}

/// Slugs the API creates are lowercase words joined by hyphens; anything else cannot exist, so
/// it is answered without touching the database.
fn plausible_slug(slug: &str) -> bool {
    (3..=140).contains(&slug.len())
        && slug
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

pub async fn course(state: &AppState, slug: &str) -> Result<CourseDetail, ApiError> {
    if !plausible_slug(slug) {
        return Err(not_found("Course"));
    }
    let detail = repo::find_by_slug(&state.db, slug)
        .await?
        .ok_or_else(|| not_found("Course"))?;
    let lessons = repo::lessons(&state.db, detail.card.id).await?;
    Ok(CourseDetail {
        card: CourseCard::build(state, &detail.card),
        description: detail.description,
        lessons: lessons.iter().map(PublicLesson::from).collect(),
    })
}

/// One page of results and whether another page follows.
pub async fn search(state: &AppState, filter: &Filter) -> Result<(Vec<CardRow>, bool), ApiError> {
    let mut rows = repo::search(&state.db, filter, i64::from(filter.limit) + 1).await?;
    let has_more = rows.len() > filter.limit as usize;
    rows.truncate(filter.limit as usize);
    Ok((rows, has_more))
}

pub async fn creator(
    state: &AppState,
    handle: &str,
) -> Result<(crate::profiles::repo::ProfileRow, Vec<CardRow>), ApiError> {
    if !valid_handle(handle) {
        return Err(not_found("Creator"));
    }
    let profile = profiles::find_by_handle(&state.db, handle)
        .await?
        .ok_or_else(|| not_found("Creator"))?;
    let filter = Filter {
        creator_id: Some(profile.user_id),
        limit: 24,
        ..Filter::default()
    };
    let (courses, _) = search(state, &filter).await?;
    Ok((profile, courses))
}
