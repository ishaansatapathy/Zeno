//! Enrollment lifecycle and the complete student playback authorization matrix.

mod common;

use axum::http::{Method, StatusCode};
use common::*;
use futures_util::join;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

macro_rules! db_test {
    ($(#[$meta:meta])* async fn $name:ident($pool:ident: PgPool) $body:block) => {
        #[sqlx::test(migrator = "zeno_core::db::MIGRATOR")]
        $(#[$meta])*
        async fn $name($pool: PgPool) $body
    };
}

struct BuiltCourse {
    course: Uuid,
    lesson: Uuid,
    video: Uuid,
    version: i32,
}

async fn create_course(app: &App, jar: &Jar, title: &str, price_paise: i64) -> Uuid {
    let response = app
        .post("/v1/courses")
        .jar(jar)
        .json(json!({
            "title": title,
            "description": "Complete course description",
            "price_paise": price_paise
        }))
        .send()
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);
    Uuid::parse_str(response.body["course"]["id"].as_str().unwrap()).unwrap()
}

async fn creator_id(pool: &PgPool, course: Uuid) -> Uuid {
    sqlx::query_scalar("select creator_id from courses where id=$1")
        .bind(course)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn video(pool: &PgPool, owner: Uuid, status: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "insert into videos \
            (id, owner_id, title, original_filename, content_type, size_bytes, part_size, \
             source_key, status, duration_seconds) \
         values ($1,$2,'Lesson','lesson.mp4','video/mp4',1,5242880,$3,$4::video_status,120)",
    )
    .bind(id)
    .bind(owner)
    .bind(format!("uploads/{id}"))
    .bind(status)
    .execute(pool)
    .await
    .unwrap();
    id
}

async fn attach(
    app: &App,
    creator: &Jar,
    course: Uuid,
    video: Uuid,
    title: &str,
    preview: bool,
) -> Uuid {
    let response = app
        .post(&format!("/v1/courses/{course}/lessons"))
        .jar(creator)
        .json(json!({
            "title": title,
            "video_id": video,
            "is_free_preview": preview
        }))
        .send()
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);
    Uuid::parse_str(response.body["lesson"]["id"].as_str().unwrap()).unwrap()
}

async fn build_course(
    app: &App,
    pool: &PgPool,
    creator: &Jar,
    title: &str,
    price_paise: i64,
    preview: bool,
    publish: bool,
) -> BuiltCourse {
    let course = create_course(app, creator, title, price_paise).await;
    let owner = creator_id(pool, course).await;
    let video = video(pool, owner, "ready").await;
    let lesson = attach(app, creator, course, video, "Lesson one", preview).await;
    let mut version = 2;
    if publish {
        let response = app
            .post(&format!("/v1/courses/{course}/publish"))
            .jar(creator)
            .json(json!({"version": version}))
            .send()
            .await;
        assert_eq!(response.status, StatusCode::OK, "{:?}", response.body);
        version = response.body["course"]["version"].as_i64().unwrap() as i32;
    }
    BuiltCourse {
        course,
        lesson,
        video,
        version,
    }
}

async fn grant(app: &App, creator: &Jar, course: Uuid, user: impl ToString) -> Res {
    app.post(&format!("/v1/courses/{course}/grants"))
        .jar(creator)
        .json(json!({"user": user.to_string()}))
        .send()
        .await
}

async fn start_lesson(app: &App, jar: &Jar, lesson: Uuid) -> Res {
    app.post(&format!("/v1/lessons/{lesson}/play"))
        .jar(jar)
        .send()
        .await
}

fn session_id(response: &Res) -> Uuid {
    Uuid::parse_str(response.body["playback"]["session_id"].as_str().unwrap()).unwrap()
}

async fn user_id(pool: &PgPool, email: &str) -> Uuid {
    sqlx::query_scalar("select id from users where email=$1")
        .bind(email)
        .fetch_one(pool)
        .await
        .unwrap()
}

