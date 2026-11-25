//! Changing one's own password and name, while signed in.

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

async fn change(app: &App, jar: &Jar, current: &str, new: &str) -> Res {
    app.post("/v1/auth/change-password")
        .jar(jar)
        .json(json!({"current_password": current, "new_password": new}))
        .send()
        .await
}

db_test! {
    async fn changing_the_password_needs_the_old_one_and_signs_everyone_out(pool: PgPool) {
        let app = app(pool.clone());
        let here = signup_verified(&app, "a@x.in").await;
        // Another place the same account is signed in.
        let elsewhere = Jar::of(&login(&app, "a@x.in", PASSWORD).await);
        assert_eq!(app.get("/v1/auth/me").jar(&elsewhere).send().await.status, StatusCode::OK);
        let before = app.mailer.sent().len();

        let res = change(&app, &here, PASSWORD, NEW_PASSWORD).await;
        assert_eq!(res.status, StatusCode::OK, "{:?}", res.body);
        assert!(
            res.set_cookies().iter().all(|cookie| cookie.contains("Max-Age=0")),
            "both cookies are cleared: {:?}",
            res.set_cookies()
        );

        // The old password is dead, the new one works, and nothing signed in before is still signed in.
        assert_eq!(login(&app, "a@x.in", PASSWORD).await.status, StatusCode::UNAUTHORIZED);
        assert_eq!(login(&app, "a@x.in", NEW_PASSWORD).await.status, StatusCode::OK);
        assert_eq!(app.get("/v1/auth/me").jar(&here).send().await.status, StatusCode::UNAUTHORIZED);
        assert_eq!(app.get("/v1/auth/me").jar(&elsewhere).send().await.status, StatusCode::UNAUTHORIZED);
        assert_eq!(app.post("/v1/auth/refresh").jar(&elsewhere).send().await.status, StatusCode::UNAUTHORIZED);

        // The person is told, at the address on the account.
        let mails = app.mailer.wait_for(before + 1).await;
        assert!(mails[before..].iter().any(|m| m.to == "a@x.in" && m.subject.contains("password was changed")));
        let kinds: Vec<String> = sqlx::query_scalar("select kind from auth_events where kind like 'password_chang%' order by id")
            .fetch_all(&pool).await.unwrap();
        assert_eq!(kinds, ["password_changed"]);
    }
}

db_test! {
    async fn a_session_alone_is_not_enough_to_take_the_account(pool: PgPool) {
        let app = app(pool.clone());
        let session = signup_verified(&app, "a@x.in").await;
        let wrong = change(&app, &session, "not the password at all", NEW_PASSWORD).await;
        assert_eq!(wrong.status, StatusCode::FORBIDDEN, "{:?}", wrong.body);
        assert_eq!(wrong.code(), "wrong_password");
        // Nothing changed: the same session works, and so does the same password.
        assert_eq!(app.get("/v1/auth/me").jar(&session).send().await.status, StatusCode::OK);
        assert_eq!(login(&app, "a@x.in", PASSWORD).await.status, StatusCode::OK);
        let kinds: Vec<String> = sqlx::query_scalar("select kind from auth_events where kind like 'password_chang%'")
            .fetch_all(&pool).await.unwrap();
        assert_eq!(kinds, ["password_change_failed"], "a refused try is on the record");

        // It is not a way to guess: five tries an hour, right or wrong.
        for _ in 0..4 {
            assert_eq!(change(&app, &session, "still not the password", NEW_PASSWORD).await.code(), "wrong_password");
        }
        let blocked = change(&app, &session, PASSWORD, NEW_PASSWORD).await;
        assert_eq!(blocked.status, StatusCode::TOO_MANY_REQUESTS, "{:?}", blocked.body);
        assert!(blocked.headers.get("retry-after").is_some());
        assert_eq!(login(&app, "a@x.in", PASSWORD).await.status, StatusCode::OK, "even the right one was not tried");
    }
}

db_test! {
    async fn the_new_password_has_to_be_a_good_one_and_a_different_one(pool: PgPool) {
        let app = app(pool.clone());
        let session = signup_verified(&app, "asha.rao@x.in").await;
        for (new, why) in [
            ("short", "too short"),
            ("password1234", "a common password"),
            ("asha.rao-and-more", "made from the email address"),
            (PASSWORD, "the one in use now"),
        ] {
            let res = change(&app, &session, PASSWORD, new).await;
            assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{why}: {:?}", res.body);
            assert_eq!(res.body["error"]["fields"][0]["field"], "new_password", "{why}");
        }
        // None of that signed anyone out.
        assert_eq!(app.get("/v1/auth/me").jar(&session).send().await.status, StatusCode::OK);
        assert_eq!(login(&app, "asha.rao@x.in", PASSWORD).await.status, StatusCode::OK);
    }
}

