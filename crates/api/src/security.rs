use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, Method};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use tower_http::cors::CorsLayer;

use crate::error::ApiError;
use crate::state::AppState;

/// Browsers attach cookies to cross-site requests, so state-changing calls must come from
/// the web app's own origin. `SameSite=Lax` already blocks most of this; the check is the
/// second line of defence. Requests without an `Origin` header (curl, server-side fetches,
/// mobile SDKs) carry no ambient browser cookies and are let through.
pub async fn require_same_origin(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let safe_method = matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    );
    if !safe_method {
        let headers = request.headers();
        let origin_ok = match headers.get(header::ORIGIN) {
            Some(origin) => origin.as_bytes() == state.config.client_origin.as_bytes(),
            None => headers
                .get("sec-fetch-site")
                .is_none_or(|site| site.as_bytes() != b"cross-site"),
        };
        if !origin_ok {
            return ApiError::forbidden("forbidden_origin", "Cross-site requests are not allowed.")
                .into_response();
        }
    }
    next.run(request).await
}

pub fn cors(client_origin: &str) -> CorsLayer {
    let layer = CorsLayer::new()
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ])
        .allow_headers([header::CONTENT_TYPE])
        .allow_credentials(true)
        .max_age(std::time::Duration::from_secs(600));
    match HeaderValue::from_str(client_origin) {
        Ok(origin) => layer.allow_origin(origin),
        Err(_) => layer,
    }
}
