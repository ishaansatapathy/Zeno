//! Uploading a recording while it is still being made: no size at the start, parts asked for one at a
//! time, the size stated at the end.

mod common;

use axum::http::StatusCode;
use common::*;
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

const MIB: usize = 1024 * 1024;
/// The smallest part S3 accepts for anything but the last.
const PART: usize = 5 * MIB;

fn bytes(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(7).wrapping_add(seed))
        .collect()
}

fn start_body() -> Value {
    json!({"filename": "Zeno recording.webm", "content_type": "video/webm", "part_size_bytes": PART})
}

async fn start(app: &App, jar: &Jar) -> (Uuid, Res) {
    let res = app
        .post("/v1/videos/streaming")
        .jar(jar)
        .json(start_body())
        .send()
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{:?}", res.body);
    let id = Uuid::parse_str(res.body["video"]["id"].as_str().unwrap()).unwrap();
    (id, res)
}

async fn part(app: &App, jar: &Jar, id: Uuid, number: u16, size: usize, last: bool) -> Res {
    app.post(&format!("/v1/videos/{id}/parts"))
        .jar(jar)
        .json(json!({"number": number, "size": size, "last": last}))
        .send()
        .await
}

async fn put_part(app: &App, jar: &Jar, id: Uuid, number: u16, data: Vec<u8>, last: bool) {
    let res = part(app, jar, id, number, data.len(), last).await;
    assert_eq!(res.status, StatusCode::OK, "{:?}", res.body);
    let url = res.body["part"]["url"].as_str().unwrap().to_owned();
    assert!(
        url.starts_with(storage_config().public_endpoint.as_str()),
        "URLs point at storage, not the API: {url}"
    );
    assert!(browser_put(&url, data).await.is_success());
}

async fn finish(app: &App, jar: &Jar, id: Uuid, size: usize) -> Res {
    app.post(&format!("/v1/videos/{id}/complete"))
        .jar(jar)
        .json(json!({"size_bytes": size}))
        .send()
        .await
}

db_test! {
    async fn a_lost_start_response_reuses_the_same_upload(pool: PgPool) {
        let app = app(pool.clone());
        let owner = signup_creator(&app, "owner@streaming.in").await;
        let other = signup_creator(&app, "other@streaming.in").await;
        let id = Uuid::new_v4();
        let body = json!({"id": id, "filename": "Zeno recording.webm", "content_type": "video/webm", "part_size_bytes": PART});
        let first = app.post("/v1/videos/streaming").jar(&owner).json(body.clone()).send().await;
        assert_eq!(first.status, StatusCode::CREATED, "{:?}", first.body);
        let again = app.post("/v1/videos/streaming").jar(&owner).json(body.clone()).send().await;
        assert_eq!(again.status, StatusCode::CREATED, "{:?}", again.body);
        assert_eq!(first.body["video"]["id"], again.body["video"]["id"]);
        assert_eq!(first.body["video"]["id"], id.to_string());
        let count: i64 = sqlx::query_scalar("select count(*) from videos where id = $1")
            .bind(id).fetch_one(&pool).await.unwrap();
        assert_eq!(count, 1);

        let changed = app.post("/v1/videos/streaming").jar(&owner)
            .json(json!({"id": id, "filename": "another.webm", "content_type": "video/webm", "part_size_bytes": PART}))
            .send().await;
        assert_eq!(changed.status, StatusCode::CONFLICT);
        assert_eq!(changed.code(), "streaming_id_conflict");
        let stranger = app.post("/v1/videos/streaming").jar(&other).json(body).send().await;
        assert_eq!(stranger.status, StatusCode::CONFLICT);
        assert_eq!(stranger.code(), "streaming_id_conflict");

        let racing_id = Uuid::new_v4();
        let racing = json!({"id": racing_id, "filename": "race.webm", "content_type": "video/webm", "part_size_bytes": PART});
        let (left, right) = tokio::join!(
            app.post("/v1/videos/streaming").jar(&owner).json(racing.clone()).send(),
            app.post("/v1/videos/streaming").jar(&owner).json(racing).send(),
        );
        assert_eq!(left.status, StatusCode::CREATED, "{:?}", left.body);
        assert_eq!(right.status, StatusCode::CREATED, "{:?}", right.body);
        assert_eq!(left.body["video"]["id"], right.body["video"]["id"]);
        let count: i64 = sqlx::query_scalar("select count(*) from videos where id = $1")
            .bind(racing_id).fetch_one(&pool).await.unwrap();
        assert_eq!(count, 1);
        // The losing request aborted only its own multipart upload, not the winner's.
        put_part(&app, &owner, racing_id, 1, bytes(PART, 31), true).await;
        let completed = finish(&app, &owner, racing_id, PART).await;
        assert_eq!(completed.status, StatusCode::OK, "{:?}", completed.body);
    }
}

