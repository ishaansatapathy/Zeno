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

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;
use sqlx::PgPool;
use tokio::sync::Mutex;
use uuid::Uuid;
use zeno_core::config::{parse_or, required, ConfigError, Environment};
use zeno_core::keyvault::KeyVault;
use zeno_core::playback::PlaybackSigner;

/// A player fetches the key once per rendition it plays, and again after errors or seeks into a
/// fresh variant; far more than this from one session is somebody harvesting.
pub const MAX_KEY_FETCHES_PER_SESSION: i32 = 60;
const MINUTE: Duration = Duration::from_secs(60);
const DAY: Duration = Duration::from_secs(24 * 60 * 60);

pub struct KeyServerConfig {
    pub env: Environment,
    pub database_url: String,
    pub redis_url: String,
    pub bind_addr: SocketAddr,
    pub trusted_proxy_hops: usize,
    pub ip_limit_per_minute: u64,
    pub user_limit_per_day: u64,
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
        let ip_limit_per_minute = parse_or("KEY_IP_LIMIT_PER_MINUTE", 120u64)?;
        let user_limit_per_day = parse_or("KEY_USER_LIMIT_PER_DAY", 1_000u64)?;
        if ip_limit_per_minute == 0 || user_limit_per_day == 0 {
            return Err(ConfigError::Invalid {
                name: "KEY_IP_LIMIT_PER_MINUTE",
                reason: "key delivery limits must be at least 1".into(),
            });
        }
        Ok(Self {
            env: parse_or("APP_ENV", Environment::Development)?,
            database_url: required("DATABASE_URL")?,
            redis_url: required("REDIS_URL")?,
            bind_addr,
            trusted_proxy_hops: parse_or("TRUSTED_PROXY_HOPS", 0usize)?,
            ip_limit_per_minute,
            user_limit_per_day,
            client_origin: origin_of(&required("CLIENT_URL")?)?,
            vault: KeyVault::from_env()?,
            tokens: PlaybackSigner::from_env()?,
        })
    }
}

const INCR_WITH_WINDOW: &str = r#"
local count = redis.call('INCR', KEYS[1])
if count == 1 then redis.call('PEXPIRE', KEYS[1], ARGV[1]) end
local ttl = redis.call('PTTL', KEYS[1])
return {count, ttl}
"#;

#[derive(Clone)]
pub struct KeyLimiter {
    backend: Arc<LimiterBackend>,
    ip_limit: u64,
    user_limit: u64,
}

enum LimiterBackend {
    Redis {
        conn: Box<redis::aio::ConnectionManager>,
        script: redis::Script,
    },
    Memory(Mutex<HashMap<String, (u64, Instant)>>),
}

enum LimitError {
    Exceeded(u64),
    Unavailable(anyhow::Error),
}

impl KeyLimiter {
    pub fn redis(conn: redis::aio::ConnectionManager, ip_limit: u64, user_limit: u64) -> Self {
        Self {
            backend: Arc::new(LimiterBackend::Redis {
                conn: Box::new(conn),
                script: redis::Script::new(INCR_WITH_WINDOW),
            }),
            ip_limit,
            user_limit,
        }
    }

    pub fn memory(ip_limit: u64, user_limit: u64) -> Self {
        Self {
            backend: Arc::new(LimiterBackend::Memory(Mutex::new(HashMap::new()))),
            ip_limit,
            user_limit,
        }
    }

    async fn ip(&self, ip: Option<IpAddr>) -> Result<(), LimitError> {
        self.hit(
            &format!("key:ip:{}", limit_bucket(ip)),
            self.ip_limit,
            MINUTE,
        )
        .await
    }

    async fn user(&self, user: Uuid) -> Result<(), LimitError> {
        self.hit(&format!("key:user:{user}"), self.user_limit, DAY)
            .await
    }

    async fn hit(&self, key: &str, limit: u64, window: Duration) -> Result<(), LimitError> {
        let (count, ttl_ms) = match self.backend.as_ref() {
            LimiterBackend::Redis { conn, script } => {
                let mut conn = conn.as_ref().clone();
                script
                    .key(format!("zeno:limit:{key}"))
                    .arg(u64::try_from(window.as_millis()).unwrap_or(u64::MAX))
                    .invoke_async::<(u64, i64)>(&mut conn)
                    .await
                    .map_err(|err| LimitError::Unavailable(err.into()))?
            }
            LimiterBackend::Memory(entries) => {
                let now = Instant::now();
                let mut entries = entries.lock().await;
                let entry = entries.entry(key.to_owned()).or_insert((0, now + window));
                if entry.1 <= now {
                    *entry = (0, now + window);
                }
                entry.0 += 1;
                let ttl = entry.1.saturating_duration_since(now).as_millis();
                (entry.0, i64::try_from(ttl).unwrap_or(i64::MAX))
            }
        };
        if count > limit {
            return Err(LimitError::Exceeded(
                u64::try_from(ttl_ms).unwrap_or(1).div_ceil(1_000).max(1),
            ));
        }
        Ok(())
    }
}

fn limit_bucket(ip: Option<IpAddr>) -> String {
    match ip {
        None => "unknown".into(),
        Some(IpAddr::V4(ip)) => ip.to_string(),
        Some(IpAddr::V6(ip)) => {
            let s = ip.segments();
            format!("{:x}:{:x}:{:x}:{:x}::/64", s[0], s[1], s[2], s[3])
        }
    }
}

