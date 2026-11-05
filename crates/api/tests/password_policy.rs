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

async fn register_with(app: &App, email: &str, password: &str) -> Res {
    app.post("/v1/auth/register")
        .json(json!({"name": "Asha Rao", "email": email, "password": password}))
        .send()
        .await
}

fn message(res: &Res) -> String {
    res.body["error"]["fields"][0]["message"]
        .as_str()
        .unwrap_or("")
        .to_owned()
}

db_test! {
    async fn common_passwords_are_refused_at_sign_up_and_no_account_is_created(pool: PgPool) {
        let app = app(pool.clone());
        for pw in ["password123456", "PASSWORD123456", "iloveyou1234", "qwertyuiop"] {
            let res = register_with(&app, "a@x.in", pw).await;
            assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{pw}");
            assert_eq!(res.body["error"]["fields"][0]["field"], "password");
            assert!(message(&res).contains("too common"), "{pw}: {}", message(&res));
        }
        app.settle().await;
        assert_eq!(count(&pool, "select count(*) from users").await, 0);
        assert!(app.mailer.sent().is_empty());
    }
}

db_test! {
    async fn repetitive_and_email_derived_passwords_are_refused(pool: PgPool) {
        let app = app(pool.clone());
        let repetitive = register_with(&app, "a@x.in", "abababababab").await;
        assert!(message(&repetitive).contains("repetitive"));

        let derived = register_with(&app, "asha.rao@example.com", "ASHA.RAO@example.com").await;
        assert_eq!(derived.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(message(&derived).contains("email"));

        let embedded = register_with(&app, "asha.rao@example.com", "my-asha.rao-2024-x").await;
        assert!(message(&embedded).contains("email"));
    }
}

db_test! {
    async fn a_good_passphrase_is_accepted(pool: PgPool) {
        let app = app(pool.clone());
        let res = register_with(&app, "a@x.in", "correct horse battery staple").await;
        assert_eq!(res.status, StatusCode::ACCEPTED);
    }
}

db_test! {
    async fn a_common_password_cannot_be_set_through_reset_and_the_link_survives(pool: PgPool) {
        let app = app(pool.clone());
        signup_verified(&app, "a@x.in").await;
        app.post("/v1/auth/forgot-password").json(json!({"email":"a@x.in"})).send().await;
        let token = token_from(&app.mailer.wait_for(2).await[1]);

        let weak = app
            .post("/v1/auth/reset-password")
            .json(json!({"token": token, "new_password": "password123456"}))
            .send()
            .await;
        assert_eq!(weak.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(weak.body["error"]["fields"][0]["field"], "new_password");

        let ok = app
            .post("/v1/auth/reset-password")
            .json(json!({"token": token, "new_password": "a-completely-new-passphrase"}))
            .send()
            .await;
        assert_eq!(ok.status, StatusCode::OK);
    }
}