db_test! {
    async fn changing_the_password_ends_what_the_account_was_watching(pool: PgPool) {
        let app = app(pool.clone());
        let session = signup_verified(&app, "a@x.in").await;
        let owner: uuid::Uuid = sqlx::query_scalar("select id from users where email = 'a@x.in'").fetch_one(&pool).await.unwrap();
        let video = uuid::Uuid::new_v4();
        sqlx::query(
            "insert into videos (id, owner_id, original_filename, content_type, size_bytes, part_size, source_key, status) \
             values ($1, $2, 'a.mp4', 'video/mp4', 10, 5242880, 'uploads/x/source', 'ready')",
        )
        .bind(video).bind(owner).execute(&pool).await.unwrap();
        let viewing = uuid::Uuid::new_v4();
        sqlx::query("insert into playback_sessions (id, user_id, video_id, expires_at) values ($1, $2, $3, now() + interval '2 hours')")
            .bind(viewing).bind(owner).bind(video).execute(&pool).await.unwrap();

        assert_eq!(change(&app, &session, PASSWORD, NEW_PASSWORD).await.status, StatusCode::OK);
        let reason: Option<String> = sqlx::query_scalar("select end_reason from playback_sessions where id = $1")
            .bind(viewing).fetch_one(&pool).await.unwrap();
        assert_eq!(reason.as_deref(), Some("password_reset"), "whoever had the password is cut off the video too");
    }
}

db_test! {
    async fn only_a_signed_in_person_can_change_a_password_and_only_their_own(pool: PgPool) {
        let app = app(pool.clone());
        let a = signup_verified(&app, "a@x.in").await;
        let _b = signup_verified(&app, "b@x.in").await;
        let nobody = app
            .post("/v1/auth/change-password")
            .json(json!({"current_password": PASSWORD, "new_password": NEW_PASSWORD}))
            .send()
            .await;
        assert_eq!(nobody.status, StatusCode::UNAUTHORIZED);
        // There is no way to name another account: a's session changes a's password, and b's still works.
        assert_eq!(change(&app, &a, PASSWORD, NEW_PASSWORD).await.status, StatusCode::OK);
        assert_eq!(login(&app, "b@x.in", PASSWORD).await.status, StatusCode::OK);
        // And a request from another site is refused before anything is looked at.
        let b = Jar::of(&login(&app, "b@x.in", PASSWORD).await);
        let forged = app
            .post("/v1/auth/change-password")
            .jar(&b)
            .origin("https://evil.example")
            .json(json!({"current_password": PASSWORD, "new_password": NEW_PASSWORD}))
            .send()
            .await;
        assert_eq!(forged.status, StatusCode::FORBIDDEN);
        assert_eq!(forged.code(), "forbidden_origin");
        assert_eq!(login(&app, "b@x.in", PASSWORD).await.status, StatusCode::OK);
    }
}

db_test! {
    async fn a_person_can_rename_themselves_and_only_themselves(pool: PgPool) {
        let app = app(pool.clone());
        let a = signup_verified(&app, "a@x.in").await;
        let _b = signup_verified(&app, "b@x.in").await;
        let res = app.req(axum::http::Method::PATCH, "/v1/auth/me").jar(&a).json(json!({"name": "  Asha Rao-Iyer "})).send().await;
        assert_eq!(res.status, StatusCode::OK, "{:?}", res.body);
        assert_eq!(res.body["user"]["name"], "Asha Rao-Iyer", "trimmed the way a name at sign-up is");
        assert_eq!(app.get("/v1/auth/me").jar(&a).send().await.body["user"]["name"], "Asha Rao-Iyer");
        let other: String = sqlx::query_scalar("select name from users where email = 'b@x.in'").fetch_one(&pool).await.unwrap();
        assert_ne!(other, "Asha Rao-Iyer", "the other account is untouched");

        for (name, why) in [("", "empty"), ("   ", "blank"), ("x".repeat(200).as_str(), "far too long"), ("a\u{0000}b", "a control character")] {
            let res = app.req(axum::http::Method::PATCH, "/v1/auth/me").jar(&a).json(json!({"name": name})).send().await;
            assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{why}: {:?}", res.body);
        }
        let unknown = app.req(axum::http::Method::PATCH, "/v1/auth/me").jar(&a).json(json!({"name": "Fine", "role": "admin"})).send().await;
        assert_eq!(unknown.status, StatusCode::UNPROCESSABLE_ENTITY, "no sneaking another field in");
        let role: String = sqlx::query_scalar("select role::text from users where email = 'a@x.in'").fetch_one(&pool).await.unwrap();
        assert_ne!(role, "admin");
        let signed_out = app.req(axum::http::Method::PATCH, "/v1/auth/me").json(json!({"name": "Fine"})).send().await;
        assert_eq!(signed_out.status, StatusCode::UNAUTHORIZED);
    }
}
