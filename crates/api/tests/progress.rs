//! Lesson progress, resume and continue-watching.
//!
//! Heartbeats are 30 s apart in real life; tests cannot wait, so they move a progress row's
//! `last_watched_at` into the past to stand in for the clock.

mod common;

use axum::http::{Method, StatusCode};
use common::*;
use futures_util::join;
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

const LESSON_SECONDS: i32 = 600;

struct Course {
    id: Uuid,
    lesson: Uuid,
    video: Uuid,
}

async fn owner_of(pool: &PgPool, course: Uuid) -> Uuid {
    sqlx::query_scalar("select creator_id from courses where id=$1")
        .bind(course)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// A published free course with one ready 10-minute lesson.
async fn free_course(app: &App, pool: &PgPool, creator: &Jar, title: &str) -> Course {
    let response = app
        .post("/v1/courses")
        .jar(creator)
        .json(json!({"title": title, "description": "Course", "price_paise": 0}))
        .send()
        .await;
    let id = Uuid::parse_str(response.body["course"]["id"].as_str().unwrap()).unwrap();
    let owner = owner_of(pool, id).await;
    let video = Uuid::new_v4();
    sqlx::query(
        "insert into videos \
            (id, owner_id, title, original_filename, content_type, size_bytes, part_size, \
             source_key, status, duration_seconds) \
         values ($1,$2,'Lesson','lesson.mp4','video/mp4',1,5242880,$3,'ready',$4)",
    )
    .bind(video)
    .bind(owner)
    .bind(format!("uploads/{video}"))
    .bind(LESSON_SECONDS)
    .execute(pool)
    .await
    .unwrap();
    let lesson = app
        .post(&format!("/v1/courses/{id}/lessons"))
        .jar(creator)
        .json(json!({"title": "Lesson one", "video_id": video}))
        .send()
        .await;
    assert_eq!(lesson.status, StatusCode::CREATED, "{:?}", lesson.body);
    let version = lesson.body["course_version"].as_i64().unwrap();
    let published = app
        .post(&format!("/v1/courses/{id}/publish"))
        .jar(creator)
        .json(json!({"version": version}))
        .send()
        .await;
    assert_eq!(published.status, StatusCode::OK, "{:?}", published.body);
    Course {
        id,
        lesson: Uuid::parse_str(lesson.body["lesson"]["id"].as_str().unwrap()).unwrap(),
        video,
    }
}

async fn enroll(app: &App, student: &Jar, course: &Course) {
    let response = app
        .post(&format!("/v1/courses/{}/enroll", course.id))
        .jar(student)
        .send()
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);
}

async fn play(app: &App, who: &Jar, lesson: Uuid) -> Res {
    app.post(&format!("/v1/lessons/{lesson}/play"))
        .jar(who)
        .send()
        .await
}

fn session_of(response: &Res) -> Uuid {
    Uuid::parse_str(response.body["playback"]["session_id"].as_str().unwrap()).unwrap()
}

async fn beat(app: &App, who: &Jar, session: Uuid, position: u32) -> Res {
    let response = app
        .post(&format!("/v1/play/{session}/heartbeat"))
        .jar(who)
        .json(json!({"position_seconds": position}))
        .send()
        .await;
    assert!(
        response.status.as_u16() < 500,
        "heartbeat at {position}: {:?}",
        response.body
    );
    response
}

/// Pretends 30 seconds have passed since the last heartbeat for this lesson.
async fn thirty_seconds_later(pool: &PgPool, lesson: Uuid) {
    sqlx::query(
        "update lesson_progress set last_watched_at = last_watched_at - interval '30 seconds' \
         where lesson_id = $1",
    )
    .bind(lesson)
    .execute(pool)
    .await
    .unwrap();
}

