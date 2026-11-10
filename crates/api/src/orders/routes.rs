use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use uuid::Uuid;

use crate::auth::extractor::AuthUser;
use crate::enrollments::dto::EnrollmentResponse;
use crate::error::ApiError;
use crate::orders::dto::{CheckoutBody, EnrolledEnvelope, OrderEnvelope, PreviewEnvelope};
use crate::orders::service;
use crate::ratelimit::{enforce_user, policies};
use crate::refunds::dto::RefundResponse;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/checkout/:course_id", post(checkout))
        .route("/v1/checkout/:course_id/preview", post(preview))
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

/// The coupon code in a checkout body, if there is one. The body is optional, and apart from the
/// coupon everything in it is ignored: the price is never the client's to name.
fn coupon_in(body: &[u8]) -> Result<Option<String>, ApiError> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(None);
    }
    let parsed: CheckoutBody = serde_json::from_slice(body).map_err(|_| {
        ApiError::bad_request(
            "invalid_body",
            "The request body must be JSON, e.g. {\"coupon\": \"CODE\"}.",
        )
    })?;
    Ok(parsed.coupon.filter(|code| !code.trim().is_empty()))
}

/// Starts (or resumes) checkout. The course is in the path, and the amount, currency and title are
/// the server's. The body may carry a `coupon` code; nothing else in it is read.
async fn checkout(
    State(state): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
    Path(course_id): Path<String>,
    body: Bytes,
) -> Result<Response, ApiError> {
    enforce_user(&state, &policies::CHECKOUT, user.id).await?;
    let course_id = Uuid::parse_str(&course_id).map_err(|_| not_found("Course"))?;
    let key = idempotency_key(&headers)?;
    let coupon = coupon_in(&body)?;
    if coupon.is_some() {
        enforce_user(&state, &policies::COUPON_TRY, user.id).await?;
    }
    match service::checkout_with(&state, &user, course_id, key.as_deref(), coupon.as_deref())
        .await?
    {
        service::Outcome::Order(outcome) => {
            let status = if outcome.created {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            };
            Ok((
                status,
                Json(service::envelope(&state, &outcome.order, None)?),
            )
                .into_response())
        }
        service::Outcome::Enrolled(enrolled) => Ok((
            StatusCode::CREATED,
            Json(EnrolledEnvelope {
                status: "enrolled",
                enrollment: EnrollmentResponse::from(&enrolled.enrollment),
                price: enrolled.price,
            }),
        )
            .into_response()),
    }
}

/// What the course would cost this person with this coupon. Creates nothing, uses nothing up.
async fn preview(
    State(state): State<AppState>,
    user: AuthUser,
    Path(course_id): Path<String>,
    body: Bytes,
) -> Result<Json<PreviewEnvelope>, ApiError> {
    enforce_user(&state, &policies::ORDER_READ, user.id).await?;
    let course_id = Uuid::parse_str(&course_id).map_err(|_| not_found("Course"))?;
    let coupon = coupon_in(&body)?;
    if coupon.is_some() {
        enforce_user(&state, &policies::COUPON_TRY, user.id).await?;
    }
    let price = service::preview(&state, &user, course_id, coupon.as_deref()).await?;
    Ok(Json(PreviewEnvelope { price }))
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
