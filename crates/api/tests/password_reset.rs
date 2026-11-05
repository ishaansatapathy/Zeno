mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::json;
use sqlx::PgPool;

macro_rules! db_test {
    ($(#[$meta:meta])* async fn $name:ident($pool:ident: PgPool) $body:block) => {
        $(#[$meta])*
        #[sqlx::test(migrator = "zeno_core::db::MIGRATOR")]
        async fn $name($pool: PgPool) $body
    };
}

const NEW_PASSWORD: &str = "a-completely-new-passphrase";

async fn forgot(app: &App, email: &str) -> Res {
    app.post("/v1/auth/forgot-password")
        .json(json!({ "email": email }))
        .send()
        .await
}

async fn reset(app: &App, token: &str, password: &str) -> Res {
    app.post("/v1/auth/reset-password")
        .json(json!({ "token": token, "new_password": password }))
        .send()
        .await
}

/// Requests a reset and returns the token from the email (the n-th message so far).
async fn request_reset(app: &App, email: &str) -> String {
    let before = app.mailer.sent().len();
    let res = forgot(app, email).await;
    assert_eq!(res.status, StatusCode::ACCEPTED);
    let mails = app.mailer.wait_for(before + 1).await;
    token_from(&mails[before])
}

db_test! {
    async fn forgot_password_answers_identically_for_known_and_unknown_emails(pool: PgPool) {
        let app = app(pool.clone());
        signup_verified(&app, "a@x.in").await;
        let known = forgot(&app, "a@x.in").await;
        let unknown = forgot(&app, "ghost@x.in").await;
        assert_eq!(known.status, StatusCode::ACCEPTED);
        assert_eq!(known.status, unknown.status);
        assert_eq!(known.body, unknown.body);

        let mails = app.mailer.wait_for(2).await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(app.mailer.sent().len(), mails.len());
        assert!(app.mailer.sent().iter().all(|m| m.to == "a@x.in"));
    }
}

db_test! {
    async fn forgot_password_rejects_malformed_emails(pool: PgPool) {
        let app = app(pool.clone());
        assert_eq!(forgot(&app, "not-an-email").await.status, StatusCode::UNPROCESSABLE_ENTITY);
    }
}

db_test! {
    async fn resetting_changes_the_password_signs_everyone_out_and_notifies(pool: PgPool) {
        let app = app(pool.clone());
        let session = signup_verified(&app, "a@x.in").await;
        let token = request_reset(&app, "a@x.in").await;

        let res = reset(&app, &token, NEW_PASSWORD).await;
        assert_eq!(res.status, StatusCode::OK);

        assert_eq!(login(&app, "a@x.in", PASSWORD).await.status, StatusCode::UNAUTHORIZED);
        assert_eq!(login(&app, "a@x.in", NEW_PASSWORD).await.status, StatusCode::OK);
        assert_eq!(
            app.post("/v1/auth/refresh").jar(&session).send().await.status,
            StatusCode::UNAUTHORIZED,
            "sessions from before the reset must be revoked"
        );
        let mails = app.mailer.wait_for(3).await;
        assert!(mails[2].subject.contains("changed"));
    }
}

db_test! {
    async fn a_reset_link_works_only_once(pool: PgPool) {
        let app = app(pool.clone());
        signup_verified(&app, "a@x.in").await;
        let token = request_reset(&app, "a@x.in").await;
        assert_eq!(reset(&app, &token, NEW_PASSWORD).await.status, StatusCode::OK);
        let again = reset(&app, &token, "yet-another-passphrase").await;
        assert_eq!(again.status, StatusCode::BAD_REQUEST);
        assert_eq!(login(&app, "a@x.in", NEW_PASSWORD).await.status, StatusCode::OK);
    }
}

db_test! {
    async fn concurrent_use_of_one_reset_link_succeeds_exactly_once(pool: PgPool) {
        let app = app(pool.clone());
        signup_verified(&app, "a@x.in").await;
        let token = request_reset(&app, "a@x.in").await;
        let handles: Vec<_> = (0..6)
            .map(|i| tokio::spawn(
                app.post("/v1/auth/reset-password")
                    .json(json!({ "token": token, "new_password": format!("passphrase-number-{i}") }))
                    .send()
            ))
            .collect();
        let mut ok = 0;
        for handle in handles {
            if handle.await.unwrap().status == StatusCode::OK {
                ok += 1;
            }
        }
        assert_eq!(ok, 1);
    }
}

db_test! {
    async fn an_expired_reset_link_is_refused(pool: PgPool) {
        let app = app(pool.clone());
        signup_verified(&app, "a@x.in").await;
        let token = request_reset(&app, "a@x.in").await;
        sqlx::query("update auth_tokens set expires_at = now() - interval '1 second' where purpose = 'password_reset'")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(reset(&app, &token, NEW_PASSWORD).await.status, StatusCode::BAD_REQUEST);
        assert_eq!(login(&app, "a@x.in", PASSWORD).await.status, StatusCode::OK);
    }
}

db_test! {
    async fn requesting_a_new_reset_link_voids_the_previous_one(pool: PgPool) {
        let app = app(pool.clone());
        signup_verified(&app, "a@x.in").await;
        let first = request_reset(&app, "a@x.in").await;
        let second = request_reset(&app, "a@x.in").await;
        assert_eq!(reset(&app, &first, NEW_PASSWORD).await.status, StatusCode::BAD_REQUEST);
        assert_eq!(reset(&app, &second, NEW_PASSWORD).await.status, StatusCode::OK);
    }
}

db_test! {
    async fn a_weak_new_password_is_rejected_without_burning_the_link(pool: PgPool) {
        let app = app(pool.clone());
        signup_verified(&app, "a@x.in").await;
        let token = request_reset(&app, "a@x.in").await;
        let weak = reset(&app, &token, "short").await;
        assert_eq!(weak.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(weak.body["error"]["fields"][0]["field"], "new_password");
        assert_eq!(reset(&app, &token, NEW_PASSWORD).await.status, StatusCode::OK);
    }
}

db_test! {
    async fn garbage_reset_tokens_are_refused(pool: PgPool) {
        let app = app(pool.clone());
        for token in ["", "nope", &"z".repeat(10_000)] {
            let res = reset(&app, token, NEW_PASSWORD).await;
            assert_eq!(res.status, StatusCode::BAD_REQUEST, "{token:?}");
            assert_eq!(res.code(), "invalid_token");
        }
    }
}

db_test! {
    async fn only_three_reset_emails_per_address_per_hour_and_the_cap_is_silent(pool: PgPool) {
        let app = app(pool.clone());
        signup_verified(&app, "a@x.in").await;
        for _ in 0..5 {
            assert_eq!(forgot(&app, "a@x.in").await.status, StatusCode::ACCEPTED);
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let resets = app
            .mailer
            .sent()
            .iter()
            .filter(|m| m.subject.contains("Reset"))
            .count();
        assert_eq!(resets, 3);
    }
}

db_test! {
    async fn resetting_also_verifies_an_address_that_was_never_confirmed(pool: PgPool) {
        let app = app(pool.clone());
        signup(&app, "a@x.in").await;
        let token = request_reset(&app, "a@x.in").await;
        assert_eq!(reset(&app, &token, NEW_PASSWORD).await.status, StatusCode::OK);
        assert_eq!(login(&app, "a@x.in", NEW_PASSWORD).await.status, StatusCode::OK);
    }
}
