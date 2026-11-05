use std::convert::Infallible;
use std::net::SocketAddr;

use axum::async_trait;
use axum::extract::{ConnectInfo, FromRequest, FromRequestParts, Request};
use axum::http::header::USER_AGENT;
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::Json;
use axum_extra::extract::CookieJar;
use serde::de::DeserializeOwned;
use uuid::Uuid;
use zeno_core::model::Role;

use crate::auth::cookies::ACCESS_COOKIE;
use crate::auth::revocation;
use crate::auth::service::ClientContext;
use crate::error::ApiError;
use crate::net::client_ip;
use crate::state::AppState;

/// The signed-in caller, taken from a verified access-token cookie. Add it as a handler
/// argument to require authentication; use `require_role` for role-restricted routes.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub id: Uuid,
    pub role: Role,
    pub session_id: Uuid,
}

impl AuthUser {
    pub fn require_role(&self, allowed: &[Role]) -> Result<(), ApiError> {
        if allowed.contains(&self.role) {
            Ok(())
        } else {
            Err(ApiError::forbidden(
                "forbidden",
                "You don't have access to this resource.",
            ))
        }
    }
}

#[async_trait]
impl FromRequestParts<AppState> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let unauthenticated = || ApiError::unauthorized("unauthenticated", "Sign in to continue.");
        let jar = CookieJar::from_headers(&parts.headers);
        let token = jar.get(ACCESS_COOKIE).ok_or_else(unauthenticated)?;
        let claims = state
            .tokens
            .verify_access(token.value())
            .map_err(|_| unauthenticated())?;
        if revocation::is_revoked(&*state.counters, claims.fid).await {
            return Err(unauthenticated());
        }
        Ok(AuthUser {
            id: claims.sub,
            role: claims.role,
            session_id: claims.sid,
        })
    }
}

/// Who is calling and from where, with the address resolved through the configured number
/// of trusted proxies.
pub struct Client(pub ClientContext);

#[async_trait]
impl FromRequestParts<AppState> for Client {
    type Rejection = Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let peer = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ConnectInfo(addr)| addr.ip());
        let ip = client_ip(&parts.headers, peer, state.config.trusted_proxy_hops);
        let user_agent = parts
            .headers
            .get(USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        Ok(Client(ClientContext { ip, user_agent }))
    }
}

/// `Json` that reports malformed bodies in the API's own error format.
pub struct ApiJson<T>(pub T);

#[async_trait]
impl<S, T> FromRequest<S> for ApiJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(value)) => Ok(ApiJson(value)),
            Err(rejection) => {
                let status = rejection.status();
                let code = match status {
                    StatusCode::PAYLOAD_TOO_LARGE => "payload_too_large",
                    StatusCode::UNSUPPORTED_MEDIA_TYPE => "unsupported_media_type",
                    _ => "invalid_request",
                };
                Err(ApiError::Http {
                    status,
                    code,
                    message: rejection.body_text(),
                })
            }
        }
    }
}