db_test! {
    async fn free_claim_is_concurrent_idempotent_and_unpublished_blocks_only_new_claims(pool: PgPool) {
        let app = app(pool.clone());
        let creator = signup_verified_as(&app, "free-owner@x.in", "creator").await;
        let student = signup_verified(&app, "free-student@x.in").await;
        let newcomer = signup_verified(&app, "free-new@x.in").await;
        let free = build_course(&app, &pool, &creator, "Free course", 0, false, true).await;

        let (first, second) = join!(
            app.post(&format!("/v1/courses/{}/enroll", free.course)).jar(&student).send(),
            app.post(&format!("/v1/courses/{}/enroll", free.course)).jar(&student).send()
        );
        assert_eq!(first.status, StatusCode::CREATED, "{:?}", first.body);
        assert_eq!(second.status, StatusCode::CREATED, "{:?}", second.body);
        assert_eq!(first.body["enrollment"]["id"], second.body["enrollment"]["id"]);
        let rows: i64 = sqlx::query_scalar("select count(*) from enrollments where course_id=$1")
            .bind(free.course).fetch_one(&pool).await.unwrap();
        assert_eq!(rows, 1);
        let owner_claim = app.post(&format!("/v1/courses/{}/enroll", free.course))
            .jar(&creator).send().await;
        assert_eq!(owner_claim.status, StatusCode::CREATED, "creators may claim their own free course");

        let unpublish = app.post(&format!("/v1/courses/{}/unpublish", free.course))
            .jar(&creator).json(json!({"version": free.version})).send().await;
        assert_eq!(unpublish.status, StatusCode::OK);
        assert_eq!(start_lesson(&app, &student, free.lesson).await.status, StatusCode::CREATED,
            "existing students retain access while a creator edits");
        let blocked = app.post(&format!("/v1/courses/{}/enroll", free.course))
            .jar(&newcomer).send().await;
        assert_eq!((blocked.status, blocked.code()), (StatusCode::CONFLICT, "course_unavailable"));

        assert_eq!(app.req(Method::DELETE, &format!("/v1/courses/{}", free.course))
            .jar(&creator).send().await.status, StatusCode::NO_CONTENT);
        assert_eq!(start_lesson(&app, &student, free.lesson).await.status, StatusCode::CREATED,
            "existing enrollment gets the promised deleted-course grace period");
        sqlx::query("update courses set deleted_at=now()-interval '31 days' where id=$1")
            .bind(free.course).execute(&pool).await.unwrap();
        assert_eq!(start_lesson(&app, &student, free.lesson).await.status, StatusCode::FORBIDDEN);

        let paid = build_course(&app, &pool, &creator, "Paid course", 10_000, false, true).await;
        let payment = app.post(&format!("/v1/courses/{}/enroll", paid.course))
            .jar(&newcomer).send().await;
        assert_eq!((payment.status, payment.code()), (StatusCode::PAYMENT_REQUIRED, "payment_required"));
        assert!(payment.body["error"]["message"].as_str().unwrap().contains("/checkout/"));

        let mine = app.get("/v1/me/courses?limit=1").jar(&student).send().await;
        assert_eq!(mine.status, StatusCode::OK, "{:?}", mine.body);
        assert_eq!(mine.body["courses"].as_array().unwrap().len(), 1);
        assert_eq!(mine.body["courses"][0]["enrollment"]["source"], "free");
    }
}

