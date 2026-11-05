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

db_test! {
    async fn register_creates_an_unverified_student_and_emails_a_link(pool: PgPool) {
        let app = app(pool.clone());
        let res = app.post("/v1/auth/register").json(register_body("Asha@Example.com")).send().await;

        assert_eq!(res.status, StatusCode::ACCEPTED);
        let mails = app.mailer.wait_for(1).await;
        assert_eq!(mails[0].to, "asha@example.com");
        assert!(mails[0].subject.contains("Verify"));
        assert!(mails[0].text.contains(&format!("{ORIGIN}/verify-email/")));

        let row: (String, bool, String) = sqlx::query_as(
            "select email, email_verified_at is not null, role::text from users",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row, ("asha@example.com".to_owned(), false, "student".to_owned()));
    }
}

db_test! {
    async fn password_is_stored_as_an_argon2id_hash_never_plaintext(pool: PgPool) {
        let app = app(pool.clone());
        signup(&app, "a@x.in").await;
        let hash: String = sqlx::query_scalar("select password_hash from users")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(hash.starts_with("$argon2id$"));
        assert!(!hash.contains(PASSWORD));
    }
}

db_test! {
    async fn creators_can_sign_up_but_admin_cannot_be_requested(pool: PgPool) {
        let app = app(pool.clone());
        let ok = app
            .post("/v1/auth/register")
            .json(json!({"name":"Teacher","email":"t@x.in","password":PASSWORD,"role":"creator"}))
            .send()
            .await;
        assert_eq!(ok.status, StatusCode::ACCEPTED);
        let role: String = sqlx::query_scalar("select role::text from users")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(role, "creator");

        let bad = app
            .post("/v1/auth/register")
            .json(json!({"name":"Eve","email":"eve@x.in","password":PASSWORD,"role":"admin"}))
            .send()
            .await;
        assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(count(&pool, "select count(*) from users").await, 1);
    }
}

