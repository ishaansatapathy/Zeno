//! Lesson authoring, dense ordering, ownership and publish-readiness invariants.

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

async fn create_course(app: &App, jar: &Jar, title: &str) -> Uuid {
    let response = app
        .post("/v1/courses")
        .jar(jar)
        .json(json!({"title": title, "description": "A complete course"}))
        .send()
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);
    Uuid::parse_str(response.body["course"]["id"].as_str().unwrap()).unwrap()
}

async fn video(
    pool: &PgPool,
    course_id: Uuid,
    status: &str,
    duration_seconds: Option<i32>,
) -> Uuid {
    let owner: Uuid = sqlx::query_scalar("select creator_id from courses where id=$1")
        .bind(course_id)
        .fetch_one(pool)
        .await
        .unwrap();
    video_for_owner(pool, owner, status, duration_seconds).await
}

async fn video_for_owner(
    pool: &PgPool,
    owner: Uuid,
    status: &str,
    duration_seconds: Option<i32>,
) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "insert into videos \
            (id, owner_id, title, original_filename, content_type, size_bytes, part_size, \
             source_key, status, duration_seconds) \
         values ($1,$2,'Lesson','lesson.mp4','video/mp4',1,5242880,$3,$4::video_status,$5)",
    )
    .bind(id)
    .bind(owner)
    .bind(format!("uploads/{id}"))
    .bind(status)
    .bind(duration_seconds)
    .execute(pool)
    .await
    .unwrap();
    id
}

async fn create_lesson(
    app: &App,
    jar: &Jar,
    course: Uuid,
    video: Uuid,
    title: &str,
    position: Option<i32>,
) -> Res {
    let mut body = json!({
        "title": title,
        "description": format!("Description for {title}"),
        "video_id": video,
        "is_free_preview": title == "Preview"
    });
    if let Some(position) = position {
        body["position"] = json!(position);
    }
    app.post(&format!("/v1/courses/{course}/lessons"))
        .jar(jar)
        .json(body)
        .send()
        .await
}

fn lesson_id(response: &Res) -> Uuid {
    Uuid::parse_str(response.body["lesson"]["id"].as_str().unwrap()).unwrap()
}

async fn list(app: &App, jar: &Jar, course: Uuid) -> Res {
    app.get(&format!("/v1/courses/{course}/lessons"))
        .jar(jar)
        .send()
        .await
}