db_test! {
    async fn creator_revocation_cannot_be_bypassed_by_claiming_a_free_course(pool: PgPool) {
        let app = app(pool.clone());
        let creator = signup_verified_as(&app, "blocked-owner@x.in", "creator").await;
        let student = signup_verified(&app, "blocked-student@x.in").await;
        let course = build_course(&app, &pool, &creator, "Moderated free course", 0, false, true).await;
        let student_id = user_id(&pool, "blocked-student@x.in").await;
        let endpoint = format!("/v1/courses/{}/enroll", course.course);
        let claimed = app.post(&endpoint).jar(&student).send().await;
        assert_eq!(claimed.status, StatusCode::CREATED);
        let enrollment_id = claimed.body["enrollment"]["id"].clone();
        let revoked = app.req(Method::DELETE, &format!(
            "/v1/courses/{}/grants/{student_id}?reason=Abuse", course.course
        )).jar(&creator).send().await;
        assert_eq!(revoked.status, StatusCode::NO_CONTENT);
        let attribution: (Uuid, String) = sqlx::query_as(
            "select revoked_by, revocation_kind from enrollments where user_id=$1 and course_id=$2"
        ).bind(student_id).bind(course.course).fetch_one(&pool).await.unwrap();
        assert_eq!(attribution, (creator_id(&pool, course.course).await, "creator".into()));
        for _ in 0..2 {
            let retried = app.post(&endpoint).jar(&student).send().await;
            assert_eq!((retried.status, retried.code()),
                (StatusCode::CONFLICT, "access_revoked_by_creator"), "{:?}", retried.body);
        }
        assert_eq!(start_lesson(&app, &student, course.lesson).await.status, StatusCode::FORBIDDEN);
        let restored = grant(&app, &creator, course.course, student_id).await;
        assert_eq!(restored.status, StatusCode::CREATED);
        assert_eq!(restored.body["enrollment"]["id"], enrollment_id);
        assert_eq!(restored.body["enrollment"]["active"], true);
        let actions: Vec<String> = sqlx::query_scalar(
            "select action from enrollment_events where enrollment_id=$1::uuid order by created_at"
        ).bind(enrollment_id.as_str().unwrap()).fetch_all(&pool).await.unwrap();
        assert_eq!(actions, vec!["created", "revoked", "reactivated"]);

        signup_verified_as(&app, "blocked-admin@x.in", "creator").await;
        sqlx::query("update users set role='admin' where email='blocked-admin@x.in'")
            .execute(&pool).await.unwrap();
        let admin = Jar::of(&login(&app, "blocked-admin@x.in", PASSWORD).await);
        assert_eq!(app.req(Method::DELETE, &format!("/v1/courses/{}/grants/{student_id}", course.course))
            .jar(&admin).send().await.status, StatusCode::NO_CONTENT);
        assert_eq!(app.post(&endpoint).jar(&student).send().await.code(), "access_revoked_by_creator");
        let attribution: String = sqlx::query_scalar(
            "select revocation_kind from enrollments where course_id=$1 and user_id=$2"
        ).bind(course.course).bind(student_id).fetch_one(&pool).await.unwrap();
        assert_eq!(attribution, "admin");
        assert_eq!(grant(&app, &admin, course.course, student_id).await.status, StatusCode::CREATED);
    }
}

db_test! {
    async fn free_claim_preserves_payment_revocation_and_serializes_with_creator_removal(pool: PgPool) {
        let app = app(pool.clone());
        let creator = signup_verified_as(&app, "race-block-owner@x.in", "creator").await;
        let student = signup_verified(&app, "race-block-student@x.in").await;
        let course = build_course(&app, &pool, &creator, "Revoke claim race", 0, false, true).await;
        let student_id = user_id(&pool, "race-block-student@x.in").await;
        let endpoint = format!("/v1/courses/{}/enroll", course.course);
        assert_eq!(app.post(&endpoint).jar(&student).send().await.status, StatusCode::CREATED);

        let (removed, claim) = join!(
            app.req(Method::DELETE, &format!("/v1/courses/{}/grants/{student_id}", course.course))
                .jar(&creator).send(),
            app.post(&endpoint).jar(&student).send()
        );
        assert_eq!(removed.status, StatusCode::NO_CONTENT);
        assert!(matches!(claim.status, StatusCode::CREATED | StatusCode::CONFLICT));
        let blocked = app.post(&endpoint).jar(&student).send().await;
        assert_eq!(blocked.code(), "access_revoked_by_creator");

        // Refund attribution survives a free claim, even when the course is now free.
        sqlx::query("update enrollments set revocation_kind='payment' where course_id=$1 and user_id=$2")
            .bind(course.course).bind(student_id).execute(&pool).await.unwrap();
        assert_eq!(app.post(&endpoint).jar(&student).send().await.code(), "access_revoked");

        let restored = grant(&app, &creator, course.course, student_id).await;
        assert_eq!(restored.status, StatusCode::CREATED);
        assert!(restored.body["enrollment"]["revoked_by"].is_null());
        assert!(restored.body["enrollment"]["revocation_kind"].is_null());
        sqlx::query("update enrollments set expires_at=now()-interval '1 second' where course_id=$1 and user_id=$2")
            .bind(course.course).bind(student_id).execute(&pool).await.unwrap();
        assert_eq!(app.post(&endpoint).jar(&student).send().await.code(), "access_expired",
            "expired manual grants cannot be silently converted into free access");
    }
}