async fn row(pool: &PgPool, lesson: Uuid) -> (i32, i32, i32, bool) {
    sqlx::query_as(
        "select last_position_seconds, furthest_seconds, watched_seconds, completed_at is not null \
         from lesson_progress where lesson_id=$1",
    )
    .bind(lesson)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn progress(app: &App, who: &Jar, course: Uuid) -> Res {
    app.get(&format!("/v1/courses/{course}/progress"))
        .jar(who)
        .send()
        .await
}

async fn continuing(app: &App, who: &Jar) -> Value {
    let response = app.get("/v1/me/continue-watching").jar(who).send().await;
    assert_eq!(response.status, StatusCode::OK, "{:?}", response.body);
    response.body["items"].clone()
}

async fn world(app: &App, pool: &PgPool, tag: &str) -> (Jar, Jar, Course) {
    let creator = signup_creator(app, &format!("creator-{tag}@x.in")).await;
    let student = signup_verified_as(app, &format!("student-{tag}@x.in"), "student").await;
    let course = free_course(app, pool, &creator, "Progress course").await;
    enroll(app, &student, &course).await;
    (creator, student, course)
}

db_test! {
    async fn heartbeats_build_progress_and_resume_belongs_to_the_lesson(pool: PgPool) {
        let app = app(pool.clone());
        let (_creator, student, course) = world(&app, &pool, "resume").await;

        let first = play(&app, &student, course.lesson).await;
        assert_eq!(first.status, StatusCode::CREATED, "{:?}", first.body);
        assert_eq!(first.body["playback"]["resume_from_seconds"], 0);
        let session = session_of(&first);

        assert_eq!(beat(&app, &student, session, 100).await.status, StatusCode::NO_CONTENT);
        assert_eq!(row(&pool, course.lesson).await, (100, 100, 0, false), "a first sighting earns nothing");
        thirty_seconds_later(&pool, course.lesson).await;
        beat(&app, &student, session, 130).await;
        assert_eq!(row(&pool, course.lesson).await, (130, 130, 30, false));

        let page = progress(&app, &student, course.id).await;
        assert_eq!(page.status, StatusCode::OK, "{:?}", page.body);
        let lesson = &page.body["progress"]["lessons"][0];
        assert_eq!(lesson["lesson_id"], json!(course.lesson));
        assert_eq!((lesson["percent_watched"].as_i64(), lesson["resume_from_seconds"].as_i64()), (Some(5), Some(130)));
        assert_eq!(page.body["progress"]["lessons_completed"], 0);

        // Stop, wipe every session (they are swept after 90 days), and press play again.
        app.req(Method::DELETE, &format!("/v1/play/{session}")).jar(&student).send().await;
        sqlx::query("delete from playback_sessions").execute(&pool).await.unwrap();
        let again = play(&app, &student, course.lesson).await;
        assert_eq!(again.body["playback"]["resume_from_seconds"], 130, "the lesson remembers");
    }
}

db_test! {
    async fn replacing_a_lesson_video_resets_progress_and_old_sessions_cannot_restore_it(pool: PgPool) {
        let app = app(pool.clone());
        let (creator, student, course) = world(&app, &pool, "replace").await;
        let old_session = session_of(&play(&app, &student, course.lesson).await);
        beat(&app, &student, old_session, 0).await;
        thirty_seconds_later(&pool, course.lesson).await;
        beat(&app, &student, old_session, 30).await;
        assert_eq!(row(&pool, course.lesson).await.2, 30);

        // Keep the original video accessible through another lesson, so its existing session
        // still heartbeats successfully after the first lesson changes video.
        let another = app.post(&format!("/v1/courses/{}/lessons", course.id)).jar(&creator)
            .json(json!({"title": "Original video", "video_id": course.video})).send().await;
        assert_eq!(another.status, StatusCode::CREATED);
        let second_lesson = Uuid::parse_str(another.body["lesson"]["id"].as_str().unwrap()).unwrap();
        let second_session = play(&app, &student, second_lesson).await;
        assert_eq!(second_session.body["playback"]["resume_from_seconds"], 0,
            "another lesson using the same video starts with its own progress");
        app.req(Method::DELETE, &format!("/v1/play/{}", session_of(&second_session)))
            .jar(&student).send().await;
        let new_video = Uuid::new_v4();
        let owner = owner_of(&pool, course.id).await;
        sqlx::query(
            "insert into videos(id, owner_id, title, original_filename, content_type, size_bytes, \
             part_size, source_key, status, duration_seconds) \
             values($1,$2,'Replacement','new.mp4','video/mp4',1,5242880,$3,'ready',$4)"
        ).bind(new_video).bind(owner).bind(format!("uploads/{new_video}"))
            .bind(LESSON_SECONDS).execute(&pool).await.unwrap();
        let changed = app.req(Method::PATCH, &format!("/v1/lessons/{}", course.lesson))
            .jar(&creator).json(json!({"video_id": new_video})).send().await;
        assert_eq!(changed.status, StatusCode::OK, "{:?}", changed.body);
        let count: i64 = sqlx::query_scalar("select count(*) from lesson_progress where lesson_id=$1")
            .bind(course.lesson).fetch_one(&pool).await.unwrap();
        assert_eq!(count, 0, "old video completion and resume are cleared");

        assert_eq!(beat(&app, &student, old_session, 60).await.status, StatusCode::NO_CONTENT);
        let count: i64 = sqlx::query_scalar("select count(*) from lesson_progress where lesson_id=$1")
            .bind(course.lesson).fetch_one(&pool).await.unwrap();
        assert_eq!(count, 0, "old session cannot recreate the cleared row");
        let new_session = play(&app, &student, course.lesson).await;
        assert_eq!(new_session.status, StatusCode::CREATED);
        assert_eq!(new_session.body["playback"]["resume_from_seconds"], 0);
    }
}

db_test! {
    async fn resume_near_the_end_starts_over(pool: PgPool) {
        let app = app(pool.clone());
        let (_creator, student, course) = world(&app, &pool, "end").await;
        let session = session_of(&play(&app, &student, course.lesson).await);
        beat(&app, &student, session, LESSON_SECONDS as u32 - 5).await;
        app.req(Method::DELETE, &format!("/v1/play/{session}")).jar(&student).send().await;
        let again = play(&app, &student, course.lesson).await;
        assert_eq!(again.body["playback"]["resume_from_seconds"], 0);
    }
}

db_test! {
    async fn skipping_to_the_end_does_not_complete_a_lesson(pool: PgPool) {
        let app = app(pool.clone());
        let (_creator, student, course) = world(&app, &pool, "skip").await;
        let session = session_of(&play(&app, &student, course.lesson).await);
        beat(&app, &student, session, 1).await;
        thirty_seconds_later(&pool, course.lesson).await;
        beat(&app, &student, session, LESSON_SECONDS as u32 - 2).await;

        let (position, furthest, watched, completed) = row(&pool, course.lesson).await;
        assert_eq!((position, furthest), (598, 598));
        // 30 s of real time earns at most 2 x 30 + 5 = 65 s; a loaded machine can let a few more
        // seconds pass between the time-travel and the heartbeat. 597 would be the bug.
        assert!(watched <= 80, "{watched} seconds credited for a jump of 597");
        assert!(!completed);
        let page = progress(&app, &student, course.id).await;
        assert_eq!(page.body["progress"]["lessons"][0]["completed"], false);
        assert_eq!(page.body["progress"]["percent_complete"], 0);
        assert_eq!(continuing(&app, &student).await.as_array().unwrap().len(), 1);
    }
}

db_test! {
    async fn watching_all_the_way_through_completes_and_leaves_continue_watching(pool: PgPool) {
        let app = app(pool.clone());
        let (_creator, student, course) = world(&app, &pool, "done").await;
        let session = session_of(&play(&app, &student, course.lesson).await);

        beat(&app, &student, session, 0).await;
        let mut position = 0;
        while position < LESSON_SECONDS as u32 {
            position = (position + 30).min(LESSON_SECONDS as u32);
            thirty_seconds_later(&pool, course.lesson).await;
            let response = beat(&app, &student, session, position).await;
            assert_eq!(response.status, StatusCode::NO_CONTENT, "at {position}");
        }
        let (_, _, watched, completed) = row(&pool, course.lesson).await;
        assert_eq!((watched, completed), (LESSON_SECONDS, true));
        let page = progress(&app, &student, course.id).await;
        assert_eq!(page.body["progress"]["lessons"][0]["completed"], true);
        assert_eq!(page.body["progress"]["lessons"][0]["percent_watched"], 100);
        assert_eq!(page.body["progress"]["percent_complete"], 100);
        assert!(continuing(&app, &student).await.as_array().unwrap().is_empty());

        // Replaying from the start later does not un-complete it.
        app.req(Method::DELETE, &format!("/v1/play/{session}")).jar(&student).send().await;
        let again = session_of(&play(&app, &student, course.lesson).await);
        thirty_seconds_later(&pool, course.lesson).await;
        beat(&app, &student, again, 5).await;
        assert!(row(&pool, course.lesson).await.3);
    }
}

db_test! {
    async fn two_screens_heartbeating_at_once_are_credited_once(pool: PgPool) {
        let app = app(pool.clone());
        let (_creator, student, course) = world(&app, &pool, "twin").await;
        let a = session_of(&play(&app, &student, course.lesson).await);
        let b = session_of(&play(&app, &student, course.lesson).await);
        beat(&app, &student, a, 100).await;
        thirty_seconds_later(&pool, course.lesson).await;
        let (x, y) = join!(beat(&app, &student, a, 130), beat(&app, &student, b, 130));
        assert_eq!((x.status, y.status), (StatusCode::NO_CONTENT, StatusCode::NO_CONTENT));
        assert_eq!(row(&pool, course.lesson).await.2, 30, "the same 30 seconds are not counted twice");
    }
}

db_test! {
    async fn continue_watching_is_per_course_newest_first_and_drops_what_you_can_no_longer_open(pool: PgPool) {
        let app = app(pool.clone());
        let creator = signup_creator(&app, "cw-creator@x.in").await;
        let student = signup_verified_as(&app, "cw-student@x.in", "student").await;
        let other = signup_verified_as(&app, "cw-other@x.in", "student").await;
        let first = free_course(&app, &pool, &creator, "First course").await;
        let second = free_course(&app, &pool, &creator, "Second course").await;
        enroll(&app, &student, &first).await;
        enroll(&app, &student, &second).await;

        let s1 = session_of(&play(&app, &student, first.lesson).await);
        beat(&app, &student, s1, 50).await;
        let s2 = session_of(&play(&app, &student, second.lesson).await);
        beat(&app, &student, s2, 70).await;

        let items = continuing(&app, &student).await;
        let items = items.as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["course_title"], "Second course", "newest first");
        assert_eq!(items[0]["resume_from_seconds"], 70);
        assert_eq!(items[0]["lesson"]["id"], json!(second.lesson));
        assert!(continuing(&app, &other).await.as_array().unwrap().is_empty(), "nobody else's");

        // Access disappears: the entry does too, rather than being offered and refused.
        let student_id: Uuid = sqlx::query_scalar("select id from users where email='cw-student@x.in'")
            .fetch_one(&pool).await.unwrap();
        let revoke = app.req(Method::DELETE, &format!("/v1/courses/{}/grants/{student_id}", second.id))
            .jar(&creator).json(json!({"reason": "test"})).send().await;
        assert!(revoke.status.is_success(), "{:?}", revoke.body);
        let items = continuing(&app, &student).await;
        let titles: Vec<_> = items.as_array().unwrap().iter().map(|i| i["course_title"].as_str().unwrap()).collect();
        assert_eq!(titles, ["First course"]);
    }
}

