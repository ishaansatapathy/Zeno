//! Key delivery against a real Postgres: every way a request can be wrong is refused, and the
//! one right way returns the key that was sealed for the video.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use zeno_core::keyvault::KeyVault;
use zeno_core::playback::{PlaybackClaims, PlaybackSigner};
use zeno_keyserver::{router, KeyServer, MAX_KEY_FETCHES_PER_SESSION};

const ORIGIN: &str = "http://localhost:3000";
const KEY: [u8; 16] = *b"sixteen byte key";

macro_rules! db_test {
    ($(#[$meta:meta])* async fn $name:ident($pool:ident: PgPool) $body:block) => {
        #[sqlx::test(migrator = "zeno_core::db::MIGRATOR")]
        $(#[$meta])*
        async fn $name($pool: PgPool) $body
    };
}

fn vault() -> KeyVault {
    KeyVault::from_secret(
        &base64::engine::general_purpose::STANDARD.encode(*b"0123456789abcdef0123456789ABCDEF"),
    )
    .unwrap()
}

fn signer() -> PlaybackSigner {
    PlaybackSigner::new("playback-secret-for-tests-only-0123456789").unwrap()
}

fn app(pool: PgPool) -> Router {
    router(KeyServer {
        db: pool,
        vault: vault(),
        tokens: signer(),
        client_origin: ORIGIN.parse().unwrap(),
    })
}

struct Seeded {
    user: Uuid,
    video: Uuid,
    session: Uuid,
    expires_at: DateTime<Utc>,
}

impl Seeded {
    fn claims(&self) -> PlaybackClaims {
        PlaybackClaims {
            session_id: self.session,
            video_id: self.video,
            user_id: self.user,
            expires_at: self.expires_at,
        }
    }

    fn token(&self) -> String {
        signer().sign(&self.claims())
    }

    fn url(&self) -> String {
        format!("/v1/keys/{}?t={}", self.video, self.token())
    }
}

async fn seed_video(pool: &PgPool, owner: Uuid, key: &[u8; 16]) -> Uuid {
    let video = Uuid::new_v4();
    sqlx::query(
        "insert into videos (id, owner_id, original_filename, content_type, size_bytes, part_size, source_key, status) \
         values ($1, $2, 'a.mp4', 'video/mp4', 10, 5242880, 'uploads/x/source', 'ready')",
    )
    .bind(video)
    .bind(owner)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("insert into video_keys (video_id, wrapped_key) values ($1, $2)")
        .bind(video)
        .bind(vault().seal(video, key).unwrap())
        .execute(pool)
        .await
        .unwrap();
    video
}

async fn seed_session(pool: &PgPool, user: Uuid, video: Uuid) -> Seeded {
    let session = Uuid::new_v4();
    let expires_at =
        DateTime::from_timestamp((Utc::now() + Duration::hours(2)).timestamp(), 0).unwrap();
    sqlx::query(
        "insert into playback_sessions (id, user_id, video_id, expires_at) values ($1, $2, $3, $4)",
    )
    .bind(session)
    .bind(user)
    .bind(video)
    .bind(expires_at)
    .execute(pool)
    .await
    .unwrap();
    Seeded {
        user,
        video,
        session,
        expires_at,
    }
}

async fn seed(pool: &PgPool) -> Seeded {
    let user = Uuid::new_v4();
    sqlx::query("insert into users (id, name, email, password_hash, role) values ($1, 'Asha', $2, 'x', 'creator')")
        .bind(user)
        .bind(format!("{user}@x.in"))
        .execute(pool)
        .await
        .unwrap();
    let video = seed_video(pool, user, &KEY).await;
    seed_session(pool, user, video).await
}

struct Reply {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: Vec<u8>,
}

async fn get(app: &Router, uri: &str, origin: Option<&str>) -> Reply {
    let mut request = Request::get(uri);
    if let Some(origin) = origin {
        request = request.header(header::ORIGIN, origin);
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    Reply {
        status,
        headers,
        body,
    }
}

db_test! {
    async fn a_valid_request_gets_the_videos_key_and_nothing_may_cache_it(pool: PgPool) {
        let app = app(pool.clone());
        let s = seed(&pool).await;
        let reply = get(&app, &s.url(), Some(ORIGIN)).await;
        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.body, KEY);
        assert_eq!(reply.headers[header::CACHE_CONTROL], "no-store, private");
        assert_eq!(reply.headers[header::CONTENT_TYPE], "application/octet-stream");
        assert_eq!(reply.headers[header::ACCESS_CONTROL_ALLOW_ORIGIN], ORIGIN);

        // Native players send no Origin; they are judged on the token alone.
        let native = get(&app, &s.url(), None).await;
        assert_eq!((native.status, native.body.as_slice()), (StatusCode::OK, &KEY[..]));
        assert!(native.headers.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none());

        let fetches: i32 = sqlx::query_scalar("select key_fetches from playback_sessions where id = $1")
            .bind(s.session).fetch_one(&pool).await.unwrap();
        assert_eq!(fetches, 2);
    }
}

db_test! {
    async fn a_page_on_another_site_cannot_fetch_the_key_even_with_a_good_token(pool: PgPool) {
        let app = app(pool.clone());
        let s = seed(&pool).await;
        for origin in ["https://pirate.example", "http://localhost:3001", "null", "http://localhost:3000.evil.com"] {
            let reply = get(&app, &s.url(), Some(origin)).await;
            assert_eq!(reply.status, StatusCode::FORBIDDEN, "{origin}");
            assert!(reply.body.len() > 16 && !reply.body.windows(16).any(|w| w == KEY));
        }
    }
}

db_test! {
    async fn forged_expired_and_malformed_tokens_are_refused(pool: PgPool) {
        let app = app(pool.clone());
        let s = seed(&pool).await;
        let other_secret = PlaybackSigner::new("someone-elses-secret-that-is-long-enough").unwrap();
        let mut expired = s.claims();
        expired.expires_at = Utc::now() - Duration::seconds(1);
        let mut tampered = s.token();
        let last = tampered.pop().unwrap();
        tampered.push(if last == 'A' { 'B' } else { 'A' });

        for token in [
            other_secret.sign(&s.claims()),
            signer().sign(&expired),
            tampered,
            String::new(),
            "v1.garbage.garbage".into(),
        ] {
            let reply = get(&app, &format!("/v1/keys/{}?t={token}", s.video), Some(ORIGIN)).await;
            assert_eq!(reply.status, StatusCode::FORBIDDEN, "{token}");
        }
        assert_eq!(get(&app, &format!("/v1/keys/{}", s.video), Some(ORIGIN)).await.status, StatusCode::FORBIDDEN);
        assert_eq!(get(&app, &format!("/v1/keys/not-a-uuid?t={}", s.token()), Some(ORIGIN)).await.status, StatusCode::FORBIDDEN);
    }
}

db_test! {
    async fn a_token_only_opens_the_video_and_session_it_was_issued_for(pool: PgPool) {
        let app = app(pool.clone());
        let mine = seed(&pool).await;
        let theirs = seed(&pool).await;
        let my_second_video = seed_video(&pool, mine.user, b"another key 16b!").await;

        // My token, someone else's video in the path.
        let reply = get(&app, &format!("/v1/keys/{}?t={}", theirs.video, mine.token()), Some(ORIGIN)).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN);
        // My token, my other video.
        let reply = get(&app, &format!("/v1/keys/{my_second_video}?t={}", mine.token()), Some(ORIGIN)).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN);
        // A validly signed token naming their video but my session (or my user on their session).
        let mut mixed = mine.claims();
        mixed.video_id = theirs.video;
        let reply = get(&app, &format!("/v1/keys/{}?t={}", theirs.video, signer().sign(&mixed)), Some(ORIGIN)).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN);
        let mut borrowed = theirs.claims();
        borrowed.user_id = mine.user;
        let reply = get(&app, &format!("/v1/keys/{}?t={}", theirs.video, signer().sign(&borrowed)), Some(ORIGIN)).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN);
    }
}