fn listed_ids(response: &Res) -> Vec<Uuid> {
    response.body["lessons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|lesson| Uuid::parse_str(lesson["id"].as_str().unwrap()).unwrap())
        .collect()
}

async fn reorder(app: &App, jar: &Jar, course: Uuid, ids: &[Uuid]) -> Res {
    app.req(Method::PUT, &format!("/v1/courses/{course}/lessons/order"))
        .jar(jar)
        .json(json!({"lesson_ids": ids}))
        .send()
        .await
}

fn permutations(values: &[Uuid]) -> Vec<Vec<Uuid>> {
    fn visit(start: usize, current: &mut Vec<Uuid>, output: &mut Vec<Vec<Uuid>>) {
        if start == current.len() {
            output.push(current.clone());
            return;
        }
        for index in start..current.len() {
            current.swap(start, index);
            visit(start + 1, current, output);
            current.swap(start, index);
        }
    }
    let mut current = values.to_vec();
    let mut output = Vec::new();
    visit(0, &mut current, &mut output);
    output
}

db_test! {
    async fn crud_preserves_dense_positions_history_duration_and_course_version(pool: PgPool) {
        let app = app(pool.clone());
        let creator = signup_verified_as(&app, "lessons@x.in", "creator").await;
        let course = create_course(&app, &creator, "Dense ordering").await;
        let videos = [
            video(&pool, course, "ready", Some(10)).await,
            video(&pool, course, "ready", Some(20)).await,
            video(&pool, course, "ready", None).await,
            video(&pool, course, "processing", Some(40)).await,
        ];

        let a = create_lesson(&app, &creator, course, videos[0], "A", None).await;
        let b = create_lesson(&app, &creator, course, videos[1], "B", None).await;
        let c = create_lesson(&app, &creator, course, videos[2], "C", None).await;
        let inserted = create_lesson(&app, &creator, course, videos[3], "Preview", Some(2)).await;
        for response in [&a, &b, &c, &inserted] {
            assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);
        }
        assert_eq!(inserted.body["lesson"]["video_status"], "processing");
        assert_eq!(inserted.body["lesson"]["is_free_preview"], true);
        assert_eq!(inserted.body["course_version"], 5);

        let listed = list(&app, &creator, course).await;
        assert_eq!(
            listed_ids(&listed),
            vec![lesson_id(&a), lesson_id(&inserted), lesson_id(&b), lesson_id(&c)]
        );
        assert_eq!(listed.body["total_duration_seconds"], 70);
        let positions: Vec<i64> = listed.body["lessons"]
            .as_array().unwrap().iter().map(|row| row["position"].as_i64().unwrap()).collect();
        assert_eq!(positions, vec![1, 2, 3, 4]);

        let changed = app.req(Method::PATCH, &format!("/v1/lessons/{}", lesson_id(&inserted)))
            .jar(&creator)
            .json(json!({"title": "New preview", "is_free_preview": false}))
            .send().await;
        assert_eq!(changed.status, StatusCode::OK, "{:?}", changed.body);
        assert_eq!(changed.body["lesson"]["title"], "New preview");
        assert_eq!(changed.body["course_version"], 6);

        let removed = app.req(Method::DELETE, &format!("/v1/lessons/{}", lesson_id(&b)))
            .jar(&creator).send().await;
        assert_eq!(removed.status, StatusCode::NO_CONTENT);
        let after = list(&app, &creator, course).await;
        assert_eq!(listed_ids(&after), vec![lesson_id(&a), lesson_id(&inserted), lesson_id(&c)]);
        let positions: Vec<i64> = after.body["lessons"].as_array().unwrap()
            .iter().map(|row| row["position"].as_i64().unwrap()).collect();
        assert_eq!(positions, vec![1, 2, 3]);
        assert_eq!(after.body["course_version"], 7);

        let tombstone: (i32, i32, bool) = sqlx::query_as(
            "select position, deleted_position, deleted_at is not null from lessons where id=$1"
        ).bind(lesson_id(&b)).fetch_one(&pool).await.unwrap();
        assert!(tombstone.0 < 0);
        assert_eq!(tombstone.1, 3);
        assert!(tombstone.2);
    }
}

db_test! {
    async fn ownership_roles_and_attachable_video_states_do_not_leak_tenants(pool: PgPool) {
        let app = app(pool.clone());
        let owner = signup_verified_as(&app, "owner-lessons@x.in", "creator").await;
        let rival = signup_verified_as(&app, "rival-lessons@x.in", "creator").await;
        let student = signup_verified(&app, "student-lessons@x.in").await;
        let course = create_course(&app, &owner, "Private lessons").await;
        let rival_course = create_course(&app, &rival, "Rival course").await;
        let rival_owner: Uuid = sqlx::query_scalar("select creator_id from courses where id=$1")
            .bind(rival_course).fetch_one(&pool).await.unwrap();

        let processing = video(&pool, course, "processing", None).await;
        let made = create_lesson(&app, &owner, course, processing, "Pending", None).await;
        assert_eq!(made.status, StatusCode::CREATED, "{:?}", made.body);
        assert_eq!(list(&app, &rival, course).await.status, StatusCode::NOT_FOUND);
        assert_eq!(list(&app, &student, course).await.status, StatusCode::FORBIDDEN);
        assert_eq!(app.req(Method::PATCH, &format!("/v1/lessons/{}", lesson_id(&made)))
            .jar(&rival).json(json!({"title": "Stolen"})).send().await.status, StatusCode::NOT_FOUND);

        let foreign = video_for_owner(&pool, rival_owner, "ready", Some(10)).await;
        assert_eq!(create_lesson(&app, &owner, course, foreign, "Foreign", None).await.status, StatusCode::NOT_FOUND);
        for status in ["uploading", "failed"] {
            let unusable = video(&pool, course, status, None).await;
            let response = create_lesson(&app, &owner, course, unusable, status, None).await;
            assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY, "{status}: {:?}", response.body);
        }
        let deleted = video(&pool, course, "ready", None).await;
        sqlx::query("update videos set deleted_at=now() where id=$1")
            .bind(deleted).execute(&pool).await.unwrap();
        assert_eq!(create_lesson(&app, &owner, course, deleted, "Deleted", None).await.status, StatusCode::NOT_FOUND);

        let reusable = video(&pool, course, "ready", Some(15)).await;
        assert_eq!(create_lesson(&app, &owner, course, reusable, "Reuse one", None).await.status, StatusCode::CREATED);
        assert_eq!(create_lesson(&app, &owner, course, reusable, "Reuse two", None).await.status, StatusCode::CREATED);
    }
}

