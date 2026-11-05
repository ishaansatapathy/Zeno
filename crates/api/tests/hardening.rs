mod common;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::http::StatusCode;
use common::*;
use serde_json::json;
use sqlx::PgPool;
use tokio::sync::Semaphore;
use zeno_api::auth::repo;
use zeno_api::mail::{Mailer, MemoryMailer, OutgoingEmail};

macro_rules! db_test {
    ($(#[$meta:meta])* async fn $name:ident($pool:ident: PgPool) $body:block) => {
        $(#[$meta])*
        #[sqlx::test(migrator = "zeno_core::db::MIGRATOR")]
        async fn $name($pool: PgPool) $body
    };
}

async fn me_status(app: &App, jar: &Jar) -> StatusCode {
    app.get("/v1/auth/me").jar(jar).send().await.status
}

db_test! {
    async fn logout_kills_the_access_token_immediately_not_after_fifteen_minutes(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_verified(&app, "a@x.in").await;
        assert_eq!(me_status(&app, &jar).await, StatusCode::OK);

        app.post("/v1/auth/logout").jar(&jar).send().await;

        assert_eq!(me_status(&app, &jar).await, StatusCode::UNAUTHORIZED);
    }
}

db_test! {
    async fn logout_still_ends_the_session_when_the_refresh_cookie_is_missing(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_verified(&app, "a@x.in").await;
        let access_only = format!("zeno_at={}", jar.get("zeno_at").unwrap());

        app.post("/v1/auth/logout").cookie_header(&access_only).send().await;

        assert_eq!(me_status(&app, &jar).await, StatusCode::UNAUTHORIZED);
        assert_eq!(app.post("/v1/auth/refresh").jar(&jar).send().await.status, StatusCode::UNAUTHORIZED);
    }
}

db_test! {
    async fn logging_out_one_device_does_not_cut_off_the_others_access_token(pool: PgPool) {
        let app = app(pool.clone());
        let laptop = signup_verified(&app, "a@x.in").await;
        let phone = Jar::of(&login(&app, "a@x.in", PASSWORD).await);
        app.post("/v1/auth/logout").jar(&laptop).send().await;
        assert_eq!(me_status(&app, &laptop).await, StatusCode::UNAUTHORIZED);
        assert_eq!(me_status(&app, &phone).await, StatusCode::OK);
    }
}

db_test! {
    async fn resetting_the_password_cuts_off_every_devices_access_token(pool: PgPool) {
        let app = app(pool.clone());
        let laptop = signup_verified(&app, "a@x.in").await;
        let phone = Jar::of(&login(&app, "a@x.in", PASSWORD).await);

        app.post("/v1/auth/forgot-password").json(json!({"email":"a@x.in"})).send().await;
        let mails = app.mailer.wait_for(2).await;
        let token = token_from(&mails[1]);
        let res = app
            .post("/v1/auth/reset-password")
            .json(json!({"token": token, "new_password": "a-completely-new-passphrase"}))
            .send()
            .await;
        assert_eq!(res.status, StatusCode::OK);

        assert_eq!(me_status(&app, &laptop).await, StatusCode::UNAUTHORIZED);
        assert_eq!(me_status(&app, &phone).await, StatusCode::UNAUTHORIZED);
        let fresh = Jar::of(&login(&app, "a@x.in", "a-completely-new-passphrase").await);
        assert_eq!(me_status(&app, &fresh).await, StatusCode::OK);
    }
}

db_test! {
    async fn token_theft_detection_also_cuts_off_the_thiefs_access_token(pool: PgPool) {
        let app = app(pool.clone());
        let victim = signup_verified(&app, "a@x.in").await;
        let stolen_refresh = victim.clone();

        let rotated = app.post("/v1/auth/refresh").jar(&victim).send().await;
        let mut current = victim.clone();
        current.apply(&rotated);
        assert_eq!(me_status(&app, &current).await, StatusCode::OK);

        sqlx::query("update sessions set rotated_at = now() - interval '1 minute' where rotated_at is not null")
            .execute(&pool)
            .await
            .unwrap();
        let replay = app.post("/v1/auth/refresh").jar(&stolen_refresh).send().await;
        assert_eq!(replay.status, StatusCode::UNAUTHORIZED);

        assert_eq!(me_status(&app, &current).await, StatusCode::UNAUTHORIZED);
    }
}

/// Holds every email until the test releases it.
struct BlockedMailer {
    gate: Arc<Semaphore>,
    inner: MemoryMailer,
}

#[async_trait]
impl Mailer for BlockedMailer {
    async fn send(&self, email: OutgoingEmail) -> anyhow::Result<()> {
        let _permit = self.gate.acquire().await?;
        self.inner.send(email).await
    }
}

db_test! {
    async fn responses_never_wait_for_email_so_timing_cannot_reveal_accounts(pool: PgPool) {
        let gate = Arc::new(Semaphore::new(0));
        let inner = MemoryMailer::default();
        let state = make_state(
            pool.clone(),
            false,
            0,
            Arc::new(BlockedMailer { gate: gate.clone(), inner: inner.clone() }),
        );
        let app = App {
            router: zeno_api::build_app(state.clone()),
            mailer: inner.clone(),
            pool: pool.clone(),
            state,
        };

        // Existing account, mail transport completely stuck: every endpoint still answers.
        let first = tokio::time::timeout(
            Duration::from_secs(5),
            app.post("/v1/auth/register").json(register_body("a@x.in")).send(),
        )
        .await
        .expect("register must not wait for the mailer");
        assert_eq!(first.status, StatusCode::ACCEPTED);
        let again = tokio::time::timeout(
            Duration::from_secs(5),
            app.post("/v1/auth/register").json(register_body("a@x.in")).send(),
        )
        .await
        .expect("registering a taken email must not wait for the mailer either");
        assert_eq!(again.body, first.body);
        for path in ["/v1/auth/forgot-password", "/v1/auth/resend-verification"] {
            let res = tokio::time::timeout(
                Duration::from_secs(5),
                app.post(path).json(json!({"email":"a@x.in"})).send(),
            )
            .await
            .expect("must not wait for the mailer");
            assert_eq!(res.status, StatusCode::ACCEPTED);
        }
        assert!(inner.sent().is_empty(), "nothing can have been delivered yet");

        gate.add_permits(100);
        app.settle().await;
        assert!(!inner.sent().is_empty());
    }
}

db_test! {
    async fn unknown_addresses_get_the_same_answer_and_no_mail(pool: PgPool) {
        let app = app(pool.clone());
        let res = app
            .post("/v1/auth/forgot-password")
            .json(json!({"email":"nobody@x.in"}))
            .send()
            .await;
        assert_eq!(res.status, StatusCode::ACCEPTED);
        app.settle().await;
        assert!(app.mailer.sent().is_empty());
    }
}

db_test! {
    async fn cleanup_removes_dead_rows_and_keeps_everything_still_needed(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_verified(&app, "a@x.in").await;
        // Four more sessions for the same user, in different states.
        for _ in 0..4 {
            login(&app, "a@x.in", PASSWORD).await;
        }
        let ids: Vec<uuid::Uuid> = sqlx::query_scalar("select id from sessions order by created_at")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(ids.len(), 5);

        // 0: live, expiring soon. 1: expired over a week ago. 2: revoked over a day ago.
        // 3: rotated but unexpired. 4: expired two days ago (inside the retention window).
        sqlx::query("update sessions set expires_at = now() + interval '3 days' where id = $1")
            .bind(ids[0]).execute(&pool).await.unwrap();
        sqlx::query("update sessions set expires_at = now() - interval '8 days' where id = $1")
            .bind(ids[1]).execute(&pool).await.unwrap();
        sqlx::query("update sessions set expires_at = now() - interval '2 days' where id = $1")
            .bind(ids[4]).execute(&pool).await.unwrap();
        sqlx::query("update sessions set revoked_at = now() - interval '2 days' where id = $1")
            .bind(ids[2]).execute(&pool).await.unwrap();
        sqlx::query("update sessions set rotated_at = now() - interval '1 hour' where id = $1")
            .bind(ids[3]).execute(&pool).await.unwrap();

        // Tokens: spent long ago (dead), expired long ago (dead), fresh unused (needed).
        app.post("/v1/auth/forgot-password").json(json!({"email":"a@x.in"})).send().await;
        app.settle().await;
        sqlx::query("insert into auth_tokens (user_id, purpose, token_hash, expires_at, used_at) \
                     select id, 'email_verification', 'a'::bytea, now() - interval '3 days', now() - interval '3 days' from users")
            .execute(&pool).await.unwrap();
        sqlx::query("insert into auth_tokens (user_id, purpose, token_hash, expires_at) \
                     select id, 'email_verification', 'b'::bytea, now() - interval '3 days' from users")
            .execute(&pool).await.unwrap();

        let purged = repo::purge_expired(&pool).await.unwrap();
        let (sessions, tokens) = (purged.sessions, purged.tokens);
        assert_eq!((sessions, tokens), (2, 2));

        let left: Vec<uuid::Uuid> = sqlx::query_scalar("select id from sessions order by created_at")
            .fetch_all(&pool).await.unwrap();
        assert_eq!(left, vec![ids[0], ids[3], ids[4]]);
        let unused: i64 = count(&pool, "select count(*) from auth_tokens where used_at is null and expires_at > now()").await;
        assert_eq!(unused, 1, "the live reset token must survive");
        assert_eq!(me_status(&app, &jar).await, StatusCode::OK);
    }
}

db_test! {
    async fn every_response_carries_a_request_id_and_a_clients_own_is_kept(pool: PgPool) {
        let app = app(pool.clone());
        let generated = app.get("/health").send().await;
        let id = generated.headers.get("x-request-id").expect("generated").to_str().unwrap();
        assert!(uuid::Uuid::parse_str(id).is_ok(), "{id}");

        let echoed = app.get("/health").header("x-request-id", "trace-from-the-edge-123").send().await;
        assert_eq!(echoed.headers.get("x-request-id").unwrap(), "trace-from-the-edge-123");

        let error = app.get("/v1/auth/me").send().await;
        assert!(error.headers.get("x-request-id").is_some());
    }
}
