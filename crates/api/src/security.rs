use axum::extract::{Request, State};
use axum::http::{header, HeaderName, HeaderValue, Method};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use tower_http::cors::{AllowOrigin, CorsLayer};

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
            Some(origin) => {
                origin.as_bytes() == state.config.client_origin.as_bytes()
                    || state
                        .config
                        .extension_origins
                        .iter()
                        .any(|allowed| allowed.as_bytes() == origin.as_bytes())
            }
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

pub fn cors(client_origin: &str, extension_origins: &[String]) -> CorsLayer {
    let layer = CorsLayer::new()
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ])
        .allow_headers([
            header::CONTENT_TYPE,
            HeaderName::from_static("idempotency-key"),
        ])
        .allow_credentials(true)
        .max_age(std::time::Duration::from_secs(600));
    let allowed: Vec<HeaderValue> = std::iter::once(client_origin)
        .chain(extension_origins.iter().map(String::as_str))
        .filter_map(|origin| HeaderValue::from_str(origin).ok())
        .collect();
    layer.allow_origin(AllowOrigin::list(allowed))
}
