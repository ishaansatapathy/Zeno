//! Optimistic concurrency over HTTP: a writer says which version of a resource it read, in an
//! `If-Match` header or in its body, and loses with `409` if someone else wrote since.
//!
//! One implementation for every versioned resource, so a course and a recording answer the same
//! way. `noun` is only for the messages ("course", "recording").

use axum::http::{header, HeaderMap, StatusCode};

use crate::error::ApiError;

/// The version in `If-Match` (`3`, `"3"` or `W/"3"`), if the header is present.
pub fn header_version(headers: &HeaderMap, noun: &str) -> Result<Option<i32>, ApiError> {
    let Some(raw) = headers.get(header::IF_MATCH) else {
        return Ok(None);
    };
    let raw = raw
        .to_str()
        .map_err(|_| ApiError::bad_request("invalid_version", "If-Match is invalid."))?;
    let value = raw
        .strip_prefix("W/")
        .unwrap_or(raw)
        .trim_matches('"')
        .parse::<i32>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            ApiError::bad_request(
                "invalid_version",
                format!("If-Match must contain a positive {noun} version."),
            )
        })?;
    Ok(Some(value))
}

/// The version the writer says it read: from the header, from the body, or both if they agree.
pub fn expected_version(
    headers: &HeaderMap,
    body: Option<i32>,
    noun: &str,
) -> Result<i32, ApiError> {
    let header = header_version(headers, noun)?;
    if header.is_some() && body.is_some() && header != body {
        return Err(ApiError::bad_request(
            "version_mismatch",
            "The body version and If-Match header disagree.",
        ));
    }
    let version = header.or(body).ok_or_else(|| ApiError::Http {
        status: StatusCode::PRECONDITION_REQUIRED,
        code: "version_required",
        message: format!("Send the {noun} version in If-Match or the request body."),
    })?;
    if version <= 0 {
        return Err(ApiError::bad_request(
            "invalid_version",
            format!("The {noun} version must be a positive integer."),
        ));
    }
    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_headers_are_strict() {
        let mut headers = HeaderMap::new();
        assert!(expected_version(&headers, None, "course").is_err());
        headers.insert(header::IF_MATCH, "W/\"7\"".parse().unwrap());
        assert_eq!(expected_version(&headers, None, "course").unwrap(), 7);
        assert!(expected_version(&headers, Some(8), "course").is_err());
        assert_eq!(expected_version(&headers, Some(7), "course").unwrap(), 7);
    }

    #[test]
    fn rejects_junk_and_names_the_resource() {
        let mut headers = HeaderMap::new();
        for junk in ["abc", "0", "-3", "\"\""] {
            headers.insert(header::IF_MATCH, junk.parse().unwrap());
            assert!(header_version(&headers, "recording").is_err(), "{junk}");
        }
        headers.clear();
        let err = expected_version(&headers, None, "recording").unwrap_err();
        assert!(format!("{err:?}").contains("recording"));
    }
}
