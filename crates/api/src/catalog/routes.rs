use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use serde::Serialize;

use crate::catalog::dto::{
    creator_page, ApiQuery, CatalogPage, CatalogQuery, CourseCard, CourseDetailEnvelope,
};
use crate::catalog::service;
use crate::error::ApiError;
use crate::state::AppState;

/// Visitors and CDNs may reuse a response for a minute and keep serving it for five more while
/// a fresh one is fetched.
const CACHE_CONTROL: &str = "public, max-age=60, stale-while-revalidate=300";

/// Mounted under `/v1/public`. No route here reads cookies or identifies the caller.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/courses", get(list))
        .route("/courses/:slug", get(course))
        .route("/creators/:handle", get(creator))
}

/// JSON with a content-derived `ETag` and the catalogue's cache lifetime.
fn cacheable<T: Serialize>(request: &HeaderMap, value: &T) -> Response {
    crate::httpcache::json_with_etag(request, value, CACHE_CONTROL)
}

async fn list(
    State(state): State<AppState>,
    request: HeaderMap,
    ApiQuery(query): ApiQuery<CatalogQuery>,
) -> Result<Response, ApiError> {
    let filter = query.validate()?;
    let (rows, has_more) = service::search(&state, &filter).await?;
    let page = CatalogPage {
        courses: rows
            .iter()
            .map(|row| CourseCard::build(&state, row))
            .collect(),
        page: filter.offset / filter.limit + 1,
        has_more,
    };
    Ok(cacheable(&request, &page))
}

async fn course(
    State(state): State<AppState>,
    request: HeaderMap,
    Path(slug): Path<String>,
) -> Result<Response, ApiError> {
    let course = service::course(&state, &slug).await?;
    Ok(cacheable(&request, &CourseDetailEnvelope { course }))
}

async fn creator(
    State(state): State<AppState>,
    request: HeaderMap,
    Path(handle): Path<String>,
) -> Result<Response, ApiError> {
    let (profile, courses) = service::creator(&state, &handle.to_ascii_lowercase()).await?;
    Ok(cacheable(
        &request,
        &creator_page(&state, &profile, &courses),
    ))
}