db_test! {
    async fn ending_the_session_or_deleting_the_video_stops_key_delivery_at_once(pool: PgPool) {
        let app = app(pool.clone());
        let ended = seed(&pool).await;
        sqlx::query("update playback_sessions set ended_at = now(), end_reason = 'stopped' where id = $1")
            .bind(ended.session).execute(&pool).await.unwrap();
        assert_eq!(get(&app, &ended.url(), Some(ORIGIN)).await.status, StatusCode::FORBIDDEN);

        let lapsed = seed(&pool).await;
        // The token still says two hours, but the session's own expiry is what counts.
        sqlx::query("update playback_sessions set expires_at = now() - interval '1 second' where id = $1")
            .bind(lapsed.session).execute(&pool).await.unwrap();
        assert_eq!(get(&app, &lapsed.url(), Some(ORIGIN)).await.status, StatusCode::FORBIDDEN);

        let deleted = seed(&pool).await;
        sqlx::query("update videos set deleted_at = now() where id = $1")
            .bind(deleted.video).execute(&pool).await.unwrap();
        assert_eq!(get(&app, &deleted.url(), Some(ORIGIN)).await.status, StatusCode::FORBIDDEN);

        let failed = seed(&pool).await;
        sqlx::query("update videos set status = 'failed' where id = $1")
            .bind(failed.video).execute(&pool).await.unwrap();
        assert_eq!(get(&app, &failed.url(), Some(ORIGIN)).await.status, StatusCode::FORBIDDEN);
    }
}

db_test! {
    async fn a_session_that_keeps_fetching_the_key_is_cut_off(pool: PgPool) {
        let app = app(pool.clone());
        let s = seed(&pool).await;
        sqlx::query("update playback_sessions set key_fetches = $2 where id = $1")
            .bind(s.session).bind(MAX_KEY_FETCHES_PER_SESSION - 1).execute(&pool).await.unwrap();
        assert_eq!(get(&app, &s.url(), Some(ORIGIN)).await.status, StatusCode::OK);
        let reply = get(&app, &s.url(), Some(ORIGIN)).await;
        assert_eq!(reply.status, StatusCode::TOO_MANY_REQUESTS);
        assert!(!reply.body.windows(16).any(|w| w == KEY));
    }
}

db_test! {
    async fn a_key_sealed_under_another_secret_is_an_error_not_a_leak(pool: PgPool) {
        let app = app(pool.clone());
        let s = seed(&pool).await;
        let foreign = KeyVault::from_secret(&base64::engine::general_purpose::STANDARD.encode(*b"ZYXWVUTSRQPONMLKJIHGFEDCBA987654")).unwrap();
        sqlx::query("update video_keys set wrapped_key = $2 where video_id = $1")
            .bind(s.video).bind(foreign.seal(s.video, &KEY).unwrap()).execute(&pool).await.unwrap();
        let reply = get(&app, &s.url(), Some(ORIGIN)).await;
        assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(!reply.body.windows(16).any(|w| w == KEY));
    }
}

db_test! {
    async fn health_answers(pool: PgPool) {
        let reply = get(&app(pool), "/health", None).await;
        assert_eq!((reply.status, reply.body.as_slice()), (StatusCode::OK, &b"ok"[..]));
    }
}