db_test! {
    async fn authorization_matrix_uses_lesson_route_and_never_raw_video_ids(pool: PgPool) {
        let app = app(pool.clone());
        let creator = signup_verified_as(&app, "matrix-owner@x.in", "creator").await;
        let student = signup_verified(&app, "matrix-student@x.in").await;
        let outsider = signup_verified(&app, "matrix-outsider@x.in").await;
        let paid = build_course(&app, &pool, &creator, "Matrix paid", 500, false, true).await;

        assert_eq!(start_lesson(&app, &creator, paid.lesson).await.status, StatusCode::CREATED);
        let denied = start_lesson(&app, &student, paid.lesson).await;
        assert_eq!((denied.status, denied.code()), (StatusCode::FORBIDDEN, "playback_denied"));
        assert_eq!(app.post(&format!("/v1/lessons/{}/play", paid.lesson)).send().await.status, StatusCode::UNAUTHORIZED);

        let student_id = user_id(&pool, "matrix-student@x.in").await;
        assert_eq!(grant(&app, &creator, paid.course, student_id).await.status, StatusCode::CREATED);
        assert_eq!(start_lesson(&app, &student, paid.lesson).await.status, StatusCode::CREATED);
        let raw_video = app.post(&format!("/v1/videos/{}/play", paid.video)).jar(&student).send().await;
        assert_eq!((raw_video.status, raw_video.code()), (StatusCode::FORBIDDEN, "playback_denied"));

        let preview = build_course(&app, &pool, &creator, "Preview course", 500, true, true).await;
        assert_eq!(start_lesson(&app, &outsider, preview.lesson).await.status, StatusCode::CREATED);

        signup_verified_as(&app, "matrix-admin@x.in", "creator").await;
        sqlx::query("update users set role='admin' where email='matrix-admin@x.in'")
            .execute(&pool).await.unwrap();
        let admin = Jar::of(&login(&app, "matrix-admin@x.in", PASSWORD).await);
        assert_eq!(start_lesson(&app, &admin, paid.lesson).await.status, StatusCode::CREATED);

        let draft = build_course(&app, &pool, &creator, "Processing course", 500, false, false).await;
        let processing = video(&pool, creator_id(&pool, draft.course).await, "processing").await;
        let processing_lesson = attach(&app, &creator, draft.course, processing, "Pending", false).await;
        assert_eq!(grant(&app, &creator, draft.course, student_id).await.status, StatusCode::CREATED);
        let not_ready = start_lesson(&app, &student, processing_lesson).await;
        assert_eq!((not_ready.status, not_ready.code()), (StatusCode::CONFLICT, "not_ready"));
    }
}