db_test! {
    async fn a_recording_uploads_part_by_part_and_its_size_is_stated_at_the_end(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_creator(&app, "t@x.in").await;

        let (id, created) = start(&app, &jar).await;
        assert_eq!(created.body["video"]["status"], "uploading");
        assert_eq!(created.body["upload"]["part_size"], PART);
        assert_eq!(created.body["upload"]["total_parts"], 0);
        assert_eq!(created.body["upload"]["parts"], json!([]));

        let total = 2 * PART + MIB;
        let payload = bytes(total, 3);
        put_part(&app, &jar, id, 1, payload[..PART].to_vec(), false).await;
        put_part(&app, &jar, id, 2, payload[PART..2 * PART].to_vec(), false).await;
        put_part(&app, &jar, id, 3, payload[2 * PART..].to_vec(), true).await;

        let done = finish(&app, &jar, id, total).await;
        assert_eq!(done.status, StatusCode::OK, "{:?}", done.body);
        assert_eq!(done.body["video"]["status"], "processing");
        assert_eq!(done.body["video"]["size_bytes"], total);

        let stored = app.state.store.head(&format!("uploads/{id}/source")).await.unwrap();
        assert_eq!(stored, Some(total as u64));
        assert_eq!(count(&pool, "select count(*) from jobs where kind = 'transcode'").await, 1);

        let again = finish(&app, &jar, id, total).await;
        assert_eq!(again.status, StatusCode::OK, "completing twice is not an error: {:?}", again.body);
        assert_eq!(count(&pool, "select count(*) from jobs").await, 1);
    }
}

db_test! {
    async fn a_recording_shorter_than_one_part_is_a_single_last_part(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_creator(&app, "t@x.in").await;
        let (id, _) = start(&app, &jar).await;
        put_part(&app, &jar, id, 1, bytes(MIB, 1), true).await;
        let done = finish(&app, &jar, id, MIB).await;
        assert_eq!(done.status, StatusCode::OK, "{:?}", done.body);
    }
}

db_test! {
    async fn resuming_lists_the_full_parts_storage_holds_and_a_lost_part_can_be_asked_for_again(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_creator(&app, "t@x.in").await;
        let (id, _) = start(&app, &jar).await;
        put_part(&app, &jar, id, 1, bytes(PART, 1), false).await;
        put_part(&app, &jar, id, 2, bytes(PART, 2), false).await;

        let resumed = app.get(&format!("/v1/videos/{id}/upload")).jar(&jar).send().await;
        assert_eq!(resumed.status, StatusCode::OK, "{:?}", resumed.body);
        assert_eq!(resumed.body["uploaded"], json!([1, 2]));

        // Asking for part 2 again gives a new URL, and putting it again replaces it.
        put_part(&app, &jar, id, 2, bytes(PART, 9), false).await;
        let resumed = app.get(&format!("/v1/videos/{id}/upload")).jar(&jar).send().await;
        assert_eq!(resumed.body["uploaded"], json!([1, 2]));
    }
}

db_test! {
    async fn a_size_that_does_not_add_up_to_the_parts_held_is_refused_and_nothing_is_queued(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_creator(&app, "t@x.in").await;
        let (id, _) = start(&app, &jar).await;
        put_part(&app, &jar, id, 1, bytes(PART, 1), false).await;
        put_part(&app, &jar, id, 2, bytes(MIB, 2), true).await;

        // Says there are three parts' worth, but only two were uploaded.
        let too_big = finish(&app, &jar, id, 2 * PART + MIB).await;
        assert_eq!(too_big.status, StatusCode::CONFLICT, "{:?}", too_big.body);
        assert_eq!(too_big.code(), "upload_incomplete");
        // Says one part, but two were uploaded: the object would not be the size stated.
        let too_small = finish(&app, &jar, id, PART).await;
        assert!(!too_small.status.is_success(), "{:?}", too_small.body);
        assert_eq!(count(&pool, "select count(*) from jobs").await, 0);

        let right = finish(&app, &jar, id, PART + MIB).await;
        assert_eq!(right.status, StatusCode::OK, "{:?}", right.body);
    }
}

