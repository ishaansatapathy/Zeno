//! Course ownership, validation, optimistic concurrency, lifecycle and pagination.

mod common;

use axum::http::{Method, StatusCode};
use common::*;
use futures_util::future::join_all;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

macro_rules! db_test {
    ($(#[$meta:meta])* async fn $name:ident($pool:ident: PgPool) $body:block) => {
        #[sqlx::test(migrator = "zeno_core::db::MIGRATOR")]
        $(#[$meta])*
        async fn $name($pool: PgPool) $body
    };
}

async fn create(app: &App, jar: &Jar, title: &str) -> Res {
    app.post("/v1/courses")
        .jar(jar)
        .json(json!({
            "title": title,
            "description": "A complete description",
            "short_description": "Learn properly",
            "price_paise": 49900,
            "language": "hi-IN",
            "level": "beginner"
        }))
        .send()
        .await
}

fn id(res: &Res) -> Uuid {
    Uuid::parse_str(res.body["course"]["id"].as_str().unwrap()).unwrap()
}

fn version(res: &Res) -> i32 {
    res.body["course"]["version"].as_i64().unwrap() as i32
}

async fn patch(app: &App, jar: &Jar, id: Uuid, body: Value) -> Res {
    app.req(Method::PATCH, &format!("/v1/courses/{id}"))
        .jar(jar)
        .json(body)
        .send()
        .await
}

db_test! {
    async fn creators_create_read_update_and_soft_delete_courses(pool: PgPool) {
        let app = app(pool.clone());
        let creator = signup_verified_as(&app, "teacher@x.in", "creator").await;
        let made = create(&app, &creator, "Physics 101").await;
        assert_eq!(made.status, StatusCode::CREATED, "{:?}", made.body);
        let course = id(&made);
        assert_eq!(made.body["course"]["slug"], "physics-101");
        assert_eq!(made.body["course"]["currency"], "INR");
        assert_eq!(made.body["course"]["status"], "draft");
        assert_eq!(made.body["course"]["language"], "hi-in");

        let changed = patch(
            &app,
            &creator,
            course,
            json!({"version": version(&made), "title": "Advanced Physics", "price_paise": 59900}),
        )
        .await;
        assert_eq!(changed.status, StatusCode::OK, "{:?}", changed.body);
        assert_eq!(changed.body["course"]["title"], "Advanced Physics");
        assert_eq!(changed.body["course"]["slug"], "physics-101", "published links stay stable");
        assert_eq!(version(&changed), version(&made) + 1);

        let shown = app.get(&format!("/v1/courses/{course}")).jar(&creator).send().await;
        assert_eq!(shown.status, StatusCode::OK);
        let removed = app.req(Method::DELETE, &format!("/v1/courses/{course}")).jar(&creator).send().await;
        assert_eq!(removed.status, StatusCode::NO_CONTENT);
        assert_eq!(app.get(&format!("/v1/courses/{course}")).jar(&creator).send().await.status, StatusCode::NOT_FOUND);
        let deleted_at: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar("select deleted_at from courses where id=$1")
            .bind(course).fetch_one(&pool).await.unwrap();
        assert!(deleted_at.is_some(), "course remains for enrollments and audit history");
    }
}

db_test! {
    async fn roles_and_ownership_are_enforced_without_leaking_courses(pool: PgPool) {
        let app = app(pool.clone());
        let owner = signup_verified_as(&app, "owner@x.in", "creator").await;
        let rival = signup_verified_as(&app, "rival@x.in", "creator").await;
        let student = signup_verified(&app, "student@x.in").await;
        let made = create(&app, &owner, "Secret batch").await;
        let course = id(&made);

        assert_eq!(app.get(&format!("/v1/courses/{course}")).jar(&rival).send().await.status, StatusCode::NOT_FOUND);
        assert_eq!(patch(&app, &rival, course, json!({"version": 1, "title": "Stolen"})).await.status, StatusCode::NOT_FOUND);
        assert_eq!(app.get(&format!("/v1/courses/{course}")).jar(&student).send().await.status, StatusCode::FORBIDDEN);
        assert_eq!(create(&app, &student, "Nope").await.status, StatusCode::FORBIDDEN);
        assert_eq!(app.get(&format!("/v1/courses/{course}")).send().await.status, StatusCode::UNAUTHORIZED);

        signup_verified_as(&app, "admin@x.in", "creator").await;
        sqlx::query("update users set role='admin' where email='admin@x.in'").execute(&pool).await.unwrap();
        let admin = Jar::of(&login(&app, "admin@x.in", PASSWORD).await);
        assert_eq!(app.get(&format!("/v1/courses/{course}")).jar(&admin).send().await.status, StatusCode::OK);
        let moderated = patch(&app, &admin, course, json!({"version": 1, "title": "Moderated title"})).await;
        assert_eq!(moderated.status, StatusCode::OK, "{:?}", moderated.body);
    }
}

db_test! {
    async fn validation_is_strict_and_reports_all_bad_fields(pool: PgPool) {
        let app = app(pool);
        let creator = signup_verified_as(&app, "teacher@x.in", "creator").await;
        let bad = app.post("/v1/courses").jar(&creator).json(json!({
            "title": "x",
            "description": "ok\u{0}bad",
            "price_paise": 50,
            "language": "--",
            "level": "wizard"
        })).send().await;
        assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY, "{:?}", bad.body);
        let fields = bad.body["error"]["fields"].as_array().unwrap();
        for name in ["title", "description", "price_paise", "language", "level"] {
            assert!(fields.iter().any(|field| field["field"] == name), "missing {name}: {fields:?}");
        }
        let unknown = app.post("/v1/courses").jar(&creator).json(json!({"title":"Valid title", "creator_id": Uuid::new_v4()})).send().await;
        assert_eq!(unknown.status, StatusCode::UNPROCESSABLE_ENTITY);
        let empty_patch = patch(&app, &creator, Uuid::new_v4(), json!({"version": 1})).await;
        assert_eq!(empty_patch.status, StatusCode::UNPROCESSABLE_ENTITY);
    }
}

db_test! {
    async fn optimistic_concurrency_allows_exactly_one_writer(pool: PgPool) {
        let app = app(pool);
        let creator = signup_verified_as(&app, "teacher@x.in", "creator").await;
        let made = create(&app, &creator, "Concurrent course").await;
        let course = id(&made);
        let attempts = ["First writer", "Second writer"].map(|title| {
            patch(&app, &creator, course, json!({"version": 1, "title": title}))
        });
        let results = join_all(attempts).await;
        assert_eq!(results.iter().filter(|res| res.status == StatusCode::OK).count(), 1);
        assert_eq!(results.iter().filter(|res| res.code() == "version_conflict").count(), 1);
        let missing_version = patch(&app, &creator, course, json!({"title": "No version"})).await;
        assert_eq!((missing_version.status, missing_version.code()), (StatusCode::PRECONDITION_REQUIRED, "version_required"));
    }
}

db_test! {
    async fn slugs_are_unique_under_parallel_creation_and_lists_use_stable_cursors(pool: PgPool) {
        let app = app(pool);
        let creator = signup_verified_as(&app, "teacher@x.in", "creator").await;
        let attempts = (0..20).map(|_| create(&app, &creator, "Same title"));
        let created = join_all(attempts).await;
        assert!(
            created.iter().all(|res| res.status == StatusCode::CREATED),
            "not every parallel create succeeded"
        );
        let mut slugs: Vec<&str> = created.iter().map(|res| res.body["course"]["slug"].as_str().unwrap()).collect();
        slugs.sort_unstable();
        slugs.dedup();
        assert_eq!(slugs.len(), 20);

        let first = app.get("/v1/courses?limit=7").jar(&creator).send().await;
        assert_eq!(first.status, StatusCode::OK, "{:?}", first.body);
        assert_eq!(first.body["courses"].as_array().unwrap().len(), 7);
        let cursor = first.body["next_cursor"].as_str().unwrap();
        let second = app.get(&format!("/v1/courses?limit=7&cursor={cursor}")).jar(&creator).send().await;
        assert_eq!(second.body["courses"].as_array().unwrap().len(), 7);
        let first_ids: Vec<&str> = first.body["courses"].as_array().unwrap().iter().map(|c| c["id"].as_str().unwrap()).collect();
        assert!(second.body["courses"].as_array().unwrap().iter().all(|c| !first_ids.contains(&c["id"].as_str().unwrap())));
        assert_eq!(app.get("/v1/courses?cursor=garbage").jar(&creator).send().await.code(), "invalid_cursor");
    }
}

db_test! {
    async fn publishing_requires_ready_lessons_and_lifecycle_is_versioned(pool: PgPool) {
        let app = app(pool.clone());
        let creator = signup_verified_as(&app, "teacher@x.in", "creator").await;
        let made = create(&app, &creator, "Publishable course").await;
        let course = id(&made);
        let empty = app.post(&format!("/v1/courses/{course}/publish")).jar(&creator).json(json!({"version": 1})).send().await;
        assert_eq!((empty.status, empty.code()), (StatusCode::CONFLICT, "course_incomplete"));

        let owner: Uuid = sqlx::query_scalar("select creator_id from courses where id=$1").bind(course).fetch_one(&pool).await.unwrap();
        let video = Uuid::new_v4();
        sqlx::query("insert into videos (id, owner_id, title, original_filename, content_type, size_bytes, part_size, source_key, status) values ($1,$2,'Lesson','a.mp4','video/mp4',1,5242880,'uploads/x','processing')")
            .bind(video).bind(owner).execute(&pool).await.unwrap();
        sqlx::query("insert into lessons (course_id, position, video_id) values ($1,1,$2)").bind(course).bind(video).execute(&pool).await.unwrap();
        let processing = app.post(&format!("/v1/courses/{course}/publish")).jar(&creator).json(json!({"version": 1})).send().await;
        assert_eq!((processing.status, processing.code()), (StatusCode::CONFLICT, "course_incomplete"));

        sqlx::query("update videos set status='ready' where id=$1").bind(video).execute(&pool).await.unwrap();
        let published = app.post(&format!("/v1/courses/{course}/publish")).jar(&creator).json(json!({"version": 1})).send().await;
        assert_eq!(published.status, StatusCode::OK, "{:?}", published.body);
        assert_eq!(published.body["course"]["status"], "published");
        assert!(!published.body["course"]["published_at"].is_null());
        assert_eq!(version(&published), 2);

        let unpublished = app.post(&format!("/v1/courses/{course}/unpublish")).jar(&creator).json(json!({"version": 2})).send().await;
        assert_eq!(unpublished.status, StatusCode::OK, "{:?}", unpublished.body);
        assert_eq!(unpublished.body["course"]["status"], "draft");
        assert!(unpublished.body["course"]["published_at"].is_null());
    }
}