db_test! {
    async fn exhaustive_reorders_are_atomic_dense_and_idempotent(pool: PgPool) {
        let app = app(pool.clone());
        let creator = signup_verified_as(&app, "reorder@x.in", "creator").await;
        let course = create_course(&app, &creator, "Permutation course").await;
        let shared = video(&pool, course, "ready", Some(5)).await;
        let mut ids = Vec::new();
        for title in ["A", "B", "C", "D"] {
            let response = create_lesson(&app, &creator, course, shared, title, None).await;
            ids.push(lesson_id(&response));
        }

        for invalid in [
            vec![ids[0], ids[1], ids[2]],
            vec![ids[0], ids[1], ids[2], ids[2]],
            vec![ids[0], ids[1], ids[2], Uuid::new_v4()],
        ] {
            let before = listed_ids(&list(&app, &creator, course).await);
            let response = reorder(&app, &creator, course, &invalid).await;
            assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY, "{:?}", response.body);
            assert_eq!(listed_ids(&list(&app, &creator, course).await), before);
        }

        for permutation in permutations(&ids) {
            let response = reorder(&app, &creator, course, &permutation).await;
            assert_eq!(response.status, StatusCode::OK, "{:?}", response.body);
            assert_eq!(listed_ids(&response), permutation);
        }
        let before = list(&app, &creator, course).await;
        let order = listed_ids(&before);
        let version = before.body["course_version"].as_i64().unwrap();
        let same = reorder(&app, &creator, course, &order).await;
        assert_eq!(same.status, StatusCode::OK);
        assert_eq!(same.body["course_version"], version, "no-op reorder must not bump version");

        let stored: Vec<(i32, Uuid)> = sqlx::query_as(
            "select position, id from lessons where course_id=$1 and deleted_at is null order by position"
        ).bind(course).fetch_all(&pool).await.unwrap();
        assert_eq!(stored.iter().map(|row| row.0).collect::<Vec<_>>(), vec![1, 2, 3, 4]);
    }
}

db_test! {
    async fn concurrent_reorders_serialize_to_a_complete_permutation(pool: PgPool) {
        let app = app(pool.clone());
        let creator = signup_verified_as(&app, "concurrent-order@x.in", "creator").await;
        let course = create_course(&app, &creator, "Concurrent ordering").await;
        let shared = video(&pool, course, "ready", Some(5)).await;
        let mut ids = Vec::new();
        for title in ["A", "B", "C", "D", "E"] {
            ids.push(lesson_id(&create_lesson(&app, &creator, course, shared, title, None).await));
        }
        let reverse: Vec<_> = ids.iter().copied().rev().collect();
        let rotate = vec![ids[2], ids[3], ids[4], ids[0], ids[1]];
        let (first, second) = join!(
            reorder(&app, &creator, course, &reverse),
            reorder(&app, &creator, course, &rotate)
        );
        assert_eq!(first.status, StatusCode::OK, "{:?}", first.body);
        assert_eq!(second.status, StatusCode::OK, "{:?}", second.body);
        let final_order = listed_ids(&list(&app, &creator, course).await);
        assert!(final_order == reverse || final_order == rotate, "{final_order:?}");
        let positions: Vec<i32> = sqlx::query_scalar(
            "select position from lessons where course_id=$1 and deleted_at is null order by position"
        ).bind(course).fetch_all(&pool).await.unwrap();
        assert_eq!(positions, vec![1, 2, 3, 4, 5]);

        let victim = final_order[0];
        let requested: Vec<_> = final_order.iter().copied().rev().collect();
        let (deleted, reordered) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            join!(
                app.req(Method::DELETE, &format!("/v1/lessons/{victim}"))
                    .jar(&creator).send(),
                reorder(&app, &creator, course, &requested)
            )
        }).await.expect("course-first lock ordering must not deadlock");
        assert_eq!(deleted.status, StatusCode::NO_CONTENT);
        assert!(matches!(reordered.status, StatusCode::OK | StatusCode::UNPROCESSABLE_ENTITY));
        let positions: Vec<i32> = sqlx::query_scalar(
            "select position from lessons where course_id=$1 and deleted_at is null order by position"
        ).bind(course).fetch_all(&pool).await.unwrap();
        assert_eq!(positions, vec![1, 2, 3, 4]);
    }
}

