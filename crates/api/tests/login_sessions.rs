mod common;

use axum::http::StatusCode;
use chrono::{Duration, Utc};
use common::*;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;
use zeno_api::auth::tokens::TokenKeys;
use zeno_core::model::Role;

macro_rules! db_test {
    ($(#[$meta:meta])* async fn $name:ident($pool:ident: PgPool) $body:block) => {
        $(#[$meta])*
        #[sqlx::test(migrator = "zeno_core::db::MIGRATOR")]
        async fn $name($pool: PgPool) $body
    };
}

db_test! {
    async fn login_is_refused_until_the_email_is_verified(pool: PgPool) {
        let app = app(pool.clone());
        signup(&app, "a@x.in").await;
        let res = login(&app, "a@x.in", PASSWORD).await;
        assert_eq!(res.status, StatusCode::FORBIDDEN);
        assert_eq!(res.code(), "email_not_verified");
        assert!(res.set_cookies().is_empty());
    }
}

db_test! {
    async fn login_sets_httponly_lax_cookies_with_the_refresh_cookie_path_scoped(pool: PgPool) {
        let app = app(pool.clone());
        signup_verified(&app, "a@x.in").await;
        let res = login(&app, "a@x.in", PASSWORD).await;
        assert_eq!(res.status, StatusCode::OK);

        let cookies = res.set_cookies();
        let access = cookies.iter().find(|c| c.starts_with("zeno_at=")).unwrap();
        let refresh = cookies.iter().find(|c| c.starts_with("zeno_rt=")).unwrap();
        for cookie in [access, refresh] {
            assert!(cookie.contains("HttpOnly"), "{cookie}");
            assert!(cookie.contains("SameSite=Lax"), "{cookie}");
            assert!(!cookie.contains("Secure"), "dev config is http: {cookie}");
        }
        assert!(access.contains("Path=/;") || access.ends_with("Path=/"), "{access}");
        assert!(refresh.contains("Path=/v1/auth"), "{refresh}");

        let jar = Jar::of(&res);
        assert!(jar.header_for("/v1/auth/refresh").unwrap().contains("zeno_rt"));
        assert!(!jar.header_for("/v1/courses").unwrap().contains("zeno_rt"));
    }
}

db_test! {
    async fn cookies_are_secure_when_configured(pool: PgPool) {
        let app = build(pool.clone(), true, 0);
        signup_verified(&app, "a@x.in").await;
        let res = login(&app, "a@x.in", PASSWORD).await;
        assert!(res.set_cookies().iter().all(|c| c.contains("Secure")));
    }
}

db_test! {
    async fn wrong_password_and_unknown_user_are_indistinguishable(pool: PgPool) {
        let app = app(pool.clone());
        signup_verified(&app, "a@x.in").await;
        let wrong = login(&app, "a@x.in", "not the password").await;
        let unknown = login(&app, "ghost@x.in", "not the password").await;
        assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
        assert_eq!(wrong.status, unknown.status);
        assert_eq!(wrong.body, unknown.body);
        assert_eq!(wrong.code(), "invalid_credentials");
    }
}

db_test! {
    async fn login_ignores_email_case_and_surrounding_whitespace(pool: PgPool) {
        let app = app(pool.clone());
        signup_verified(&app, "a@x.in").await;
        assert_eq!(login(&app, "  A@X.IN ", PASSWORD).await.status, StatusCode::OK);
    }
}

db_test! {
    async fn absurdly_long_or_malformed_login_input_is_a_clean_rejection(pool: PgPool) {
        let app = app(pool.clone());
        let long_pw = login(&app, "a@x.in", &"p".repeat(10_000)).await;
        assert_eq!(long_pw.status, StatusCode::UNAUTHORIZED);
        let long_email = login(&app, &format!("{}@x.in", "a".repeat(400)), PASSWORD).await;
        assert_eq!(long_email.status, StatusCode::UNAUTHORIZED);

        let not_json = app.post("/v1/auth/login").raw_body(b"{nope".to_vec(), Some("application/json")).send().await;
        assert_eq!(not_json.status, StatusCode::BAD_REQUEST);
        assert_eq!(not_json.code(), "invalid_request");

        let no_content_type = app.post("/v1/auth/login").raw_body(b"{}".to_vec(), None).send().await;
        assert_eq!(no_content_type.status, StatusCode::UNSUPPORTED_MEDIA_TYPE);

        let extra = app.post("/v1/auth/login")
            .json(json!({"email":"a@x.in","password":"x","admin":true}))
            .send()
            .await;
        assert_eq!(extra.status, StatusCode::UNPROCESSABLE_ENTITY);
    }
}

db_test! {
    async fn five_failures_lock_further_attempts_even_with_the_right_password(pool: PgPool) {
        let app = app(pool.clone());
        signup_verified(&app, "a@x.in").await;
        for _ in 0..5 {
            assert_eq!(login(&app, "a@x.in", "wrong").await.status, StatusCode::UNAUTHORIZED);
        }
        let locked = login(&app, "a@x.in", PASSWORD).await;
        assert_eq!(locked.status, StatusCode::TOO_MANY_REQUESTS);
        assert!(locked.headers.get("retry-after").is_some());
        assert!(locked.set_cookies().is_empty());
    }
}

db_test! {
    async fn a_stranger_cannot_lock_the_owner_out_from_another_address(pool: PgPool) {
        let app = build(pool.clone(), false, 1);
        signup_verified(&app, "a@x.in").await;
        for _ in 0..5 {
            let res = app.post("/v1/auth/login")
                .header("x-forwarded-for", "6.6.6.6")
                .json(json!({"email":"a@x.in","password":"wrong"}))
                .send()
                .await;
            assert_eq!(res.status, StatusCode::UNAUTHORIZED);
        }
        let attacker = app.post("/v1/auth/login")
            .header("x-forwarded-for", "6.6.6.6")
            .json(json!({"email":"a@x.in","password":PASSWORD}))
            .send()
            .await;
        assert_eq!(attacker.status, StatusCode::TOO_MANY_REQUESTS);

        let owner = app.post("/v1/auth/login")
            .header("x-forwarded-for", "203.0.113.7")
            .json(json!({"email":"a@x.in","password":PASSWORD}))
            .send()
            .await;
        assert_eq!(owner.status, StatusCode::OK);
    }
}

db_test! {
    async fn a_forged_forwarded_for_prefix_cannot_dodge_the_lockout(pool: PgPool) {
        let app = build(pool.clone(), false, 1);
        signup_verified(&app, "a@x.in").await;
        for i in 0..5 {
            let res = app.post("/v1/auth/login")
                .header("x-forwarded-for", &format!("10.0.0.{i}, 6.6.6.6"))
                .json(json!({"email":"a@x.in","password":"wrong"}))
                .send()
                .await;
            assert_eq!(res.status, StatusCode::UNAUTHORIZED);
        }
        let res = app.post("/v1/auth/login")
            .header("x-forwarded-for", "10.0.0.99, 6.6.6.6")
            .json(json!({"email":"a@x.in","password":PASSWORD}))
            .send()
            .await;
        assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS);
    }
}

db_test! {
    async fn a_successful_login_resets_the_failure_count(pool: PgPool) {
        let app = app(pool.clone());
        signup_verified(&app, "a@x.in").await;
        for _ in 0..4 {
            login(&app, "a@x.in", "wrong").await;
        }
        assert_eq!(login(&app, "a@x.in", PASSWORD).await.status, StatusCode::OK);
        for _ in 0..4 {
            assert_eq!(login(&app, "a@x.in", "wrong").await.status, StatusCode::UNAUTHORIZED);
        }
        assert_eq!(login(&app, "a@x.in", PASSWORD).await.status, StatusCode::OK);
    }
}

db_test! {
    async fn me_rejects_missing_forged_expired_and_wrongly_signed_tokens(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_verified(&app, "a@x.in").await;
        assert_eq!(app.get("/v1/auth/me").jar(&jar).send().await.status, StatusCode::OK);

        let none = app.get("/v1/auth/me").send().await;
        assert_eq!(none.status, StatusCode::UNAUTHORIZED);
        assert_eq!(none.code(), "unauthenticated");

        let garbage = app.get("/v1/auth/me").cookie_header("zeno_at=garbage").send().await;
        assert_eq!(garbage.status, StatusCode::UNAUTHORIZED);

        let (user, session) = (Uuid::new_v4(), Uuid::new_v4());
        let expired = TokenKeys::new(SECRET.as_bytes())
            .issue_access(user, Role::Admin, session, Utc::now() - Duration::hours(1))
            .unwrap()
            .0;
        let res = app.get("/v1/auth/me").cookie_header(&format!("zeno_at={expired}")).send().await;
        assert_eq!(res.status, StatusCode::UNAUTHORIZED);

        let forged = TokenKeys::new(b"an-attacker-guessed-this-secret-xx")
            .issue_access(user, Role::Admin, session, Utc::now())
            .unwrap()
            .0;
        let res = app.get("/v1/auth/me").cookie_header(&format!("zeno_at={forged}")).send().await;
        assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    }
}

db_test! {
    async fn the_refresh_token_is_not_accepted_as_an_access_token(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_verified(&app, "a@x.in").await;
        let rt = jar.get("zeno_rt").unwrap();
        let res = app.get("/v1/auth/me").cookie_header(&format!("zeno_at={rt}")).send().await;
        assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    }
}

db_test! {
    async fn refresh_rotates_the_token_and_reuse_after_the_grace_window_kills_the_family(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_verified(&app, "a@x.in").await;
        let old_rt = jar.get("zeno_rt").unwrap().to_owned();

        let refreshed = app.post("/v1/auth/refresh").jar(&jar).send().await;
        assert_eq!(refreshed.status, StatusCode::NO_CONTENT);
        let mut fresh = jar.clone();
        fresh.apply(&refreshed);
        assert_ne!(fresh.get("zeno_rt").unwrap(), old_rt);
        assert_eq!(app.get("/v1/auth/me").jar(&fresh).send().await.status, StatusCode::OK);

        // Replaying the old token straight away looks like a double-submit from two tabs:
        // refused, but the live session and the browser's cookies are left alone.
        let raced = app.post("/v1/auth/refresh").jar(&jar).send().await;
        assert_eq!(raced.status, StatusCode::UNAUTHORIZED);
        assert_eq!(raced.code(), "refresh_conflict");
        assert!(raced.set_cookies().is_empty());
        assert_eq!(app.post("/v1/auth/refresh").jar(&fresh).send().await.status, StatusCode::NO_CONTENT);

        // Replaying it later means it was copied: everyone in the family is signed out.
        sqlx::query("update sessions set rotated_at = now() - interval '1 minute' where rotated_at is not null")
            .execute(&pool)
            .await
            .unwrap();
        let stolen = app.post("/v1/auth/refresh").jar(&jar).send().await;
        assert_eq!(stolen.status, StatusCode::UNAUTHORIZED);
        assert_eq!(stolen.code(), "session_expired");
        assert!(stolen.set_cookies().iter().any(|c| c.contains("Max-Age=0")));

        let victim = app.post("/v1/auth/refresh").jar(&fresh).send().await;
        assert_eq!(victim.status, StatusCode::UNAUTHORIZED);
        assert_eq!(count(&pool, "select count(*) from sessions where revoked_at is null").await, 0);
    }
}

db_test! {
    async fn simultaneous_refreshes_with_one_token_yield_a_single_new_session(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_verified(&app, "a@x.in").await;
        let handles: Vec<_> = (0..6)
            .map(|_| tokio::spawn(app.post("/v1/auth/refresh").jar(&jar).send()))
            .collect();
        let mut statuses = vec![];
        for handle in handles {
            statuses.push(handle.await.unwrap().status);
        }
        assert_eq!(statuses.iter().filter(|s| **s == StatusCode::NO_CONTENT).count(), 1, "{statuses:?}");
        assert_eq!(count(&pool, "select count(*) from sessions").await, 2);
        assert_eq!(count(&pool, "select count(*) from sessions where rotated_at is null").await, 1);
    }
}

db_test! {
    async fn refresh_without_a_cookie_is_unauthorised(pool: PgPool) {
        let app = app(pool.clone());
        let res = app.post("/v1/auth/refresh").send().await;
        assert_eq!(res.status, StatusCode::UNAUTHORIZED);
        let junk = app.post("/v1/auth/refresh").cookie_header("zeno_rt=not-a-real-token").send().await;
        assert_eq!(junk.status, StatusCode::UNAUTHORIZED);
        assert_eq!(junk.code(), "session_expired");
    }
}

db_test! {
    async fn expired_and_over_age_sessions_cannot_refresh(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_verified(&app, "a@x.in").await;

        sqlx::query("update sessions set expires_at = now() - interval '1 second'")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(app.post("/v1/auth/refresh").jar(&jar).send().await.status, StatusCode::UNAUTHORIZED);

        let jar = login(&app, "a@x.in", PASSWORD).await;
        let jar = Jar::of(&jar);
        sqlx::query("update sessions set absolute_expires_at = now() - interval '1 second' where expires_at > now()")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(app.post("/v1/auth/refresh").jar(&jar).send().await.status, StatusCode::UNAUTHORIZED);
    }
}

db_test! {
    async fn a_rotated_session_never_outlives_its_absolute_lifetime(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_verified(&app, "a@x.in").await;
        sqlx::query("update sessions set absolute_expires_at = now() + interval '2 days'")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(app.post("/v1/auth/refresh").jar(&jar).send().await.status, StatusCode::NO_CONTENT);
        let over_cap: i64 = count(
            &pool,
            "select count(*) from sessions where rotated_at is null and expires_at > absolute_expires_at",
        )
        .await;
        assert_eq!(over_cap, 0);
    }
}

db_test! {
    async fn logout_revokes_the_session_and_clears_cookies(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_verified(&app, "a@x.in").await;
        let res = app.post("/v1/auth/logout").jar(&jar).send().await;
        assert_eq!(res.status, StatusCode::NO_CONTENT);
        let cleared = Jar::of(&res);
        assert!(cleared.get("zeno_at").is_none() && cleared.get("zeno_rt").is_none());
        assert!(res.set_cookies().iter().all(|c| c.contains("Max-Age=0")));

        assert_eq!(app.post("/v1/auth/refresh").jar(&jar).send().await.status, StatusCode::UNAUTHORIZED);
        assert_eq!(count(&pool, "select count(*) from sessions where revoked_at is null").await, 0);
    }
}

db_test! {
    async fn logout_is_idempotent_and_needs_no_session(pool: PgPool) {
        let app = app(pool.clone());
        assert_eq!(app.post("/v1/auth/logout").send().await.status, StatusCode::NO_CONTENT);
        let jar = signup_verified(&app, "a@x.in").await;
        assert_eq!(app.post("/v1/auth/logout").jar(&jar).send().await.status, StatusCode::NO_CONTENT);
        assert_eq!(app.post("/v1/auth/logout").jar(&jar).send().await.status, StatusCode::NO_CONTENT);
    }
}

db_test! {
    async fn logging_out_one_device_leaves_other_devices_signed_in(pool: PgPool) {
        let app = app(pool.clone());
        let laptop = signup_verified(&app, "a@x.in").await;
        let phone = Jar::of(&login(&app, "a@x.in", PASSWORD).await);
        app.post("/v1/auth/logout").jar(&laptop).send().await;
        assert_eq!(app.post("/v1/auth/refresh").jar(&phone).send().await.status, StatusCode::NO_CONTENT);
    }
}

db_test! {
    async fn session_rows_record_device_details_without_storing_the_raw_token(pool: PgPool) {
        let app = app(pool.clone());
        signup_verified(&app, "a@x.in").await;
        let res = app.post("/v1/auth/login")
            .header("user-agent", "ZenoTest/1.0")
            .json(json!({"email":"a@x.in","password":PASSWORD}))
            .send()
            .await;
        let jar = Jar::of(&res);
        let ua: String = sqlx::query_scalar("select user_agent from sessions order by created_at desc limit 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(ua, "ZenoTest/1.0");
        let raw = jar.get("zeno_rt").unwrap();
        let stored: Vec<u8> = sqlx::query_scalar("select refresh_token_hash from sessions order by created_at desc limit 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(stored.len(), 32);
        assert_ne!(stored, raw.as_bytes());
    }
}
