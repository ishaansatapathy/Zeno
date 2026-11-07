//! Key delivery: the only place a video's AES-128 content key ever leaves the system.
//!
//! `GET /v1/keys/:video_id?t=<playback token>` answers the 16 raw key bytes when, and only when:
//! the token's signature is good and it has not expired; it was issued for this video; the
//! playback session it names is still open, unexpired and belongs to the token's user; the video
//! is ready and not deleted; and the session has not already fetched the key an unreasonable
//! number of times. Every refusal looks the same from outside.
//!
//! A request from a browser page on any origin but the web app's is refused outright, which is
//! the "domain lock": a key URL copied into a player on another site does not work there.

use std::net::SocketAddr;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;
use zeno_core::config::{parse_or, required, ConfigError, Environment};
use zeno_core::keyvault::KeyVault;
use zeno_core::playback::PlaybackSigner;

/// A player fetches the key once per rendition it plays, and again after errors or seeks into a
/// fresh variant; far more than this from one session is somebody harvesting.
pub const MAX_KEY_FETCHES_PER_SESSION: i32 = 60;

pub struct KeyServerConfig {
    pub env: Environment,
    pub database_url: String,
    pub bind_addr: SocketAddr,
    /// Origin of the web app (`CLIENT_URL`), the only browser origin allowed to fetch keys.
    pub client_origin: String,
    pub vault: KeyVault,
    pub tokens: PlaybackSigner,
}

impl KeyServerConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let bind_addr = parse_or("KEYSERVER_BIND_ADDR", "0.0.0.0:8081".to_owned())?
            .parse()
            .map_err(|e: std::net::AddrParseError| ConfigError::Invalid {
                name: "KEYSERVER_BIND_ADDR",
                reason: e.to_string(),
            })?;
        Ok(Self {
            env: parse_or("APP_ENV", Environment::Development)?,
            database_url: required("DATABASE_URL")?,
            bind_addr,
            client_origin: origin_of(&required("CLIENT_URL")?)?,
            vault: KeyVault::from_env()?,
            tokens: PlaybackSigner::from_env()?,
        })
    }
}

fn origin_of(raw: &str) -> Result<String, ConfigError> {
    let url = url::Url::parse(raw.trim()).map_err(|e| ConfigError::Invalid {
        name: "CLIENT_URL",
        reason: e.to_string(),
    })?;
    Ok(url.origin().ascii_serialization())
}

#[derive(Clone)]
pub struct KeyServer {
    pub db: PgPool,
    pub vault: KeyVault,
    pub tokens: PlaybackSigner,
    pub client_origin: HeaderValue,
}

impl KeyServer {
    pub fn new(db: PgPool, config: &KeyServerConfig) -> anyhow::Result<Self> {
        Ok(Self {
            db,
            vault: config.vault.clone(),
            tokens: config.tokens.clone(),
            client_origin: HeaderValue::from_str(&config.client_origin)?,
        })
    }
}

pub fn router(state: KeyServer) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/v1/keys/:video_id", get(key))
        .with_state(state)
}

#[derive(Deserialize)]
struct KeyQuery {
    t: Option<String>,
}

fn refuse(status: StatusCode, code: &'static str) -> Response {
    (
        status,
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({ "error": { "code": code, "message": "Playback is not allowed." } })),
    )
        .into_response()
}

fn denied() -> Response {
    refuse(StatusCode::FORBIDDEN, "key_denied")
}

#[derive(sqlx::FromRow)]
struct Granted {
    key_fetches: i32,
    wrapped_key: Vec<u8>,
}

async fn key(
    State(state): State<KeyServer>,
    Path(video_id): Path<String>,
    Query(query): Query<KeyQuery>,
    headers: HeaderMap,
) -> Response {
    // Domain lock. Requests without an Origin (native players, the HLS stack in Safari) carry on
    // to the token checks.
    let origin = headers.get(header::ORIGIN);
    if origin.is_some_and(|o| o != state.client_origin) {
        tracing::info!(origin = ?origin, "key request from a foreign origin");
        return denied();
    }
    let (Ok(video_id), Some(token)) = (Uuid::parse_str(&video_id), query.t) else {
        return denied();
    };
    let claims = match state.tokens.verify(&token, Utc::now()) {
        Ok(claims) if claims.video_id == video_id => claims,
        Ok(_) => return denied(),
        Err(err) => {
            tracing::debug!(%err, %video_id, "key token refused");
            return denied();
        }
    };

    let granted: Option<Granted> = match sqlx::query_as(
        "update playback_sessions s set last_seen_at = now(), key_fetches = s.key_fetches + 1 \
         from videos v, video_keys k \
         where s.id = $1 and s.user_id = $2 and s.video_id = $3 \
           and v.id = s.video_id and k.video_id = s.video_id \
           and s.ended_at is null and s.expires_at > now() \
           and v.status = 'ready' and v.deleted_at is null \
         returning s.key_fetches, k.wrapped_key",
    )
    .bind(claims.session_id)
    .bind(claims.user_id)
    .bind(video_id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(granted) => granted,
        Err(err) => {
            tracing::error!(error = ?err, "key lookup failed");
            return refuse(StatusCode::SERVICE_UNAVAILABLE, "unavailable");
        }
    };
    let Some(granted) = granted else {
        tracing::debug!(session_id = %claims.session_id, %video_id, "no open session for key request");
        return denied();
    };
    if granted.key_fetches > MAX_KEY_FETCHES_PER_SESSION {
        tracing::warn!(session_id = %claims.session_id, user_id = %claims.user_id, fetches = granted.key_fetches, "key fetch limit hit");
        return refuse(StatusCode::TOO_MANY_REQUESTS, "too_many_key_requests");
    }
    let key = match state.vault.open(video_id, &granted.wrapped_key) {
        Ok(key) => key,
        Err(err) => {
            // The stored key does not open under this secret: misconfiguration, not the viewer.
            tracing::error!(%err, %video_id, "stored content key could not be unwrapped");
            return refuse(StatusCode::INTERNAL_SERVER_ERROR, "internal_error");
        }
    };

    let mut response = (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/octet-stream"),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("no-store, private"),
            ),
            (
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            ),
            (header::VARY, HeaderValue::from_static("Origin")),
        ],
        key.to_vec(),
    )
        .into_response();
    if origin.is_some() {
        response.headers_mut().insert(
            header::ACCESS_CONTROL_ALLOW_ORIGIN,
            state.client_origin.clone(),
        );
    }
    response
}
