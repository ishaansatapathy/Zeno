mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use zeno_api::auth::repo;

macro_rules! db_test {
    ($(#[$meta:meta])* async fn $name:ident($pool:ident: PgPool) $body:block) => {
        $(#[$meta])*
        #[sqlx::test(migrator = "zeno_core::db::MIGRATOR")]
        async fn $name($pool: PgPool) $body
    };
}

/// Event kinds with their counts, sorted by kind. Events are written in the background, so
/// their insertion order is not guaranteed; counts are.
async fn kinds(pool: &PgPool) -> Vec<(String, i64)> {
    sqlx::query_as("select kind, count(*) from auth_events group by kind order by kind")
        .fetch_all(pool)
        .await
        .unwrap()
}

fn kind(name: &str, n: i64) -> (String, i64) {
    (name.to_owned(), n)
}

db_test! {
    async fn an_account_lifecycle_leaves_a_trail(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_verified(&app, "a@x.in").await;
        login(&app, "a@x.in", "wrong password here").await;
        login(&app, "a@x.in", PASSWORD).await;
        app.post("/v1/auth/logout").jar(&jar).send().await;
        app.settle().await;

        assert_eq!(
            kinds(&pool).await,
            vec![
                kind("email_verified", 1),
                kind("logged_out", 1),
                kind("login_failed", 1),
                kind("login_succeeded", 1),
                kind("signed_up", 1),
            ]
        );
        let owner: i64 = count(&pool, "select count(*) from auth_events where user_id is not null").await;
        assert_eq!(owner, 5, "every event above belongs to the account");
    }
}

db_test! {
    async fn failed_logins_for_unknown_emails_store_only_a_hash_with_where_they_came_from(pool: PgPool) {
        let app = build(pool.clone(), false, 1);
        app.post("/v1/auth/login")
            .header("x-forwarded-for", "203.0.113.9")
            .header("user-agent", "AttackerBot/9")
            .json(json!({"email":"Ghost@X.in","password":"whatever"}))
            .send()
            .await;
        app.settle().await;

        let row: (Option<uuid::Uuid>, Option<Vec<u8>>, String, String) = sqlx::query_as(
            "select user_id, email_hash, host(ip), user_agent from auth_events",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.0, None);
        assert_eq!(row.1.unwrap(), Sha256::digest(b"ghost@x.in").to_vec(), "normalised, then hashed");
        assert_eq!(row.2, "203.0.113.9");
        assert_eq!(row.3, "AttackerBot/9");
    }
}

db_test! {
    async fn a_lockout_is_visible_as_failures_then_a_block(pool: PgPool) {
        let app = app(pool.clone());
        signup_verified(&app, "a@x.in").await;
        for _ in 0..6 {
            login(&app, "a@x.in", "wrong password here").await;
        }
        app.settle().await;
        let counts = kinds(&pool).await;
        assert!(counts.contains(&kind("login_failed", 5)), "{counts:?}");
        assert!(counts.contains(&kind("login_blocked", 1)), "{counts:?}");
    }
}

db_test! {
    async fn signing_in_before_verifying_is_recorded_against_the_account(pool: PgPool) {
        let app = app(pool.clone());
        signup(&app, "a@x.in").await;
        login(&app, "a@x.in", PASSWORD).await;
        app.settle().await;
        let n: i64 = count(
            &pool,
            "select count(*) from auth_events e join users u on u.id = e.user_id \
             where e.kind = 'login_unverified' and u.email = 'a@x.in'",
        )
        .await;
        assert_eq!(n, 1);
    }
}

db_test! {
    async fn token_theft_is_recorded_with_the_session_family(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_verified(&app, "a@x.in").await;
        app.post("/v1/auth/refresh").jar(&jar).send().await;
        sqlx::query("update sessions set rotated_at = now() - interval '1 minute' where rotated_at is not null")
            .execute(&pool)
            .await
            .unwrap();
        app.post("/v1/auth/refresh").jar(&jar).send().await;
        app.settle().await;

        let (family,): (uuid::Uuid,) = sqlx::query_as("select family_id from auth_events where kind = 'refresh_reuse_detected'")
            .fetch_one(&pool)
            .await
            .unwrap();
        let session_family: uuid::Uuid = sqlx::query_scalar("select family_id from sessions limit 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(family, session_family);
    }
}

db_test! {
    async fn password_resets_are_recorded_but_unknown_addresses_leave_no_trace(pool: PgPool) {
        let app = app(pool.clone());
        signup_verified(&app, "a@x.in").await;
        app.post("/v1/auth/forgot-password").json(json!({"email":"ghost@x.in"})).send().await;
        app.post("/v1/auth/forgot-password").json(json!({"email":"a@x.in"})).send().await;
        let token = token_from(&app.mailer.wait_for(2).await[1]);
        app.post("/v1/auth/reset-password")
            .json(json!({"token": token, "new_password": "a-completely-new-passphrase"}))
            .send()
            .await;
        app.settle().await;

        let counts = kinds(&pool).await;
        assert!(counts.contains(&kind("password_reset_requested", 1)), "{counts:?}");
        assert!(counts.contains(&kind("password_reset_completed", 1)), "{counts:?}");
    }
}

db_test! {
    async fn activity_shows_only_my_own_events_newest_first(pool: PgPool) {
        let app = build(pool.clone(), false, 1);
        let mine = signup_verified(&app, "me@x.in").await;
        signup_verified(&app, "other@x.in").await;
        app.post("/v1/auth/login")
            .header("x-forwarded-for", "198.51.100.4")
            .header("user-agent", "Chrome-on-my-phone")
            .json(json!({"email":"me@x.in","password":PASSWORD}))
            .send()
            .await;
        app.settle().await;

        let res = app.get("/v1/auth/activity").jar(&mine).send().await;
        assert_eq!(res.status, StatusCode::OK);
        let events = res.body["events"].as_array().unwrap();
        assert!(!events.is_empty());
        assert_eq!(events[0]["kind"], "login_succeeded");
        assert_eq!(events[0]["ip"], "198.51.100.4");
        assert_eq!(events[0]["user_agent"], "Chrome-on-my-phone");
        let times: Vec<&str> = events.iter().map(|e| e["at"].as_str().unwrap()).collect();
        assert!(times.windows(2).all(|w| w[0] >= w[1]), "{times:?}");

        let mine_count: i64 = count(
            &pool,
            "select count(*) from auth_events e join users u on u.id = e.user_id where u.email = 'me@x.in'",
        )
        .await;
        assert_eq!(events.len() as i64, mine_count, "nothing from other.x.in leaks in");
        assert!(events.iter().all(|e| e.get("user_id").is_none() && e.get("email_hash").is_none()));
    }
}

db_test! {
    async fn activity_requires_signing_in_and_is_capped_at_twenty(pool: PgPool) {
        let app = app(pool.clone());
        assert_eq!(app.get("/v1/auth/activity").send().await.status, StatusCode::UNAUTHORIZED);

        let jar = signup_verified(&app, "a@x.in").await;
        app.settle().await;
        sqlx::query(
            "insert into auth_events (user_id, kind) select id, 'login_succeeded' from users, generate_series(1, 30)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let res = app.get("/v1/auth/activity").jar(&jar).send().await;
        assert_eq!(res.body["events"].as_array().unwrap().len(), 20);
    }
}

db_test! {
    async fn a_broken_audit_log_never_breaks_sign_in(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_verified(&app, "a@x.in").await;
        app.settle().await;
        sqlx::query("drop table auth_events").execute(&pool).await.unwrap();

        assert_eq!(login(&app, "a@x.in", PASSWORD).await.status, StatusCode::OK);
        assert_eq!(login(&app, "a@x.in", "wrong password here").await.status, StatusCode::UNAUTHORIZED);
        assert_eq!(app.post("/v1/auth/logout").jar(&jar).send().await.status, StatusCode::NO_CONTENT);
        app.settle().await;
    }
}

db_test! {
    async fn deleting_an_account_keeps_the_trail_but_drops_the_link(pool: PgPool) {
        let app = app(pool.clone());
        signup_verified(&app, "a@x.in").await;
        app.settle().await;
        let before = count(&pool, "select count(*) from auth_events").await;
        assert!(before > 0);

        sqlx::query("delete from users").execute(&pool).await.unwrap();

        assert_eq!(count(&pool, "select count(*) from auth_events").await, before);
        assert_eq!(count(&pool, "select count(*) from auth_events where user_id is not null").await, 0);
    }
}

db_test! {
    async fn old_events_and_delivered_mail_are_purged_recent_ones_kept(pool: PgPool) {
        sqlx::query("insert into auth_events (kind, created_at) values ('login_failed', now() - interval '400 days'), ('login_failed', now() - interval '30 days')")
            .execute(&pool).await.unwrap();
        sqlx::query(
            "insert into email_outbox (to_address, subject, body, expires_at, sent_at) values \
             ('a@x.in', 's', '', now(), now() - interval '10 days'), \
             ('b@x.in', 's', '', now(), now() - interval '1 day'), \
             ('c@x.in', 's', 'pending link', now() + interval '1 hour', null)",
        )
        .execute(&pool).await.unwrap();

        let purged = repo::purge_expired(&pool).await.unwrap();

        assert_eq!((purged.events, purged.emails), (1, 1));
        assert_eq!(count(&pool, "select count(*) from auth_events").await, 1);
        assert_eq!(count(&pool, "select count(*) from email_outbox").await, 2);
    }
}