db_test! {
    async fn course_progress_is_for_enrolled_students_only(pool: PgPool) {
        let app = app(pool.clone());
        let (creator, student, course) = world(&app, &pool, "priv").await;
        let outsider = signup_verified_as(&app, "priv-outsider@x.in", "student").await;
        let uri = format!("/v1/courses/{}/progress", course.id);

        assert_eq!(app.get(&uri).send().await.status, StatusCode::UNAUTHORIZED);
        assert_eq!(progress(&app, &outsider, course.id).await.status, StatusCode::NOT_FOUND);
        assert_eq!(progress(&app, &creator, course.id).await.status, StatusCode::NOT_FOUND,
                   "a creator sees students' progress through the analytics, not as a student");
        assert_eq!(progress(&app, &student, course.id).await.status, StatusCode::OK);
        assert_eq!(app.get("/v1/courses/not-a-uuid/progress").jar(&student).send().await.status, StatusCode::NOT_FOUND);
        let page = progress(&app, &student, course.id).await;
        assert_eq!(
            page.body["progress"]["lessons"][0].as_object().unwrap().keys().cloned().collect::<std::collections::BTreeSet<_>>().len(),
            8
        );
    }
}

db_test! {
    async fn an_owners_preview_through_the_video_route_records_no_progress(pool: PgPool) {
        let app = app(pool.clone());
        let creator = signup_creator(&app, "preview-owner@x.in").await;
        let course = free_course(&app, &pool, &creator, "Preview course").await;
        let started = app.post(&format!("/v1/videos/{}/play", course.video)).jar(&creator).send().await;
        assert_eq!(started.status, StatusCode::CREATED, "{:?}", started.body);
        beat(&app, &creator, session_of(&started), 40).await;
        let rows: i64 = sqlx::query_scalar("select count(*) from lesson_progress").fetch_one(&pool).await.unwrap();
        assert_eq!(rows, 0);
    }
}