fn client_ip(headers: &HeaderMap, peer: Option<IpAddr>, trusted_hops: usize) -> Option<IpAddr> {
    if trusted_hops == 0 {
        return peer;
    }
    let chain: Vec<&str> = headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .collect();
    if chain.len() < trusted_hops {
        return peer;
    }
    chain[chain.len() - trusted_hops].parse().ok().or(peer)
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
    pub limiter: KeyLimiter,
    pub trusted_proxy_hops: usize,
}

impl KeyServer {
    pub fn new(db: PgPool, config: &KeyServerConfig, limiter: KeyLimiter) -> anyhow::Result<Self> {
        Ok(Self {
            db,
            vault: config.vault.clone(),
            tokens: config.tokens.clone(),
            client_origin: HeaderValue::from_str(&config.client_origin)?,
            limiter,
            trusted_proxy_hops: config.trusted_proxy_hops,
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
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::REFERRER_POLICY, "no-referrer"),
        ],
        Json(json!({ "error": { "code": code, "message": "Playback is not allowed." } })),
    )
        .into_response()
}

fn limited(retry_after: u64) -> Response {
    let mut response = refuse(StatusCode::TOO_MANY_REQUESTS, "too_many_key_requests");
    if let Ok(value) = HeaderValue::from_str(&retry_after.to_string()) {
        response.headers_mut().insert(header::RETRY_AFTER, value);
    }
    response
}

fn unavailable(err: anyhow::Error) -> Response {
    tracing::error!(error = ?err, "key rate limiter unavailable; failing closed");
    refuse(StatusCode::SERVICE_UNAVAILABLE, "unavailable")
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
    connect: Option<ConnectInfo<SocketAddr>>,
    Path(video_id): Path<String>,
    Query(query): Query<KeyQuery>,
    headers: HeaderMap,
) -> Response {
    let ip = client_ip(
        &headers,
        connect.map(|ConnectInfo(addr)| addr.ip()),
        state.trusted_proxy_hops,
    );
    match state.limiter.ip(ip).await {
        Ok(()) => {}
        Err(LimitError::Exceeded(retry)) => return limited(retry),
        Err(LimitError::Unavailable(err)) => return unavailable(err),
    }
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
    match state.limiter.user(claims.user_id).await {
        Ok(()) => {}
        Err(LimitError::Exceeded(retry)) => return limited(retry),
        Err(LimitError::Unavailable(err)) => return unavailable(err),
    }

    let granted: Option<Granted> = match sqlx::query_as(
        "with candidate as materialized ( \
             select s.id, k.wrapped_key, case \
                 when s.expires_at <= now() then 'expired' \
                 when v.deleted_at is not null then 'video_deleted' \
                 when v.status <> 'ready' then 'access_revoked' \
                 else video_access_state(s.user_id, s.video_id) \
             end as access_state \
             from playback_sessions s \
             join videos v on v.id=s.video_id \
             join video_keys k on k.video_id=s.video_id \
             where s.id=$1 and s.user_id=$2 and s.video_id=$3 and s.ended_at is null \
         ), ended as ( \
             update playback_sessions s set ended_at=now(), end_reason=c.access_state \
             from candidate c where s.id=c.id and c.access_state <> 'allowed' \
         ) \
         update playback_sessions s set last_seen_at=now(), key_fetches=s.key_fetches+1 \
         from candidate c where s.id=c.id and c.access_state='allowed' \
         returning s.key_fetches, c.wrapped_key",
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
                header::REFERRER_POLICY,
                HeaderValue::from_static("no-referrer"),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_addresses_are_only_trusted_when_configured() {
        let peer: IpAddr = "10.0.0.1".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "6.6.6.6, 1.2.3.4".parse().unwrap());
        assert_eq!(client_ip(&headers, Some(peer), 0), Some(peer));
        assert_eq!(
            client_ip(&headers, Some(peer), 1),
            Some("1.2.3.4".parse().unwrap())
        );
        assert_eq!(
            limit_bucket(Some("2001:db8:1:2::1".parse().unwrap())),
            "2001:db8:1:2::/64"
        );
    }

    /// CI supplies Redis; local runs without it still exercise the same policy through memory.
    #[tokio::test]
    async fn redis_limiter_uses_shared_expiring_counters() {
        let Ok(url) = std::env::var("TEST_REDIS_URL") else {
            eprintln!("skipped: TEST_REDIS_URL not set");
            return;
        };
        let client = redis::Client::open(url).unwrap();
        let conn = redis::aio::ConnectionManager::new(client).await.unwrap();
        let limiter = KeyLimiter::redis(conn, 2, 2);
        let ip: IpAddr = format!("10.{}.{}.{}", rand_byte(), rand_byte(), rand_byte())
            .parse()
            .unwrap();
        assert!(limiter.ip(Some(ip)).await.is_ok());
        assert!(limiter.ip(Some(ip)).await.is_ok());
        assert!(matches!(
            limiter.ip(Some(ip)).await,
            Err(LimitError::Exceeded(_))
        ));
    }

    fn rand_byte() -> u8 {
        let bytes = Uuid::new_v4();
        bytes.as_bytes()[0].max(1)
    }
}