db_test! {
    async fn validation_errors_name_each_bad_field(pool: PgPool) {
        let app = app(pool.clone());
        let res = app
            .post("/v1/auth/register")
            .json(json!({"name":"x","email":"nope","password":"short"}))
            .send()
            .await;
        assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(res.code(), "validation_failed");
        let fields: Vec<&str> = res.body["error"]["fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["field"].as_str().unwrap())
            .collect();
        assert_eq!(fields, ["name", "email", "password"]);
        assert_eq!(count(&pool, "select count(*) from users").await, 0);
    }
}

db_test! {
    async fn registering_a_taken_email_looks_identical_and_warns_the_owner(pool: PgPool) {
        let app = app(pool.clone());
        let first = app.post("/v1/auth/register").json(register_body("a@x.in")).send().await;
        let token = token_from(&app.mailer.wait_for(1).await[0]);
        assert_eq!(verify(&app, &token).await.status, StatusCode::OK);

        let second = app.post("/v1/auth/register").json(register_body("a@x.in")).send().await;

        assert_eq!(second.status, first.status);
        assert_eq!(second.body, first.body);
        assert_eq!(count(&pool, "select count(*) from users").await, 1);
        let mails = app.mailer.wait_for(2).await;
        assert!(mails[1].subject.contains("already have"));
        assert!(!mails[1].text.contains("verify-email"));
    }
}

db_test! {
    async fn registering_an_unverified_email_again_resends_and_voids_the_old_link(pool: PgPool) {
        let app = app(pool.clone());
        let old = signup(&app, "a@x.in").await;
        let new = signup(&app, "a@x.in").await;

        assert_ne!(old, new);
        assert_eq!(count(&pool, "select count(*) from users").await, 1);
        assert_eq!(verify(&app, &old).await.status, StatusCode::BAD_REQUEST);
        assert_eq!(verify(&app, &new).await.status, StatusCode::OK);
    }
}

db_test! {
    async fn concurrent_signups_for_one_email_create_exactly_one_user(pool: PgPool) {
        let app = app(pool.clone());
        let handles: Vec<_> = (0..6)
            .map(|_| {
                let req = app.post("/v1/auth/register").json(register_body("race@x.in"));
                tokio::spawn(req.send())
            })
            .collect();
        for handle in handles {
            assert_eq!(handle.await.unwrap().status, StatusCode::ACCEPTED);
        }
        assert_eq!(count(&pool, "select count(*) from users").await, 1);
    }
}

db_test! {
    async fn registration_is_rate_limited_per_address(pool: PgPool) {
        let app = app(pool.clone());
        for i in 0..10 {
            let res = app.post("/v1/auth/register").json(register_body(&format!("u{i}@x.in"))).send().await;
            assert_eq!(res.status, StatusCode::ACCEPTED);
        }
        let blocked = app.post("/v1/auth/register").json(register_body("u10@x.in")).send().await;
        assert_eq!(blocked.status, StatusCode::TOO_MANY_REQUESTS);
        assert!(blocked.headers.get("retry-after").is_some());
        assert_eq!(count(&pool, "select count(*) from users").await, 10);
    }
}

db_test! {
    async fn verifying_signs_the_user_in_and_marks_the_email_verified(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_verified(&app, "a@x.in").await;

        assert!(jar.get("zeno_at").is_some());
        assert!(jar.get("zeno_rt").is_some());
        let me = app.get("/v1/auth/me").jar(&jar).send().await;
        assert_eq!(me.status, StatusCode::OK);
        assert_eq!(me.body["user"]["email"], "a@x.in");
        assert_eq!(me.body["user"]["email_verified"], true);
        assert!(me.body["user"].get("password_hash").is_none());
    }
}

db_test! {
    async fn a_verification_link_works_only_once(pool: PgPool) {
        let app = app(pool.clone());
        let token = signup(&app, "a@x.in").await;
        assert_eq!(verify(&app, &token).await.status, StatusCode::OK);
        let again = verify(&app, &token).await;
        assert_eq!(again.status, StatusCode::BAD_REQUEST);
        assert_eq!(again.code(), "invalid_token");
    }
}

db_test! {
    async fn concurrent_use_of_one_verification_link_succeeds_exactly_once(pool: PgPool) {
        let app = app(pool.clone());
        let token = signup(&app, "a@x.in").await;
        let handles: Vec<_> = (0..6).map(|_| tokio::spawn(
            app.post("/v1/auth/verify-email").json(json!({ "token": token })).send()
        )).collect();
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
    async fn an_expired_verification_link_is_refused(pool: PgPool) {
        let app = app(pool.clone());
        let token = signup(&app, "a@x.in").await;
        sqlx::query("update auth_tokens set expires_at = now() - interval '1 second'")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(verify(&app, &token).await.status, StatusCode::BAD_REQUEST);
    }
}

db_test! {
    async fn garbage_and_empty_tokens_are_refused(pool: PgPool) {
        let app = app(pool.clone());
        for token in ["", "nope", &"a".repeat(5000), "../../etc/passwd"] {
            let res = verify(&app, token).await;
            assert_eq!(res.status, StatusCode::BAD_REQUEST, "{token:?}");
            assert_eq!(res.code(), "invalid_token");
        }
    }
}

db_test! {
    async fn a_verification_token_cannot_reset_a_password(pool: PgPool) {
        let app = app(pool.clone());
        let token = signup(&app, "a@x.in").await;
        let res = app
            .post("/v1/auth/reset-password")
            .json(json!({ "token": token, "new_password": "a-brand-new-password" }))
            .send()
            .await;
        assert_eq!(res.status, StatusCode::BAD_REQUEST);
        assert_eq!(verify(&app, &token).await.status, StatusCode::OK);
    }
}

db_test! {
    async fn resend_verification_is_uniform_and_capped_per_address(pool: PgPool) {
        let app = app(pool.clone());
        signup(&app, "a@x.in").await;
        let known = app.post("/v1/auth/resend-verification").json(json!({"email":"a@x.in"})).send().await;
        let unknown = app.post("/v1/auth/resend-verification").json(json!({"email":"ghost@x.in"})).send().await;
        assert_eq!(known.status, StatusCode::ACCEPTED);
        assert_eq!(known.body, unknown.body);

        for _ in 0..4 {
            let res = app.post("/v1/auth/resend-verification").json(json!({"email":"a@x.in"})).send().await;
            assert_eq!(res.status, StatusCode::ACCEPTED);
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        // 1 signup email + 3 resends allowed by the per-address cap; ghost@ never gets mail.
        let sent = app.mailer.sent();
        assert_eq!(sent.len(), 4, "{sent:#?}");
        assert!(sent.iter().all(|m| m.to == "a@x.in"));
    }
}