db_test! {
    async fn revoke_and_expiry_end_the_next_request_with_the_right_reason(pool: PgPool) {
        let app = app(pool.clone());
        let creator = signup_verified_as(&app, "revoke-owner@x.in", "creator").await;
        let student = signup_verified(&app, "revoke-student@x.in").await;
        let course = build_course(&app, &pool, &creator, "Revocation course", 500, false, true).await;
        let student_id = user_id(&pool, "revoke-student@x.in").await;
        let granted = grant(&app, &creator, course.course, student_id).await;
        let enrollment_id = Uuid::parse_str(granted.body["enrollment"]["id"].as_str().unwrap()).unwrap();

        let started = start_lesson(&app, &student, course.lesson).await;
        let session = session_id(&started);
        let playlist_url = started.body["playback"]["playlist_url"]
            .as_str()
            .unwrap()
            .to_owned();
        let revoked = app.req(Method::DELETE, &format!("/v1/courses/{}/grants/{student_id}?reason=Refunded", course.course))
            .jar(&creator).send().await;
        assert_eq!(revoked.status, StatusCode::NO_CONTENT);
        let heartbeat = app.post(&format!("/v1/play/{session}/heartbeat"))
            .jar(&student).json(json!({"position_seconds": 12})).send().await;
        assert_eq!((heartbeat.status, heartbeat.code()), (StatusCode::CONFLICT, "session_ended"));
        let playlist = app.get(&playlist_url).send().await;
        assert_eq!((playlist.status, playlist.code()), (StatusCode::FORBIDDEN, "playback_denied"));
        let reason: String = sqlx::query_scalar("select end_reason from playback_sessions where id=$1")
            .bind(session).fetch_one(&pool).await.unwrap();
        assert_eq!(reason, "access_revoked");

        let regranted = grant(&app, &creator, course.course, student_id).await;
        assert_eq!(Uuid::parse_str(regranted.body["enrollment"]["id"].as_str().unwrap()).unwrap(), enrollment_id);
        let expiring = start_lesson(&app, &student, course.lesson).await;
        let expiring_session = session_id(&expiring);
        sqlx::query("update enrollments set expires_at=now()-interval '1 second' where id=$1")
            .bind(enrollment_id).execute(&pool).await.unwrap();
        let expired = app.post(&format!("/v1/play/{expiring_session}/heartbeat"))
            .jar(&student).json(json!({"position_seconds": 20})).send().await;
        assert_eq!((expired.status, expired.code()), (StatusCode::CONFLICT, "session_ended"));
        let reason: String = sqlx::query_scalar("select end_reason from playback_sessions where id=$1")
            .bind(expiring_session).fetch_one(&pool).await.unwrap();
        assert_eq!(reason, "expired");

        let actions: Vec<String> = sqlx::query_scalar(
            "select action from enrollment_events where enrollment_id=$1 order by created_at"
        ).bind(enrollment_id).fetch_all(&pool).await.unwrap();
        assert_eq!(actions, vec!["created", "revoked", "reactivated"]);
    }
}

db_test! {
    async fn pending_email_invites_convert_on_verification_and_revoked_invites_do_not(pool: PgPool) {
        let app = app(pool.clone());
        let creator = signup_verified_as(&app, "invite-owner@x.in", "creator").await;
        let course = build_course(&app, &pool, &creator, "Invite course", 500, false, false).await;

        let invited = grant(&app, &creator, course.course, "future@x.in").await;
        assert_eq!(invited.status, StatusCode::ACCEPTED, "{:?}", invited.body);
        app.settle().await;
        let hash: Option<Vec<u8>> = sqlx::query_scalar("select token_hash from course_invites where email='future@x.in'")
            .fetch_one(&pool).await.unwrap();
        assert!(hash.is_none(), "verified-email invites do not generate unused tokens");
        let invite_mail = app.mailer.sent().into_iter().find(|mail| {
            mail.to == "future@x.in" && mail.subject.contains("invited")
        }).expect("the pending invite email must be delivered");
        assert!(invite_mail.text.contains("/register"));
        assert!(!invite_mail.text.contains("?invite="));

        let future = signup_verified(&app, "future@x.in").await;
        let mine = app.get("/v1/me/courses").jar(&future).send().await;
        assert_eq!(mine.body["courses"].as_array().unwrap().len(), 1);
        assert_eq!(mine.body["courses"][0]["enrollment"]["source"], "grant");
        let accepted: bool = sqlx::query_scalar(
            "select accepted_at is not null from course_invites where email='future@x.in'"
        ).fetch_one(&pool).await.unwrap();
        assert!(accepted);

        assert_eq!(grant(&app, &creator, course.course, "revoked-invite@x.in").await.status, StatusCode::ACCEPTED);
        app.settle().await;
        let revoked = app.req(Method::DELETE, &format!(
            "/v1/courses/{}/grants/revoked-invite@x.in", course.course
        )).jar(&creator).send().await;
        assert_eq!(revoked.status, StatusCode::NO_CONTENT);
        let no_access = signup_verified(&app, "revoked-invite@x.in").await;
        assert!(app.get("/v1/me/courses").jar(&no_access).send().await.body["courses"]
            .as_array().unwrap().is_empty());

        let racing_email = "invite-race@x.in";
        let (racing_grant, registered) = join!(
            grant(&app, &creator, course.course, racing_email),
            app.post("/v1/auth/register")
                .json(register_body(racing_email))
                .send()
        );
        assert!(matches!(racing_grant.status, StatusCode::CREATED | StatusCode::ACCEPTED));
        assert_eq!(registered.status, StatusCode::ACCEPTED);
        app.settle().await;
        let verification = app.mailer.sent().into_iter().find(|mail| {
            mail.to == racing_email && mail.subject == "Verify your Zeno account"
        }).expect("the concurrent registration must still send verification");
        let verified = verify(&app, &token_from(&verification)).await;
        assert_eq!(verified.status, StatusCode::OK, "{:?}", verified.body);
        let racing_user = user_id(&pool, racing_email).await;
        let rows: i64 = sqlx::query_scalar(
            "select count(*) from enrollments where course_id=$1 and user_id=$2"
        ).bind(course.course).bind(racing_user).fetch_one(&pool).await.unwrap();
        assert_eq!(rows, 1, "grant/register races converge to one enrollment");
    }
}

