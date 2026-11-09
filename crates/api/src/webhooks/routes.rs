use std::net::SocketAddr;

use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{ConnectInfo, DefaultBodyLimit};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::error::ApiError;
use crate::ratelimit::{enforce_address, policies};
use crate::state::AppState;
use crate::webhooks::service::{self, Received};

/// Real events are a few kilobytes; this leaves room for large payloads and nothing more.
const MAX_BODY_BYTES: usize = 256 * 1024;
const SIGNATURE_HEADER: &str = "x-razorpay-signature";
const EVENT_ID_HEADER: &str = "x-razorpay-event-id";

pub fn router() -> Router<AppState> {
    Router::new().route(
        "/v1/webhooks/razorpay",
        post(razorpay).layer(DefaultBodyLimit::max(MAX_BODY_BYTES)),
    )
}

/// The provider's id for this delivery, or, if it is missing or odd, a hash of the body: either
/// way a redelivery of the same event maps to the same key.
fn event_id(headers: &HeaderMap, body: &[u8]) -> String {
    let given = headers
        .get(EVENT_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|id| (1..=200).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_graphic()));
    match given {
        Some(id) => id.to_owned(),
        None => format!("body-sha256:{:x}", Sha256::digest(body)),
    }
}

/// `POST /v1/webhooks/razorpay`: server to server. No cookies, no CSRF, no account; the signature
/// over the raw body is the credential.
///
/// * `401` for a missing or wrong signature (the provider will see its configuration is wrong);
/// * `200` for everything we understood, including events we deliberately do nothing with, so the
///   provider stops resending them;
/// * `5xx` only when handling failed and a retry might succeed.
async fn razorpay(
    axum::extract::State(state): axum::extract::State<AppState>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let body = body.map_err(|_| ApiError::Http {
        status: StatusCode::PAYLOAD_TOO_LARGE,
        code: "payload_too_large",
        message: "The request body is too large.".into(),
    })?;

    // The bytes exactly as received: re-serialising JSON would change them and break the check.
    let signature = headers
        .get(SIGNATURE_HEADER)
        .and_then(|value| value.to_str().ok());
    let genuine = signature.is_some_and(|sig| state.payments.verify_webhook(&body, sig));
    if !genuine {
        // Forgeries are counted per address; the provider's own traffic never reaches this line.
        enforce_address(
            &state,
            &policies::WEBHOOK_REJECTED,
            &headers,
            peer.map(|ConnectInfo(addr)| addr.ip()),
        )
        .await?;
        tracing::warn!("webhook with a missing or invalid signature");
        return Err(ApiError::unauthorized(
            "invalid_signature",
            "The webhook signature is missing or invalid.",
        ));
    }

    let id = event_id(&headers, &body);
    let answer = match service::receive(&state, &id, &body).await? {
        Received::Handled(outcome) => json!({"received": true, "outcome": outcome.as_str()}),
        Received::Duplicate => json!({"received": true, "duplicate": true}),
    };
    Ok((StatusCode::OK, Json(answer)))
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    #[test]
    fn the_provider_event_id_is_used_when_it_is_sane() {
        let mut headers = HeaderMap::new();
        headers.insert(EVENT_ID_HEADER, HeaderValue::from_static("evt_Abc123"));
        assert_eq!(event_id(&headers, b"{}"), "evt_Abc123");
    }

    #[test]
    fn a_missing_or_odd_id_falls_back_to_the_body_hash() {
        let empty = HeaderMap::new();
        let from_body = event_id(&empty, b"{\"a\":1}");
        assert!(from_body.starts_with("body-sha256:") && from_body.len() == 12 + 64);
        // The same body always gives the same key, and a different one a different key.
        assert_eq!(from_body, event_id(&empty, b"{\"a\":1}"));
        assert_ne!(from_body, event_id(&empty, b"{\"a\":2}"));
        for odd in ["", "has space", &"x".repeat(201)] {
            let mut headers = HeaderMap::new();
            if let Ok(value) = HeaderValue::from_str(odd) {
                headers.insert(EVENT_ID_HEADER, value);
            }
            assert!(
                event_id(&headers, b"{}").starts_with("body-sha256:"),
                "{odd:?}"
            );
        }
    }
}
