//! Responses an edge or a browser may reuse: JSON with a content-derived `ETag`, so a repeat
//! request that already holds this exact answer gets an empty `304` instead.
//!
//! Used by anything public and cacheable (the catalogue, share-link pages). The origin sees at
//! most one request per cache lifetime per edge, however many people ask.

use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use sha2::{Digest, Sha256};

/// A weak validator over the serialised body: 16 hex digits of its SHA-256.
fn etag_of(body: &[u8]) -> String {
    let digest = Sha256::digest(body);
    format!(
        "W/\"{}\"",
        digest[..8]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    )
}

/// Whether `If-None-Match` already names `etag` (or is the wildcard).
fn already_held(request: &HeaderMap, etag: &str) -> bool {
    request
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|sent| {
            sent.split(',')
                .map(str::trim)
                .any(|candidate| candidate == "*" || candidate == etag)
        })
}

/// `value` as JSON with `Cache-Control: cache_control` and an `ETag`; `304` when the caller
/// already has this exact body.
pub fn json_with_etag<T: Serialize>(
    request: &HeaderMap,
    value: &T,
    cache_control: &'static str,
) -> Response {
    let body = serde_json::to_vec(value).expect("cacheable responses serialise");
    let etag = etag_of(&body);
    let mut response = if already_held(request, &etag) {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        ([(header::CONTENT_TYPE, "application/json")], body).into_response()
    };
    let headers = response.headers_mut();
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(cache_control),
    );
    headers.insert(
        header::ETAG,
        HeaderValue::from_str(&etag).expect("hex is a valid header value"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_same_body_has_the_same_validator_and_a_different_one_differs() {
        assert_eq!(etag_of(b"{\"a\":1}"), etag_of(b"{\"a\":1}"));
        assert_ne!(etag_of(b"{\"a\":1}"), etag_of(b"{\"a\":2}"));
        assert!(etag_of(b"x").starts_with("W/\"") && etag_of(b"x").len() == 3 + 16 + 1);
    }

    #[test]
    fn a_matching_validator_gets_an_empty_304_with_the_cache_headers() {
        let value = json!({"title": "Lecture"});
        let first = json_with_etag(&HeaderMap::new(), &value, "public, max-age=15");
        assert_eq!(first.status(), StatusCode::OK);
        let etag = first.headers()[header::ETAG].to_str().unwrap().to_owned();
        assert_eq!(first.headers()[header::CACHE_CONTROL], "public, max-age=15");

        for sent in [etag.as_str(), "*", &format!("\"other\", {etag}")] {
            let mut request = HeaderMap::new();
            request.insert(header::IF_NONE_MATCH, sent.parse().unwrap());
            let again = json_with_etag(&request, &value, "public, max-age=15");
            assert_eq!(again.status(), StatusCode::NOT_MODIFIED, "{sent}");
            assert_eq!(again.headers()[header::ETAG].to_str().unwrap(), etag);
            assert_eq!(again.headers()[header::CACHE_CONTROL], "public, max-age=15");
        }
        let mut stale = HeaderMap::new();
        stale.insert(
            header::IF_NONE_MATCH,
            "W/\"0000000000000000\"".parse().unwrap(),
        );
        assert_eq!(
            json_with_etag(&stale, &value, "public, max-age=15").status(),
            StatusCode::OK
        );
    }
}