db_test! {
    async fn shared_video_access_and_student_listing_are_tenant_safe_and_paginated(pool: PgPool) {
        let app = app(pool.clone());
        let creator = signup_verified_as(&app, "shared-owner@x.in", "creator").await;
        let rival = signup_verified_as(&app, "shared-rival@x.in", "creator").await;
        let student = signup_verified(&app, "shared-student@x.in").await;
        let first = create_course(&app, &creator, "Shared one", 500).await;
        let second = create_course(&app, &creator, "Shared two", 500).await;
        let shared = video(&pool, creator_id(&pool, first).await, "ready").await;
        let first_lesson = attach(&app, &creator, first, shared, "First", false).await;
        let second_lesson = attach(&app, &creator, second, shared, "Second", false).await;
        let student_id = user_id(&pool, "shared-student@x.in").await;
        assert_eq!(grant(&app, &rival, first, student_id).await.status, StatusCode::NOT_FOUND);
        assert_eq!(grant(&app, &student, first, student_id).await.status, StatusCode::FORBIDDEN);
        assert_eq!(grant(&app, &creator, first, student_id).await.status, StatusCode::CREATED);
        assert_eq!(start_lesson(&app, &student, second_lesson).await.status, StatusCode::CREATED,
            "access through any course containing the video is sufficient");

        for index in 0..3 {
            let email = format!("listed-{index}@x.in");
            signup_verified(&app, &email).await;
            assert_eq!(grant(&app, &creator, first, user_id(&pool, &email).await).await.status, StatusCode::CREATED);
        }
        let phone_email = "listed-phone@x.in";
        signup_verified(&app, phone_email).await;
        sqlx::query("update users set phone='+919876543210' where email=$1")
            .bind(phone_email).execute(&pool).await.unwrap();
        assert_eq!(grant(&app, &creator, first, "+919876543210").await.status, StatusCode::CREATED);
        let page = app.get(&format!("/v1/courses/{first}/students?limit=2"))
            .jar(&creator).send().await;
        assert_eq!(page.body["students"].as_array().unwrap().len(), 2);
        let cursor = page.body["next_cursor"].as_str().unwrap();
        let next = app.get(&format!("/v1/courses/{first}/students?limit=2&cursor={cursor}"))
            .jar(&creator).send().await;
        assert_eq!(next.body["students"].as_array().unwrap().len(), 2);
        let first_ids: Vec<&str> = page.body["students"].as_array().unwrap().iter()
            .map(|row| row["enrollment"]["id"].as_str().unwrap()).collect();
        assert!(next.body["students"].as_array().unwrap().iter()
            .all(|row| !first_ids.contains(&row["enrollment"]["id"].as_str().unwrap())));
        assert_eq!(app.get(&format!("/v1/courses/{first}/students")).jar(&rival).send().await.status,
            StatusCode::NOT_FOUND);

        app.req(Method::DELETE, &format!("/v1/lessons/{first_lesson}"))
            .jar(&creator).send().await;
        let still_shared = start_lesson(&app, &student, second_lesson).await;
        assert_eq!(still_shared.status, StatusCode::FORBIDDEN,
            "deleted lessons no longer confer video access");
    }
}