db_test! {
    async fn publish_matrix_tracks_processing_ready_and_soft_deleted_lessons(pool: PgPool) {
        let app = app(pool.clone());
        let creator = signup_verified_as(&app, "publish-lessons@x.in", "creator").await;
        let course = create_course(&app, &creator, "Publish matrix").await;
        let pending_video = video(&pool, course, "processing", Some(30)).await;
        let ready_video = video(&pool, course, "ready", Some(40)).await;
        let pending = create_lesson(&app, &creator, course, pending_video, "Pending", None).await;
        let pending_id = lesson_id(&pending);

        let blocked = app.post(&format!("/v1/courses/{course}/publish"))
            .jar(&creator).json(json!({"version": 2})).send().await;
        assert_eq!((blocked.status, blocked.code()), (StatusCode::CONFLICT, "course_incomplete"));

        let changed = app.req(Method::PATCH, &format!("/v1/lessons/{pending_id}"))
            .jar(&creator).json(json!({"video_id": ready_video})).send().await;
        assert_eq!(changed.body["course_version"], 3);
        let published = app.post(&format!("/v1/courses/{course}/publish"))
            .jar(&creator).json(json!({"version": 3})).send().await;
        assert_eq!(published.status, StatusCode::OK, "{:?}", published.body);
        assert_eq!(published.body["course"]["status"], "published");

        let reused_course = create_course(&app, &creator, "Reused video course").await;
        let reused = create_lesson(
            &app,
            &creator,
            reused_course,
            ready_video,
            "Shared video",
            None,
        )
        .await;
        assert_eq!(reused.status, StatusCode::CREATED);
        let reused_published = app
            .post(&format!("/v1/courses/{reused_course}/publish"))
            .jar(&creator)
            .json(json!({"version": 2}))
            .send()
            .await;
        assert_eq!(reused_published.status, StatusCode::OK);

        let deleted_video = app.req(Method::DELETE, &format!("/v1/videos/{ready_video}"))
            .jar(&creator).send().await;
        assert_eq!(deleted_video.status, StatusCode::NO_CONTENT);
        let unavailable = list(&app, &creator, course).await;
        assert_eq!(unavailable.body["lessons"][0]["video_status"], "unavailable");
        let drafted = app.get(&format!("/v1/courses/{course}"))
            .jar(&creator).send().await;
        assert_eq!(drafted.body["course"]["status"], "draft");
        assert!(drafted.body["course"]["published_at"].is_null());
        assert_eq!(drafted.body["course"]["version"], 5);
        let reused_drafted = app
            .get(&format!("/v1/courses/{reused_course}"))
            .jar(&creator)
            .send()
            .await;
        assert_eq!(reused_drafted.body["course"]["status"], "draft");
        assert_eq!(reused_drafted.body["course"]["version"], 4);
        let invalid_published = app.post(&format!("/v1/courses/{course}/publish"))
            .jar(&creator).json(json!({"version": 5})).send().await;
        assert_eq!((invalid_published.status, invalid_published.code()), (StatusCode::CONFLICT, "course_incomplete"));

        let replacement = video(&pool, course, "ready", Some(40)).await;
        let repaired = app.req(Method::PATCH, &format!("/v1/lessons/{pending_id}"))
            .jar(&creator).json(json!({"video_id": replacement})).send().await;
        assert_eq!(repaired.body["course_version"], 6);
        let republished = app.post(&format!("/v1/courses/{course}/publish"))
            .jar(&creator).json(json!({"version": 6})).send().await;
        assert_eq!(republished.status, StatusCode::OK, "{:?}", republished.body);

        let second = create_lesson(&app, &creator, course, pending_video, "New pending", None).await;
        assert_eq!(second.status, StatusCode::CREATED);
        let course_state = app.get(&format!("/v1/courses/{course}")).jar(&creator).send().await;
        assert_eq!(course_state.body["course"]["status"], "draft", "invalidating mutation auto-unpublishes");
        assert!(course_state.body["course"]["published_at"].is_null());
        assert_eq!(course_state.body["course"]["version"], 8);

        let stale_publish = app.post(&format!("/v1/courses/{course}/publish"))
            .jar(&creator).json(json!({"version": 7})).send().await;
        assert_eq!((stale_publish.status, stale_publish.code()), (StatusCode::CONFLICT, "course_incomplete"));

        app.req(Method::DELETE, &format!("/v1/lessons/{}", lesson_id(&second)))
            .jar(&creator).send().await;
        let republished = app.post(&format!("/v1/courses/{course}/publish"))
            .jar(&creator).json(json!({"version": 9})).send().await;
        assert_eq!(republished.status, StatusCode::OK, "{:?}", republished.body);
        app.req(Method::DELETE, &format!("/v1/lessons/{pending_id}"))
            .jar(&creator).send().await;
        let empty = app.get(&format!("/v1/courses/{course}")).jar(&creator).send().await;
        assert_eq!(empty.body["course"]["status"], "draft");
        assert!(list(&app, &creator, course).await.body["lessons"].as_array().unwrap().is_empty());
        let blocked = app.post(&format!("/v1/courses/{course}/publish"))
            .jar(&creator).json(json!({"version": 11})).send().await;
        assert_eq!((blocked.status, blocked.code()), (StatusCode::CONFLICT, "course_incomplete"));

        let deleted_replacement = app.req(Method::DELETE, &format!("/v1/videos/{replacement}"))
            .jar(&creator).send().await;
        assert_eq!(deleted_replacement.status, StatusCode::NO_CONTENT);
        assert!(zeno_api::videos::repo::delete_purged(&pool, replacement).await.unwrap());
        let retained: (i64, bool) = sqlx::query_as(
            "select count(*), max(storage_purged_at) is not null from videos where id=$1"
        ).bind(replacement).fetch_one(&pool).await.unwrap();
        assert_eq!(retained, (1, true), "lesson history retains a completed video tombstone");
        assert!(!zeno_api::videos::repo::purgeable(&pool, 50).await.unwrap()
            .iter().any(|(id, _)| *id == replacement));
    }
}

