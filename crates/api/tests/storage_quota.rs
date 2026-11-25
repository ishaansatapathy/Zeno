//! How much a person may keep: what is counted, what is refused, what gives room back, and that uploads started
//! at the same moment cannot all be told there is room for the last of it.

mod common;

use axum::http::{Method, StatusCode};
use common::*;
use futures_util::future::join_all;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

macro_rules! db_test {
    ($(#[$meta:meta])* async fn $name:ident($pool:ident: PgPool) $body:block) => {
        $(#[$meta])*
        #[sqlx::test(migrator = "zeno_core::db::MIGRATOR")]
        async fn $name($pool: PgPool) $body
    };
}

const MIB: u64 = 1024 * 1024;
/// The smallest part S3 accepts for anything but the last.
const PART: usize = 5 * 1024 * 1024;

fn with_quota(pool: PgPool, quota: u64) -> App {
    app_with_tweaked_config(pool, |config| config.default_storage_quota_bytes = quota)
}

fn new_video(size: u64) -> Value {
    json!({"filename": "Lecture.mp4", "size_bytes": size, "content_type": "video/mp4"})
}

async fn upload(app: &App, jar: &Jar, size: u64) -> Res {
    app.post("/v1/videos")
        .jar(jar)
        .json(new_video(size))
        .send()
        .await
}

async fn storage(app: &App, jar: &Jar) -> Value {
    let res = app.get("/v1/me/storage").jar(jar).send().await;
    assert_eq!(res.status, StatusCode::OK, "{:?}", res.body);
    res.body["storage"].clone()
}

async fn start_recording(app: &App, jar: &Jar) -> Res {
    app.post("/v1/videos/streaming")
        .jar(jar)
        .json(json!({"filename": "Zeno recording.webm", "content_type": "video/webm", "part_size_bytes": PART}))
        .send()
        .await
}

async fn ask_part(app: &App, jar: &Jar, id: &str, number: u16, size: usize, last: bool) -> Res {
    app.post(&format!("/v1/videos/{id}/parts"))
        .jar(jar)
        .json(json!({"number": number, "size": size, "last": last}))
        .send()
        .await
}

db_test! {
    async fn it_says_what_is_used_and_what_is_left(pool: PgPool) {
        let app = with_quota(pool, 100 * MIB);
        let asha = signup_creator(&app, "asha@quota.in").await;
        let before = storage(&app, &asha).await;
        assert_eq!(
            before,
            json!({"used_bytes": 0, "quota_bytes": 100 * MIB, "remaining_bytes": 100 * MIB, "videos": 0})
        );

        assert_eq!(upload(&app, &asha, 30 * MIB).await.status, StatusCode::CREATED);
        assert_eq!(upload(&app, &asha, 5 * MIB).await.status, StatusCode::CREATED);
        let after = storage(&app, &asha).await;
        assert_eq!(
            after,
            json!({"used_bytes": 35 * MIB, "quota_bytes": 100 * MIB, "remaining_bytes": 65 * MIB, "videos": 2})
        );

        // Somebody else's videos are not mine.
        let ben = signup_creator(&app, "ben@quota.in").await;
        assert_eq!(storage(&app, &ben).await["used_bytes"], 0);

        // Signed out there is nothing to say.
        let nobody = app.get("/v1/me/storage").send().await;
        assert_eq!(nobody.status, StatusCode::UNAUTHORIZED);
    }
}

db_test! {
    async fn an_upload_that_does_not_fit_is_refused_and_leaves_nothing_behind(pool: PgPool) {
        let app = with_quota(pool.clone(), 100 * MIB);
        let asha = signup_creator(&app, "asha@quota.in").await;
        assert_eq!(upload(&app, &asha, 60 * MIB).await.status, StatusCode::CREATED);

        let refused = upload(&app, &asha, 50 * MIB).await;
        assert_eq!(refused.status, StatusCode::PAYLOAD_TOO_LARGE, "{:?}", refused.body);
        assert_eq!(refused.code(), "storage_quota_exceeded");
        let message = refused.body["error"]["message"].as_str().unwrap();
        assert!(message.contains("50.0 MB") && message.contains("40.0 MB") && message.contains("100.0 MB"), "{message}");
        let rows: i64 = sqlx::query_scalar("select count(*) from videos").fetch_one(&pool).await.unwrap();
        assert_eq!(rows, 1, "a refused upload has no row");

        // Exactly what is left fits; one byte more does not.
        assert_eq!(upload(&app, &asha, 40 * MIB).await.status, StatusCode::CREATED);
        let full = storage(&app, &asha).await;
        assert_eq!(full["remaining_bytes"], 0);
        let one_more = upload(&app, &asha, 1).await;
        assert_eq!(one_more.status, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(one_more.code(), "storage_quota_exceeded");
    }
}

db_test! {
    async fn deleting_gives_the_room_back_and_a_failed_video_never_counted(pool: PgPool) {
        let app = with_quota(pool.clone(), 60 * MIB);
        let asha = signup_creator(&app, "asha@quota.in").await;
        let big = upload(&app, &asha, 50 * MIB).await;
        let id = big.body["video"]["id"].as_str().unwrap().to_owned();
        assert_eq!(upload(&app, &asha, 20 * MIB).await.code(), "storage_quota_exceeded");

        let gone = app.req(Method::DELETE, &format!("/v1/videos/{id}")).jar(&asha).send().await;
        assert_eq!(gone.status, StatusCode::NO_CONTENT, "{:?}", gone.body);
        assert_eq!(storage(&app, &asha).await["used_bytes"], 0, "a deleted video is not kept");
        let again = upload(&app, &asha, 50 * MIB).await;
        assert_eq!(again.status, StatusCode::CREATED, "{:?}", again.body);

        // One that could not be made (the file was not a video) has nothing stored worth counting.
        let id = again.body["video"]["id"].as_str().unwrap();
        sqlx::query("update videos set status = 'failed' where id = $1::uuid")
            .bind(id).execute(&pool).await.unwrap();
        assert_eq!(storage(&app, &asha).await["used_bytes"], 0);
        assert_eq!(upload(&app, &asha, 60 * MIB).await.status, StatusCode::CREATED);
    }
}

db_test! {
    async fn an_accounts_own_limit_beats_the_default_and_touches_nobody_else(pool: PgPool) {
        let app = with_quota(pool.clone(), 100 * MIB);
        let asha = signup_creator(&app, "asha@quota.in").await;
        let ben = signup_creator(&app, "ben@quota.in").await;
        sqlx::query("update users set storage_quota_bytes = $2 where email = $1")
            .bind("asha@quota.in").bind(10 * MIB as i64).execute(&pool).await.unwrap();

        assert_eq!(storage(&app, &asha).await["quota_bytes"], 10 * MIB);
        assert_eq!(upload(&app, &asha, 11 * MIB).await.code(), "storage_quota_exceeded");
        assert_eq!(upload(&app, &asha, 10 * MIB).await.status, StatusCode::CREATED);
        // The other account still has the default.
        assert_eq!(storage(&app, &ben).await["quota_bytes"], 100 * MIB);
        assert_eq!(upload(&app, &ben, 50 * MIB).await.status, StatusCode::CREATED);

        // An account may be given more than the default, and none at all.
        sqlx::query("update users set storage_quota_bytes = 0 where email = $1")
            .bind("ben@quota.in").execute(&pool).await.unwrap();
        assert_eq!(storage(&app, &ben).await["remaining_bytes"], 0, "never below zero");
        assert_eq!(upload(&app, &ben, 1).await.code(), "storage_quota_exceeded");
    }
}

db_test! {
    async fn a_recording_is_told_while_it_is_made_and_still_has_its_own_copy(pool: PgPool) {
        // Room for two parts and one more megabyte: 11 MiB.
        let app = with_quota(pool.clone(), 11 * MIB);
        let asha = signup_creator(&app, "asha@quota.in").await;
        let started = start_recording(&app, &asha).await;
        assert_eq!(started.status, StatusCode::CREATED, "{:?}", started.body);
        let id = started.body["video"]["id"].as_str().unwrap().to_owned();

        for number in 1..=2u16 {
            let res = ask_part(&app, &asha, &id, number, PART, false).await;
            assert_eq!(res.status, StatusCode::OK, "part {number}: {:?}", res.body);
        }
        // The third would end at 15 MiB.
        let third = ask_part(&app, &asha, &id, 3, PART, false).await;
        assert_eq!(third.status, StatusCode::PAYLOAD_TOO_LARGE, "{:?}", third.body);
        assert_eq!(third.code(), "storage_quota_exceeded");
        // A short last part that still fits (10 MiB + 1 MiB = 11 MiB, exactly the room) is allowed.
        let last = ask_part(&app, &asha, &id, 3, MIB as usize, true).await;
        assert_eq!(last.status, StatusCode::OK, "{:?}", last.body);
        let one_over = ask_part(&app, &asha, &id, 3, MIB as usize + 1, true).await;
        assert_eq!(one_over.code(), "storage_quota_exceeded");
        // Asking for a part that was already given is never refused for room it already has.
        assert_eq!(ask_part(&app, &asha, &id, 1, PART, false).await.status, StatusCode::OK);
    }
}

db_test! {
    async fn what_else_the_person_keeps_counts_against_a_recording(pool: PgPool) {
        let app = with_quota(pool.clone(), 30 * MIB);
        let asha = signup_creator(&app, "asha@quota.in").await;
        assert_eq!(upload(&app, &asha, 22 * MIB).await.status, StatusCode::CREATED);
        let started = start_recording(&app, &asha).await;
        assert_eq!(started.status, StatusCode::CREATED);
        let id = started.body["video"]["id"].as_str().unwrap().to_owned();
        // 22 MiB kept, so a recording has 8 MiB: one whole part and no second.
        assert_eq!(ask_part(&app, &asha, &id, 1, PART, false).await.status, StatusCode::OK);
        assert_eq!(ask_part(&app, &asha, &id, 2, PART, false).await.code(), "storage_quota_exceeded");
    }
}

db_test! {
    async fn a_recording_cannot_start_when_there_is_no_room_at_all(pool: PgPool) {
        let app = with_quota(pool.clone(), 0);
        let asha = signup_creator(&app, "asha@quota.in").await;
        let started = start_recording(&app, &asha).await;
        assert_eq!(started.status, StatusCode::PAYLOAD_TOO_LARGE, "{:?}", started.body);
        assert_eq!(started.code(), "storage_quota_exceeded");
        let rows: i64 = sqlx::query_scalar("select count(*) from videos").fetch_one(&pool).await.unwrap();
        assert_eq!(rows, 0);
    }
}

db_test! {
    async fn uploads_started_at_the_same_moment_cannot_all_be_told_there_is_room(pool: PgPool) {
        // Room for two of 40 MiB. Eight are asked for at once: each looks at the person's total and sees room, so
        // without the lock the count would be well over two.
        let app = with_quota(pool.clone(), 100 * MIB);
        let asha = signup_creator(&app, "asha@quota.in").await;
        let attempts = (0..8).map(|_| upload(&app, &asha, 40 * MIB));
        let answers = join_all(attempts).await;
        let made = answers.iter().filter(|r| r.status == StatusCode::CREATED).count();
        let refused = answers
            .iter()
            .filter(|r| r.status == StatusCode::PAYLOAD_TOO_LARGE && r.code() == "storage_quota_exceeded")
            .count();
        assert_eq!(made, 2, "{:?}", answers.iter().map(|r| r.status).collect::<Vec<_>>());
        assert_eq!(made + refused, 8, "every answer is one or the other: {:?}", answers.iter().map(|r| (r.status, r.code().to_owned())).collect::<Vec<_>>());
        let used: i64 = sqlx::query_scalar("select coalesce(sum(size_bytes), 0)::bigint from videos").fetch_one(&pool).await.unwrap();
        assert_eq!(used, 80 * MIB as i64, "what was kept is within the quota");
        assert!(storage(&app, &asha).await["remaining_bytes"].as_i64().unwrap() >= 0);
    }
}

db_test! {
    async fn two_people_do_not_wait_for_each_others_room(pool: PgPool) {
        let app = with_quota(pool.clone(), 50 * MIB);
        let asha = signup_creator(&app, "asha@quota.in").await;
        let ben = signup_creator(&app, "ben@quota.in").await;
        let mine = (0..4).map(|_| upload(&app, &asha, 20 * MIB));
        let theirs = (0..4).map(|_| upload(&app, &ben, 20 * MIB));
        let (mine, theirs) = futures_util::future::join(join_all(mine), join_all(theirs)).await;
        for answers in [&mine, &theirs] {
            let made = answers.iter().filter(|r| r.status == StatusCode::CREATED).count();
            assert_eq!(made, 2, "each has room for two of 20 MiB in 50: {:?}", answers.iter().map(|r| r.status).collect::<Vec<_>>());
        }
    }
}

db_test! {
    async fn a_video_made_before_the_quota_existed_still_counts(pool: PgPool) {
        let app = with_quota(pool.clone(), 100 * MIB);
        let asha = signup_creator(&app, "asha@quota.in").await;
        let owner: Uuid = sqlx::query_scalar("select id from users where email = 'asha@quota.in'")
            .fetch_one(&pool).await.unwrap();
        // A ready video, as the worker leaves it.
        sqlx::query(
            "insert into videos (id, owner_id, original_filename, content_type, size_bytes, part_size, source_key, status) \
             values ($1, $2, 'old.mp4', 'video/mp4', $3, 5242880, 'uploads/old/source', 'ready')",
        )
        .bind(Uuid::new_v4()).bind(owner).bind(95 * MIB as i64)
        .execute(&pool).await.unwrap();
        assert_eq!(storage(&app, &asha).await["used_bytes"], 95 * MIB);
        assert_eq!(upload(&app, &asha, 6 * MIB).await.code(), "storage_quota_exceeded");
        assert_eq!(upload(&app, &asha, 5 * MIB).await.status, StatusCode::CREATED);
    }
}
