use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct FieldError {
    pub field: &'static str,
    pub message: String,
}

impl FieldError {
    pub fn new(field: &'static str, message: impl Into<String>) -> Self {
        Self {
            field,
            message: message.into(),
        }
    }
}

#[derive(Debug)]
pub enum ApiError {
    BadRequest {
        code: &'static str,
        message: String,
    },
    Validation(Vec<FieldError>),
    /// A client error whose status is decided elsewhere (body too large, wrong media type).
    Http {
        status: StatusCode,
        code: &'static str,
        message: String,
    },
    Unauthorized {
        code: &'static str,
        message: &'static str,
    },
    Forbidden {
        code: &'static str,
        message: &'static str,
    },
    TooManyRequests {
        retry_after_secs: u64,
    },
    Unavailable(&'static str),
    Internal(anyhow::Error),
}

impl ApiError {
    pub fn bad_request(code: &'static str, message: impl Into<String>) -> Self {
        Self::BadRequest {
            code,
            message: message.into(),
        }
    }

    pub fn unauthorized(code: &'static str, message: &'static str) -> Self {
        Self::Unauthorized { code, message }
    }

    pub fn forbidden(code: &'static str, message: &'static str) -> Self {
        Self::Forbidden { code, message }
    }

    pub fn validation(errors: Vec<FieldError>) -> Self {
        Self::Validation(errors)
    }
}

impl<E> From<E> for ApiError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self::Internal(err.into())
    }
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    error: ErrorDetail<'a>,
}

#[derive(Serialize)]
struct ErrorDetail<'a> {
    code: &'a str,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    fields: Option<&'a [FieldError]>,
}

fn respond(
    status: StatusCode,
    code: &str,
    message: &str,
    fields: Option<&[FieldError]>,
) -> Response {
    let body = ErrorBody {
        error: ErrorDetail {
            code,
            message,
            fields,
        },
    };
    (status, Json(body)).into_response()
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            ApiError::BadRequest { code, message } => {
                respond(StatusCode::BAD_REQUEST, code, &message, None)
            }
            ApiError::Http {
                status,
                code,
                message,
            } => respond(status, code, &message, None),
            ApiError::Validation(fields) => respond(
                StatusCode::UNPROCESSABLE_ENTITY,
                "validation_failed",
                "Some fields are invalid.",
                Some(&fields),
            ),
            ApiError::Unauthorized { code, message } => {
                respond(StatusCode::UNAUTHORIZED, code, message, None)
            }
            ApiError::Forbidden { code, message } => {
                respond(StatusCode::FORBIDDEN, code, message, None)
            }
            ApiError::TooManyRequests { retry_after_secs } => {
                let mut response = respond(
                    StatusCode::TOO_MANY_REQUESTS,
                    "too_many_requests",
                    "Too many attempts. Please try again later.",
                    None,
                );
                if let Ok(value) = retry_after_secs.to_string().parse() {
                    response.headers_mut().insert(header::RETRY_AFTER, value);
                }
                response
            }
            ApiError::Unavailable(what) => {
                respond(StatusCode::SERVICE_UNAVAILABLE, "unavailable", what, None)
            }
            ApiError::Internal(err) => {
                tracing::error!(error = ?err, "internal error");
                respond(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "Something went wrong. Please try again.",
                    None,
                )
            }
        }
    }
}