db_test! {
    async fn a_recording_must_say_how_big_it_ended_up(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_creator(&app, "t@x.in").await;
        let (id, _) = start(&app, &jar).await;
        put_part(&app, &jar, id, 1, bytes(MIB, 1), true).await;

        let none = app.post(&format!("/v1/videos/{id}/complete")).jar(&jar).send().await;
        assert_eq!(none.status, StatusCode::UNPROCESSABLE_ENTITY, "{:?}", none.body);
        let zero = finish(&app, &jar, id, 0).await;
        assert_eq!(zero.status, StatusCode::UNPROCESSABLE_ENTITY, "{:?}", zero.body);
        let huge = finish(&app, &jar, id, 1 << 50).await;
        assert_eq!(huge.status, StatusCode::UNPROCESSABLE_ENTITY, "{:?}", huge.body);
        let junk = app
            .post(&format!("/v1/videos/{id}/complete"))
            .jar(&jar)
            .json(json!({"size": 5}))
            .send()
            .await;
        assert!(junk.status.is_client_error(), "{:?}", junk.body);
        assert_eq!(count(&pool, "select count(*) from jobs").await, 0);
    }
}

db_test! {
    async fn the_start_and_each_part_are_validated(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_creator(&app, "t@x.in").await;

        for body in [
            json!({"filename": "a.webm", "content_type": "video/webm", "part_size_bytes": 1024}),
            json!({"filename": "a.webm", "content_type": "video/webm", "part_size_bytes": 65 * MIB}),
            json!({"filename": "a.webm", "content_type": "text/plain"}),
        ] {
            let res = app.post("/v1/videos/streaming").jar(&jar).json(body.clone()).send().await;
            assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{body}: {:?}", res.body);
        }
        // Default part size when none is given.
        let res = app
            .post("/v1/videos/streaming")
            .jar(&jar)
            .json(json!({"filename": "a.webm", "content_type": "video/webm"}))
            .send()
            .await;
        assert_eq!(res.status, StatusCode::CREATED, "{:?}", res.body);
        assert_eq!(res.body["upload"]["part_size"], 16 * MIB);

        let (id, _) = start(&app, &jar).await;
        // Not the last, so exactly the part size; the last, at most the part size; numbered from 1.
        assert_eq!(part(&app, &jar, id, 1, PART - 1, false).await.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(part(&app, &jar, id, 1, PART + 1, true).await.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(part(&app, &jar, id, 1, 0, true).await.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(part(&app, &jar, id, 0, PART, false).await.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(part(&app, &jar, id, 60_000, PART, false).await.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(part(&app, &jar, id, 1, PART, false).await.status, StatusCode::OK);
        assert_eq!(part(&app, &jar, id, 1, 1, true).await.status, StatusCode::OK);
    }
}

db_test! {
    async fn parts_are_only_for_the_owners_recordings_in_progress(pool: PgPool) {
        let app = app(pool.clone());
        let owner = signup_creator(&app, "owner@x.in").await;
        let other = signup_creator(&app, "other@x.in").await;
        let (id, _) = start(&app, &owner).await;

        assert_eq!(part(&app, &other, id, 1, PART, false).await.status, StatusCode::NOT_FOUND);
        let theirs = finish(&app, &other, id, PART).await;
        assert_eq!(theirs.status, StatusCode::NOT_FOUND);
        let anonymous = app
            .post(&format!("/v1/videos/{id}/parts"))
            .json(json!({"number": 1, "size": PART}))
            .send()
            .await;
        assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);

        // A file picked from disk takes all its parts at once, not one by one.
        let classic = app
            .post("/v1/videos")
            .jar(&owner)
            .json(json!({"filename": "a.mp4", "size_bytes": MIB, "content_type": "video/mp4"}))
            .send()
            .await;
        let classic_id = Uuid::parse_str(classic.body["video"]["id"].as_str().unwrap()).unwrap();
        let res = part(&app, &owner, classic_id, 1, MIB, true).await;
        assert_eq!(res.status, StatusCode::CONFLICT, "{:?}", res.body);
        assert_eq!(res.code(), "not_streaming");
        // ... and a size stated for it must be the size it was started with.
        let wrong = finish(&app, &owner, classic_id, 2 * MIB).await;
        assert_eq!(wrong.status, StatusCode::UNPROCESSABLE_ENTITY, "{:?}", wrong.body);
    }
}

db_test! {
    async fn a_recording_nobody_finishes_is_abandoned_like_any_other_upload(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_creator(&app, "t@x.in").await;
        let (id, _) = start(&app, &jar).await;
        put_part(&app, &jar, id, 1, bytes(PART, 1), false).await;
        sqlx::query("update videos set updated_at = now() - interval '3 days', created_at = now() - interval '3 days' where id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
        zeno_api::videos::service::abandon_stale_uploads(&app.state).await.unwrap();
        let left = count(&pool, "select count(*) from videos where status = 'uploading'").await;
        assert_eq!(left, 0, "the stale upload is gone");    }
}
