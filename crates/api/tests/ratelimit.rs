//! Request budgets: per client address for public routes, per account for signed-in actions,
//! and what each policy does when the counter store is down.

mod common;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::http::{header, Method, StatusCode};
use common::*;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;
use zeno_core::ratelimit::RateLimiter;

macro_rules! db_test {
    ($(#[$meta:meta])* async fn $name:ident($pool:ident: PgPool) $body:block) => {
        #[sqlx::test(migrator = "zeno_core::db::MIGRATOR")]
        $(#[$meta])*
        async fn $name($pool: PgPool) $body
    };
}

/// An app whose limiter uses a clock the test moves by hand.
fn app_with_limiter(pool: PgPool, hops: usize, limiter: RateLimiter) -> App {
    let mailer = zeno_api::mail::MemoryMailer::default();
    let state =
        make_state(pool.clone(), false, hops, Arc::new(mailer.clone())).with_limiter(limiter);
    App {
        router: zeno_api::build_app(state.clone()),
        mailer,
        pool,
        state,
    }
}

struct Clock(Arc<AtomicU64>);

impl Clock {
    fn limiter(start_ms: u64) -> (Self, RateLimiter) {
        let cell = Arc::new(AtomicU64::new(start_ms));
        let reader = cell.clone();
        (
            Self(cell),
            RateLimiter::memory_with_clock(move || reader.load(Ordering::SeqCst)),
        )
    }

    fn advance_secs(&self, secs: u64) {
        self.0.fetch_add(secs * 1000, Ordering::SeqCst);
    }
}

/// A multiple of every window used, so tests start at a window boundary.
const T0: u64 = 1_000_000_000_000 - (1_000_000_000_000 % 86_400_000);

async fn browse(app: &App, client: &str) -> axum::http::StatusCode {
    app.get("/v1/public/courses")
        .header("x-forwarded-for", client)
        .send()
        .await
        .status
}

db_test! {
    async fn the_public_catalogue_is_limited_per_client_address(pool: PgPool) {
        let (clock, limiter) = Clock::limiter(T0);
        let app = app_with_limiter(pool, 1, limiter);

        for i in 0..120 {
            assert_eq!(browse(&app, "203.0.113.7").await, StatusCode::OK, "request {i}");
        }
        let refused = app.get("/v1/public/courses").header("x-forwarded-for", "203.0.113.7").send().await;
        assert_eq!(refused.status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(refused.code(), "too_many_requests");
        let retry: u64 = refused.headers[header::RETRY_AFTER].to_str().unwrap().parse().unwrap();
        assert!((1..=61).contains(&retry), "{retry}");

        // Another visitor is unaffected, and so is a different route that is not limited.
        assert_eq!(browse(&app, "203.0.113.8").await, StatusCode::OK);
        assert_eq!(app.get("/health").send().await.status, StatusCode::OK);

        // Time heals it.
        clock.advance_secs(61);
        assert_eq!(browse(&app, "203.0.113.7").await, StatusCode::OK);
    }
}

db_test! {
    async fn a_forged_forwarded_header_cannot_buy_a_fresh_budget(pool: PgPool) {
        let (_clock, limiter) = Clock::limiter(T0);
        let app = app_with_limiter(pool, 1, limiter);
        // With one trusted proxy only the rightmost entry is believed; everything an attacker
        // prepends is ignored, so rotating it changes nothing.
        for i in 0..120 {
            let forged = format!("10.9.{}.{}, 198.51.100.4", i / 200, i % 200);
            assert_eq!(browse(&app, &forged).await, StatusCode::OK);
        }
        assert_eq!(browse(&app, "10.1.1.1, 198.51.100.4").await, StatusCode::TOO_MANY_REQUESTS);
    }
}

db_test! {
    async fn ipv6_users_share_one_budget_per_slash_64(pool: PgPool) {
        let (_clock, limiter) = Clock::limiter(T0);
        let app = app_with_limiter(pool, 1, limiter);
        for i in 0..120 {
            let address = format!("2001:db8:1:2:{i:x}::1");
            assert_eq!(browse(&app, &address).await, StatusCode::OK);
        }
        assert_eq!(browse(&app, "2001:db8:1:2:ffff::9").await, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(browse(&app, "2001:db8:1:3::1").await, StatusCode::OK, "a different /64");
    }
}

db_test! {
    async fn heartbeats_and_enrollment_are_limited_per_account(pool: PgPool) {
        let (_clock, limiter) = Clock::limiter(T0);
        let app = app_with_limiter(pool, 0, limiter);
        let asha = signup_verified_as(&app, "asha-rl@x.in", "student").await;
        let ravi = signup_verified_as(&app, "ravi-rl@x.in", "student").await;
        let session = Uuid::new_v4();
        let beat = |jar: &Jar| {
            app.post(&format!("/v1/play/{session}/heartbeat"))
                .jar(jar)
                .json(json!({"position_seconds": 10}))
                .send()
        };

        for i in 0..30 {
            // No such session: 404, which still proves the request got past the limiter.
            assert_eq!(beat(&asha).await.status, StatusCode::NOT_FOUND, "beat {i}");
        }
        let refused = beat(&asha).await;
        assert_eq!(refused.status, StatusCode::TOO_MANY_REQUESTS);
        assert!(refused.headers.contains_key(header::RETRY_AFTER));
        assert_eq!(beat(&ravi).await.status, StatusCode::NOT_FOUND, "another account is unaffected");

        let missing = Uuid::new_v4();
        for i in 0..20 {
            let r = app.post(&format!("/v1/courses/{missing}/enroll")).jar(&asha).send().await;
            assert_eq!(r.status, StatusCode::NOT_FOUND, "enroll {i}");
        }
        let r = app.post(&format!("/v1/courses/{missing}/enroll")).jar(&asha).send().await;
        assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);

        for i in 0..20 {
            let r = app.post(&format!("/v1/videos/{missing}/play")).jar(&ravi).send().await;
            assert_ne!(r.status, StatusCode::TOO_MANY_REQUESTS, "play {i}");
        }
        let r = app.post(&format!("/v1/videos/{missing}/play")).jar(&ravi).send().await;
        assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
    }
}

db_test! {
    async fn when_the_counter_store_is_down_each_policy_does_what_it_promised(pool: PgPool) {
        let app = app_with_limiter(pool, 0, RateLimiter::failing());
        let asha = signup_verified_as(&app, "down-rl@x.in", "student").await;
        let missing = Uuid::new_v4();

        // Open: the catalogue and heartbeats keep working.
        assert_eq!(app.get("/v1/public/courses").send().await.status, StatusCode::OK);
        let beat = app.post(&format!("/v1/play/{missing}/heartbeat")).jar(&asha)
            .json(json!({"position_seconds": 1})).send().await;
        assert_eq!(beat.status, StatusCode::NOT_FOUND, "passed the limiter");

        // Closed: starting playback and enrolling are refused rather than left unlimited.
        let play = app.post(&format!("/v1/lessons/{missing}/play")).jar(&asha).send().await;
        assert_eq!((play.status, play.code()), (StatusCode::SERVICE_UNAVAILABLE, "unavailable"));
        let enroll = app.post(&format!("/v1/courses/{missing}/enroll")).jar(&asha).send().await;
        assert_eq!(enroll.status, StatusCode::SERVICE_UNAVAILABLE);
    }
}

db_test! {
    async fn limiter_failures_do_not_leak_into_auth_or_health(pool: PgPool) {
        let app = app_with_limiter(pool, 0, RateLimiter::failing());
        assert_eq!(app.get("/health").send().await.status, StatusCode::OK);
        let r = app.req(Method::POST, "/v1/auth/login")
            .json(json!({"email": "nobody@x.in", "password": PASSWORD})).send().await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED, "{:?}", r.body);
    }
}
