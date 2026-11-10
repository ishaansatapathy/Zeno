use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use uuid::Uuid;

use crate::auth::extractor::AuthUser;
use crate::error::ApiError;
use crate::orders::dto::OrderEnvelope;
use crate::orders::service;
use crate::ratelimit::{enforce_user, policies};
use crate::refunds::dto::RefundResponse;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/checkout/:course_id", post(checkout))
        .route("/v1/orders/:id", get(show))
}

fn not_found(subject: &'static str) -> ApiError {
    ApiError::Http {
        status: StatusCode::NOT_FOUND,
        code: "not_found",
        message: format!("{subject} not found."),
    }
}

/// The optional `Idempotency-Key` header: 1 to 64 visible characters from a small, URL-safe
/// set. Sending the header twice is ambiguous and refused.
fn idempotency_key(headers: &HeaderMap) -> Result<Option<String>, ApiError> {
    let invalid = || {
        ApiError::bad_request(
            "invalid_idempotency_key",
            "Idempotency-Key must be 1 to 64 characters: letters, digits and - _ . :",
        )
    };
    let mut values = headers.get_all("idempotency-key").iter();
    let Some(first) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(invalid());
    }
    let key = first.to_str().map_err(|_| invalid())?;
    let valid = (1..=64).contains(&key.len())
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'));
    if valid {
        Ok(Some(key.to_owned()))
    } else {
        Err(invalid())
    }
}

/// Starts (or resumes) checkout. The request has no body: the course is in the path, and the
/// amount, currency and title are the server's.
async fn checkout(
    State(state): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
    Path(course_id): Path<String>,
) -> Result<(StatusCode, Json<OrderEnvelope>), ApiError> {
    enforce_user(&state, &policies::CHECKOUT, user.id).await?;
    let course_id = Uuid::parse_str(&course_id).map_err(|_| not_found("Course"))?;
    let key = idempotency_key(&headers)?;
    let outcome = service::checkout(&state, &user, course_id, key.as_deref()).await?;
    let status = if outcome.created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((
        status,
        Json(service::envelope(&state, &outcome.order, None)?),
    ))
}

async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<OrderEnvelope>, ApiError> {
    enforce_user(&state, &policies::ORDER_READ, user.id).await?;
    let id = Uuid::parse_str(&id).map_err(|_| not_found("Order"))?;
    let order = service::show(&state, &user, id).await?;
    let refund = crate::refunds::repo::latest_for_order(&state.db, order.id).await?;
    Ok(Json(service::envelope(
        &state,
        &order,
        refund.as_ref().map(RefundResponse::from),
    )?))
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    fn headers(values: &[&str]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for value in values {
            map.append("idempotency-key", HeaderValue::from_str(value).unwrap());
        }
        map
    }

    #[test]
    fn the_key_is_optional_and_checked_when_present() {
        assert_eq!(idempotency_key(&headers(&[])).unwrap(), None);
        assert_eq!(
            idempotency_key(&headers(&["a1-B2_c3.d4:e5"]))
                .unwrap()
                .as_deref(),
            Some("a1-B2_c3.d4:e5")
        );
        let long = "k".repeat(64);
        assert!(idempotency_key(&headers(&[&long])).unwrap().is_some());
        for bad in [
            "",
            &"k".repeat(65),
            "has space",
            "slash/er",
            "emoji🙂",
            "semi;colon",
        ] {
            // An empty or non-ASCII value may not even build as a header; either way it is refused.
            if let Ok(value) = HeaderValue::from_str(bad) {
                let mut map = HeaderMap::new();
                map.insert("idempotency-key", value);
                assert!(idempotency_key(&map).is_err(), "{bad:?}");
            }
        }
        assert!(idempotency_key(&headers(&["one", "two"])).is_err());
    }
}
