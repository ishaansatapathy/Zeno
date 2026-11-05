mod common;

use axum::http::{header, StatusCode};
use common::*;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;
use zeno_api::auth::extractor::AuthUser;
use zeno_core::model::Role;

macro_rules! db_test {
    ($(#[$meta:meta])* async fn $name:ident($pool:ident: PgPool) $body:block) => {
        $(#[$meta])*
        #[sqlx::test(migrator = "zeno_core::db::MIGRATOR")]
        async fn $name($pool: PgPool) $body
    };
}

db_test! {
    async fn cross_origin_state_changing_requests_are_blocked(pool: PgPool) {
        let app = app(pool.clone());
        let body = json!({"email":"a@x.in","password":"whatever"});

        let evil = app.post("/v1/auth/login").origin("https://evil.example").json(body.clone()).send().await;
        assert_eq!(evil.status, StatusCode::FORBIDDEN);
        assert_eq!(evil.code(), "forbidden_origin");

        let lookalike = app.post("/v1/auth/login").origin("http://localhost:3000.evil.example").json(body.clone()).send().await;
        assert_eq!(lookalike.status, StatusCode::FORBIDDEN);

        let wrong_scheme = app.post("/v1/auth/login").origin("https://localhost:3000").json(body.clone()).send().await;
        assert_eq!(wrong_scheme.status, StatusCode::FORBIDDEN);

        let same = app.post("/v1/auth/login").json(body.clone()).send().await;
        assert_eq!(same.status, StatusCode::UNAUTHORIZED);

        let no_origin = app.post("/v1/auth/login").no_origin().json(body.clone()).send().await;
        assert_eq!(no_origin.status, StatusCode::UNAUTHORIZED);
    }
}

db_test! {
    async fn browsers_that_declare_a_cross_site_request_are_blocked_even_without_origin(pool: PgPool) {
        let app = app(pool.clone());
        let res = app.post("/v1/auth/logout")
            .no_origin()
            .header("sec-fetch-site", "cross-site")
            .send()
            .await;
        assert_eq!(res.status, StatusCode::FORBIDDEN);
        let ok = app.post("/v1/auth/logout")
            .no_origin()
            .header("sec-fetch-site", "same-origin")
            .send()
            .await;
        assert_eq!(ok.status, StatusCode::NO_CONTENT);
    }
}

db_test! {
    async fn a_cross_origin_logout_cannot_end_someone_elses_session(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_verified(&app, "a@x.in").await;
        let res = app.post("/v1/auth/logout").origin("https://evil.example").jar(&jar).send().await;
        assert_eq!(res.status, StatusCode::FORBIDDEN);
        assert_eq!(app.post("/v1/auth/refresh").jar(&jar).send().await.status, StatusCode::NO_CONTENT);
    }
}

db_test! {
    async fn cors_allows_only_the_web_app_origin_with_credentials(pool: PgPool) {
        let app = app(pool.clone());
        let ok = app.req(axum::http::Method::OPTIONS, "/v1/auth/login")
            .header("origin", ORIGIN)
            .header("access-control-request-method", "POST")
            .header("access-control-request-headers", "content-type")
            .send()
            .await;
        assert_eq!(ok.headers.get("access-control-allow-origin").unwrap(), ORIGIN);
        assert_eq!(ok.headers.get("access-control-allow-credentials").unwrap(), "true");

        let evil = app.req(axum::http::Method::OPTIONS, "/v1/auth/login")
            .header("origin", "https://evil.example")
            .header("access-control-request-method", "POST")
            .send()
            .await;
        // The allowed origin is fixed, so a browser on the evil origin sees a mismatch and
        // refuses to expose the response; what must never happen is echoing the evil origin.
        assert_ne!(
            evil.headers.get("access-control-allow-origin").map(|v| v.as_bytes()),
            Some(b"https://evil.example".as_slice())
        );
    }
}

db_test! {
    async fn oversized_bodies_are_rejected_before_parsing(pool: PgPool) {
        let app = app(pool.clone());
        let res = app.post("/v1/auth/register")
            .json(json!({"name": "x".repeat(100_000), "email": "a@x.in", "password": PASSWORD}))
            .send()
            .await;
        assert_eq!(res.status, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(res.code(), "payload_too_large");
    }
}

db_test! {
    async fn errors_are_json_and_never_leak_internals(pool: PgPool) {
        let app = app(pool.clone());
        let res = app.get("/v1/auth/me").send().await;
        assert_eq!(res.headers.get(header::CONTENT_TYPE).unwrap(), "application/json");
        assert!(res.body["error"]["code"].is_string());
        let text = res.body.to_string();
        assert!(!text.contains("sqlx") && !text.contains("panicked") && !text.contains("src/"));
    }
}

db_test! {
    async fn health_is_cheap_and_ready_checks_dependencies(pool: PgPool) {
        let app = app(pool.clone());
        assert_eq!(app.get("/health").send().await.status, StatusCode::OK);
        let ready = app.get("/ready").send().await;
        assert_eq!(ready.status, StatusCode::OK);
        assert_eq!(ready.body["status"], "ready");

        let broken = app.pool.clone();
        broken.close().await;
        assert_eq!(app.get("/health").send().await.status, StatusCode::OK);
        let down = app.get("/ready").send().await;
        assert_eq!(down.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(down.code(), "unavailable");
    }
}

db_test! {
    async fn unbuilt_endpoints_answer_501_and_unknown_ones_404(pool: PgPool) {
        let app = app(pool.clone());
        assert_eq!(app.post("/v1/courses").json(json!({})).send().await.status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(app.get("/v1/nope").send().await.status, StatusCode::NOT_FOUND);
    }
}

#[test]
fn role_guard_allows_only_listed_roles() {
    let user = |role| AuthUser {
        id: Uuid::new_v4(),
        role,
        session_id: Uuid::new_v4(),
    };
    assert!(user(Role::Creator)
        .require_role(&[Role::Creator, Role::Admin])
        .is_ok());
    assert!(user(Role::Admin)
        .require_role(&[Role::Creator, Role::Admin])
        .is_ok());
    assert!(user(Role::Student)
        .require_role(&[Role::Creator, Role::Admin])
        .is_err());
    assert!(user(Role::Student).require_role(&[]).is_err());
}