db_test! {
    async fn publish_racing_an_incomplete_lesson_never_misreports_version_conflict(pool: PgPool) {
        let app = app(pool.clone());
        let creator = signup_verified_as(&app, "publish-race@x.in", "creator").await;
        let course = create_course(&app, &creator, "Publish race").await;
        let ready = video(&pool, course, "ready", Some(20)).await;
        let processing = video(&pool, course, "processing", Some(30)).await;
        assert_eq!(
            create_lesson(&app, &creator, course, ready, "Ready", None).await.status,
            StatusCode::CREATED
        );

        let (published, inserted) = join!(
            app.post(&format!("/v1/courses/{course}/publish"))
                .jar(&creator)
                .json(json!({"version": 2}))
                .send(),
            create_lesson(&app, &creator, course, processing, "Processing", None)
        );
        assert_eq!(inserted.status, StatusCode::CREATED, "{:?}", inserted.body);
        assert!(
            published.status == StatusCode::OK
                || (published.status == StatusCode::CONFLICT
                    && published.code() == "course_incomplete"),
            "publish race returned the wrong classification: {:?}",
            published.body
        );
        assert_ne!(published.code(), "version_conflict");

        let final_course = app
            .get(&format!("/v1/courses/{course}"))
            .jar(&creator)
            .send()
            .await;
        assert_eq!(final_course.body["course"]["status"], "draft");
        let positions: Vec<i32> = sqlx::query_scalar(
            "select position from lessons where course_id=$1 and deleted_at is null order by position"
        ).bind(course).fetch_all(&pool).await.unwrap();
        assert_eq!(positions, vec![1, 2]);
    }
}
