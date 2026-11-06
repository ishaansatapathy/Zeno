mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;
use zeno_api::videos::service::{abandon_stale_uploads, purge_deleted, purge_old_sources};
use zeno_core::storage::StorageError;

macro_rules! db_test {
    ($(#[$meta:meta])* async fn $name:ident($pool:ident: PgPool) $body:block) => {
        $(#[$meta])*
        #[sqlx::test(migrator = "zeno_core::db::MIGRATOR")]
        async fn $name($pool: PgPool) $body
    };
}

const MIB: usize = 1024 * 1024;

fn bytes(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(7).wrapping_add(seed))
        .collect()
}

fn create_body(size: usize) -> Value {
    json!({"filename": "Lecture 1.mp4", "size_bytes": size, "content_type": "video/mp4"})
}

async fn create(app: &App, jar: &Jar, size: usize) -> Res {
    app.post("/v1/videos")
        .jar(jar)
        .json(create_body(size))
        .send()
        .await
}

fn video_id(res: &Res) -> Uuid {
    Uuid::parse_str(res.body["video"]["id"].as_str().expect("video id")).unwrap()
}

fn part_urls(upload: &Value) -> Vec<(u64, u64, String)> {
    upload["parts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["number"].as_u64().unwrap(),
                p["size"].as_u64().unwrap(),
                p["url"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

/// 17 MiB = a 16 MiB first part and a 1 MiB last part.
const TWO_PART_SIZE: usize = 17 * MIB;

db_test! {
    async fn a_creator_can_upload_in_parts_straight_to_storage_and_complete(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_creator(&app, "t@x.in").await;

        let created = create(&app, &jar, TWO_PART_SIZE).await;
        assert_eq!(created.status, StatusCode::CREATED, "{:?}", created.body);
        let id = video_id(&created);
        assert_eq!(created.body["video"]["status"], "uploading");
        assert_eq!(created.body["upload"]["total_parts"], 2);
        let parts = part_urls(&created.body["upload"]);
        assert_eq!(parts.iter().map(|p| (p.0, p.1)).collect::<Vec<_>>(), vec![(1, 16 * MIB as u64), (2, MIB as u64)]);
        assert!(parts[0].2.starts_with("http://127.0.0.1:9000/"), "URLs point at storage, not the API: {}", parts[0].2);

        let payload = bytes(TWO_PART_SIZE, 9);
        assert!(browser_put(&parts[0].2, payload[..16 * MIB].to_vec()).await.is_success());
        assert!(browser_put(&parts[1].2, payload[16 * MIB..].to_vec()).await.is_success());

        let done = app.post(&format!("/v1/videos/{id}/complete")).jar(&jar).send().await;
        assert_eq!(done.status, StatusCode::OK, "{:?}", done.body);
        assert_eq!(done.body["video"]["status"], "processing");

        let stored = app.state.store.head(&format!("uploads/{id}/source")).await.unwrap();
        assert_eq!(stored, Some(TWO_PART_SIZE as u64));
        let jobs: Vec<(String, Value)> = sqlx::query_as("select kind, payload from jobs").fetch_all(&pool).await.unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].0, "transcode");
        assert_eq!(jobs[0].1["video_id"], id.to_string());

        let shown = app.get(&format!("/v1/videos/{id}")).jar(&jar).send().await;
        assert_eq!(shown.body["video"]["status"], "processing");
        assert_eq!(shown.body["video"]["original_filename"], "Lecture 1.mp4");
        assert_eq!(shown.body["video"]["title"], "Lecture 1");
    }
}

db_test! {
    async fn an_interrupted_upload_resumes_with_urls_for_only_the_missing_parts(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_creator(&app, "t@x.in").await;
        let created = create(&app, &jar, TWO_PART_SIZE).await;
        let id = video_id(&created);
        let parts = part_urls(&created.body["upload"]);
        assert!(browser_put(&parts[0].2, bytes(16 * MIB, 1)).await.is_success());

        let resumed = app.get(&format!("/v1/videos/{id}/upload")).jar(&jar).send().await;
        assert_eq!(resumed.status, StatusCode::OK, "{:?}", resumed.body);
        assert_eq!(resumed.body["uploaded"], json!([1]));
        let missing = part_urls(&resumed.body);
        assert_eq!(missing.iter().map(|p| p.0).collect::<Vec<_>>(), vec![2]);

        let early = app.post(&format!("/v1/videos/{id}/complete")).jar(&jar).send().await;
        assert_eq!(early.status, StatusCode::CONFLICT);
        assert_eq!(early.code(), "upload_incomplete");
        assert!(early.body["error"]["message"].as_str().unwrap().contains('2'));
        assert_eq!(count(&pool, "select count(*) from jobs").await, 0, "nothing is queued for an incomplete upload");

        assert!(browser_put(&missing[0].2, bytes(MIB, 2)).await.is_success());
        let done = app.post(&format!("/v1/videos/{id}/complete")).jar(&jar).send().await;
        assert_eq!(done.status, StatusCode::OK);
    }
}

db_test! {
    async fn storage_itself_refuses_a_part_of_the_wrong_size_so_a_client_cannot_exceed_its_declared_size(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_creator(&app, "t@x.in").await;
        let created = create(&app, &jar, TWO_PART_SIZE).await;
        let parts = part_urls(&created.body["upload"]);

        let oversized = browser_put(&parts[1].2, bytes(2 * MIB, 3)).await;
        assert!(oversized.is_client_error(), "got {oversized}");
        let undersized = browser_put(&parts[0].2, bytes(15 * MIB, 3)).await;
        assert!(undersized.is_client_error(), "got {undersized}");
    }
}

db_test! {
    async fn completing_twice_or_concurrently_queues_exactly_one_transcode_job(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_creator(&app, "t@x.in").await;
        let created = create(&app, &jar, MIB).await;
        let id = video_id(&created);
        let parts = part_urls(&created.body["upload"]);
        assert!(browser_put(&parts[0].2, bytes(MIB, 4)).await.is_success());

        let handles: Vec<_> = (0..6)
            .map(|_| tokio::spawn(app.post(&format!("/v1/videos/{id}/complete")).jar(&jar).send()))
            .collect();
        for handle in handles {
            let res = handle.await.unwrap();
            assert_eq!(res.status, StatusCode::OK, "{:?}", res.body);
            assert_eq!(res.body["video"]["status"], "processing");
        }
        let again = app.post(&format!("/v1/videos/{id}/complete")).jar(&jar).send().await;
        assert_eq!(again.status, StatusCode::OK);
        assert_eq!(count(&pool, "select count(*) from jobs").await, 1);
    }
}

db_test! {
    async fn only_creators_can_upload_and_nobody_else_can_touch_someone_elses_video(pool: PgPool) {
        let app = app(pool.clone());
        let student = signup_verified(&app, "s@x.in").await;
        let owner = signup_creator(&app, "owner@x.in").await;
        let other = signup_creator(&app, "other@x.in").await;

        assert_eq!(app.post("/v1/videos").json(create_body(MIB)).send().await.status, StatusCode::UNAUTHORIZED);
        let forbidden = create(&app, &student, MIB).await;
        assert_eq!(forbidden.status, StatusCode::FORBIDDEN);
        assert_eq!(count(&pool, "select count(*) from videos").await, 0);

        let id = video_id(&create(&app, &owner, MIB).await);
        for (method, path) in [("GET", format!("/v1/videos/{id}")), ("GET", format!("/v1/videos/{id}/upload")),
                               ("POST", format!("/v1/videos/{id}/complete")), ("POST", format!("/v1/videos/{id}/abort"))] {
            let req = if method == "GET" { app.get(&path) } else { app.post(&path) };
            let res = req.jar(&other).send().await;
            assert_eq!(res.status, StatusCode::NOT_FOUND, "{method} {path}");
            assert_eq!(res.code(), "not_found", "someone else's video must look like a missing one");
        }
        let student_view = app.get(&format!("/v1/videos/{id}")).jar(&student).send().await;
        assert_eq!(student_view.status, StatusCode::NOT_FOUND);
        assert_eq!(app.get(&format!("/v1/videos/{id}")).jar(&owner).send().await.status, StatusCode::OK);
    }
}

db_test! {
    async fn an_admin_can_view_any_video_but_not_drive_its_upload(pool: PgPool) {
        let app = app(pool.clone());
        let owner = signup_creator(&app, "owner@x.in").await;
        signup_verified_as(&app, "boss@x.in", "creator").await;
        sqlx::query("update users set role = 'admin' where email = 'boss@x.in'").execute(&pool).await.unwrap();
        let login = login(&app, "boss@x.in", PASSWORD).await;
        let admin = Jar::of(&login);

        let id = video_id(&create(&app, &owner, MIB).await);
        assert_eq!(app.get(&format!("/v1/videos/{id}")).jar(&admin).send().await.status, StatusCode::OK);
        assert_eq!(app.post(&format!("/v1/videos/{id}/abort")).jar(&admin).send().await.status, StatusCode::NOT_FOUND);
    }
}

db_test! {
    async fn bad_requests_are_rejected_before_anything_is_created(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_creator(&app, "t@x.in").await;

        let cases = [
            json!({"filename":"a.mp4","size_bytes":0,"content_type":"video/mp4"}),
            json!({"filename":"a.mp4","size_bytes": MAX_UPLOAD_BYTES + 1,"content_type":"video/mp4"}),
            json!({"filename":"a.exe","size_bytes":10,"content_type":"application/x-msdownload"}),
            json!({"filename":"a.mp4","size_bytes":10,"content_type":"video/mp4","owner_id":"someone-else"}),
            json!({"filename":"a.mp4","size_bytes":-5,"content_type":"video/mp4"}),
            json!({"filename":"a.mp4","content_type":"video/mp4"}),
        ];
        for body in cases {
            let res = app.post("/v1/videos").jar(&jar).json(body.clone()).send().await;
            assert!(
                res.status == StatusCode::UNPROCESSABLE_ENTITY || res.status == StatusCode::BAD_REQUEST,
                "{body}: {} {:?}", res.status, res.body
            );
        }
        assert_eq!(count(&pool, "select count(*) from videos").await, 0);
        let bad_id = app.get("/v1/videos/not-a-uuid").jar(&jar).send().await;
        assert_eq!((bad_id.status, bad_id.code()), (StatusCode::NOT_FOUND, "not_found"));
    }
}

db_test! {
    async fn a_creator_cannot_pile_up_unfinished_uploads(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_creator(&app, "t@x.in").await;
        let mut first = None;
        for _ in 0..10 {
            let res = create(&app, &jar, MIB).await;
            assert_eq!(res.status, StatusCode::CREATED);
            first.get_or_insert(video_id(&res));
        }
        let blocked = create(&app, &jar, MIB).await;
        assert_eq!(blocked.status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(blocked.code(), "too_many_uploads");

        app.post(&format!("/v1/videos/{}/abort", first.unwrap())).jar(&jar).send().await;
        assert_eq!(create(&app, &jar, MIB).await.status, StatusCode::CREATED, "cancelling frees a slot");
    }
}

db_test! {
    async fn cancelling_removes_the_video_and_its_pending_storage_upload(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_creator(&app, "t@x.in").await;
        let created = create(&app, &jar, MIB).await;
        let id = video_id(&created);
        let parts = part_urls(&created.body["upload"]);
        assert!(browser_put(&parts[0].2, bytes(MIB, 5)).await.is_success());
        let upload_id: String = sqlx::query_scalar("select upload_id from videos").fetch_one(&pool).await.unwrap();

        let res = app.post(&format!("/v1/videos/{id}/abort")).jar(&jar).send().await;
        assert_eq!(res.status, StatusCode::NO_CONTENT);
        assert_eq!(count(&pool, "select count(*) from videos").await, 0);
        let listed = app.state.store.list_parts(&format!("uploads/{id}/source"), &upload_id).await;
        assert!(matches!(listed, Err(StorageError::NotFound)), "{listed:?}");

        assert_eq!(app.get(&format!("/v1/videos/{id}")).jar(&jar).send().await.status, StatusCode::NOT_FOUND);
    }
}

db_test! {
    async fn a_video_that_is_already_processing_cannot_be_cancelled_or_resumed(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_creator(&app, "t@x.in").await;
        let created = create(&app, &jar, MIB).await;
        let id = video_id(&created);
        browser_put(&part_urls(&created.body["upload"])[0].2, bytes(MIB, 6)).await;
        app.post(&format!("/v1/videos/{id}/complete")).jar(&jar).send().await;

        let abort = app.post(&format!("/v1/videos/{id}/abort")).jar(&jar).send().await;
        assert_eq!((abort.status, abort.code()), (StatusCode::CONFLICT, "not_abortable"));
        let resume = app.get(&format!("/v1/videos/{id}/upload")).jar(&jar).send().await;
        assert_eq!((resume.status, resume.code()), (StatusCode::CONFLICT, "not_uploading"));
        assert_eq!(count(&pool, "select count(*) from videos").await, 1);
    }
}

db_test! {
    async fn abandoned_uploads_are_cleaned_up_after_a_day_and_fresh_ones_are_left_alone(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_creator(&app, "t@x.in").await;
        let stale = video_id(&create(&app, &jar, MIB).await);
        let fresh = video_id(&create(&app, &jar, MIB).await);
        sqlx::query("update videos set created_at = now() - interval '25 hours' where id = $1")
            .bind(stale).execute(&pool).await.unwrap();
        let stale_upload: String = sqlx::query_scalar("select upload_id from videos where id = $1").bind(stale).fetch_one(&pool).await.unwrap();

        assert_eq!(abandon_stale_uploads(&app.state).await.unwrap(), 1);

        let left: Vec<Uuid> = sqlx::query_scalar("select id from videos").fetch_all(&pool).await.unwrap();
        assert_eq!(left, vec![fresh]);
        let listed = app.state.store.list_parts(&format!("uploads/{stale}/source"), &stale_upload).await;
        assert!(matches!(listed, Err(StorageError::NotFound)));
    }
}

db_test! {
    async fn internal_failure_detail_never_reaches_the_status_response(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_creator(&app, "t@x.in").await;
        let id = video_id(&create(&app, &jar, MIB).await);

        sqlx::query("update videos set status = 'failed', error_code = 'ffmpeg exited 1 at /tmp/work/secret' where id = $1")
            .bind(id).execute(&pool).await.unwrap();
        let shown = app.get(&format!("/v1/videos/{id}")).jar(&jar).send().await;
        assert_eq!(shown.body["video"]["status"], "failed");
        assert_eq!(shown.body["video"]["error_code"], "processing_failed");
        assert!(!shown.body.to_string().contains("secret"));
        assert!(shown.body["video"].get("source_key").is_none() && shown.body["video"].get("upload_id").is_none());

        sqlx::query("update videos set error_code = 'unsupported_media' where id = $1").bind(id).execute(&pool).await.unwrap();
        let known = app.get(&format!("/v1/videos/{id}")).jar(&jar).send().await;
        assert_eq!(known.body["video"]["error_code"], "unsupported_media");
    }
}

db_test! {
    async fn a_revoked_session_can_no_longer_start_uploads(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_creator(&app, "t@x.in").await;
        app.post("/v1/auth/logout").jar(&jar).send().await;
        assert_eq!(create(&app, &jar, MIB).await.status, StatusCode::UNAUTHORIZED);
    }
}

/// Uploads a small source, completes it, and pretends a worker published `segments` objects and
/// finished: the video is `ready` and its transcode job is done.
async fn ready_video(app: &App, jar: &Jar, pool: &PgPool, segments: usize) -> Uuid {
    let id = processing_video(app, jar).await;
    for n in 0..segments {
        app.state
            .store
            .put_bytes(&format!("videos/{id}/seg{n}.ts"), vec![0; 8], "video/mp2t")
            .await
            .unwrap();
    }
    sqlx::query("update videos set status = 'ready', ready_at = now() where id = $1")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("insert into video_keys (video_id, wrapped_key) values ($1, $2)")
        .bind(id)
        .bind(vec![7u8; 45])
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("update jobs set completed_at = now() where payload ->> 'video_id' = $1")
        .bind(id.to_string())
        .execute(pool)
        .await
        .unwrap();
    id
}

/// A completed upload: status `processing`, with its transcode job queued.
async fn processing_video(app: &App, jar: &Jar) -> Uuid {
    let created = create(app, jar, MIB).await;
    let id = video_id(&created);
    browser_put(&part_urls(&created.body["upload"])[0].2, bytes(MIB, 5)).await;
    let done = app
        .post(&format!("/v1/videos/{id}/complete"))
        .jar(jar)
        .send()
        .await;
    assert_eq!(done.status, StatusCode::OK, "{:?}", done.body);
    id
}

fn delete_request(app: &App, id: impl std::fmt::Display) -> Req {
    app.req(axum::http::Method::DELETE, &format!("/v1/videos/{id}"))
}

/// (does the original exist, how many published objects exist)
async fn objects(app: &App, id: Uuid) -> (bool, usize) {
    let source = app
        .state
        .store
        .head(&format!("uploads/{id}/source"))
        .await
        .unwrap()
        .is_some();
    (
        source,
        app.state
            .store
            .list_prefix(&format!("videos/{id}/"))
            .await
            .unwrap()
            .len(),
    )
}

db_test! {
    async fn deleting_a_video_stops_serving_it_at_once_and_a_sweep_removes_its_files(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_creator(&app, "t@x.in").await;
        let other = ready_video(&app, &jar, &pool, 3).await;
        let id = ready_video(&app, &jar, &pool, 3).await;
        assert_eq!(objects(&app, id).await, (true, 3));

        let gone = delete_request(&app, id).jar(&jar).send().await;
        assert_eq!(gone.status, StatusCode::NO_CONTENT);
        // Stops being served immediately, though the files stay until the sweep.
        assert_eq!(app.get(&format!("/v1/videos/{id}")).jar(&jar).send().await.status, StatusCode::NOT_FOUND);
        assert_eq!(delete_request(&app, id).jar(&jar).send().await.status, StatusCode::NOT_FOUND);
        assert_eq!(objects(&app, id).await, (true, 3));

        assert_eq!(purge_deleted(&app.state).await.unwrap(), 1);
        assert_eq!(objects(&app, id).await, (false, 0));
        assert_eq!(count(&pool, &format!("select count(*) from videos where id = '{id}'")).await, 0);
        assert_eq!(count(&pool, &format!("select count(*) from video_keys where video_id = '{id}'")).await, 0);
        // The other video, and a second sweep, are unaffected.
        assert_eq!(objects(&app, other).await, (true, 3));
        assert_eq!(purge_deleted(&app.state).await.unwrap(), 0);
        assert_eq!(app.get(&format!("/v1/videos/{other}")).jar(&jar).send().await.status, StatusCode::OK);
    }
}

db_test! {
    async fn only_the_owner_can_delete_a_video(pool: PgPool) {
        let app = app(pool.clone());
        let owner = signup_creator(&app, "owner@x.in").await;
        let rival = signup_creator(&app, "rival@x.in").await;
        let student = signup_verified(&app, "kid@x.in").await;
        let id = ready_video(&app, &owner, &pool, 1).await;

        assert_eq!(delete_request(&app, id).jar(&rival).send().await.status, StatusCode::NOT_FOUND);
        assert_eq!(delete_request(&app, id).jar(&student).send().await.status, StatusCode::FORBIDDEN);
        assert_eq!(delete_request(&app, id).send().await.status, StatusCode::UNAUTHORIZED);
        assert_eq!(delete_request(&app, "not-a-uuid").jar(&owner).send().await.status, StatusCode::NOT_FOUND);
        assert_eq!(app.get(&format!("/v1/videos/{id}")).jar(&owner).send().await.status, StatusCode::OK);
        assert_eq!(purge_deleted(&app.state).await.unwrap(), 0);
        assert_eq!(objects(&app, id).await, (true, 1));
    }
}

db_test! {
    async fn deleting_an_unfinished_upload_cancels_it(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_creator(&app, "t@x.in").await;
        let id = video_id(&create(&app, &jar, MIB).await);
        let gone = delete_request(&app, id).jar(&jar).send().await;
        assert_eq!(gone.status, StatusCode::NO_CONTENT);
        assert_eq!(count(&pool, "select count(*) from videos").await, 0);
    }
}

db_test! {
    async fn a_deleted_video_is_not_purged_while_its_transcode_job_could_still_write_output(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_creator(&app, "t@x.in").await;
        // Processing: the job is live and a worker may be publishing right now.
        let id = processing_video(&app, &jar).await;
        assert_eq!(delete_request(&app, id).jar(&jar).send().await.status, StatusCode::NO_CONTENT);

        assert_eq!(purge_deleted(&app.state).await.unwrap(), 0);
        assert_eq!(count(&pool, "select count(*) from videos").await, 1);
        assert!(objects(&app, id).await.0);

        // The worker publishes (late), then its job ends: now everything can go.
        app.state.store.put_bytes(&format!("videos/{id}/late.ts"), vec![0; 8], "video/mp2t").await.unwrap();
        sqlx::query("update jobs set completed_at = now()").execute(&pool).await.unwrap();
        assert_eq!(purge_deleted(&app.state).await.unwrap(), 1);
        assert_eq!(objects(&app, id).await, (false, 0));
    }
}

db_test! {
    async fn originals_are_deleted_after_the_retention_period_but_the_renditions_stay(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_creator(&app, "t@x.in").await;
        let old = ready_video(&app, &jar, &pool, 2).await;
        let recent = ready_video(&app, &jar, &pool, 2).await;
        let processing = processing_video(&app, &jar).await;
        sqlx::query("update videos set ready_at = now() - interval '8 days' where id = $1").bind(old).execute(&pool).await.unwrap();
        sqlx::query("update videos set ready_at = now() - interval '6 days' where id = $1").bind(recent).execute(&pool).await.unwrap();
        sqlx::query("update videos set updated_at = now() - interval '30 days' where id = $1").bind(processing).execute(&pool).await.unwrap();

        assert_eq!(purge_old_sources(&app.state).await.unwrap(), 1);
        assert_eq!(objects(&app, old).await, (false, 2), "original gone, renditions kept");
        assert_eq!(objects(&app, recent).await, (true, 2));
        assert!(objects(&app, processing).await.0, "a video still being processed keeps its original");
        let shown = app.get(&format!("/v1/videos/{old}")).jar(&jar).send().await;
        assert_eq!(shown.body["video"]["status"], "ready");
        assert_eq!(purge_old_sources(&app.state).await.unwrap(), 0, "once is enough");
    }
}

db_test! {
    async fn failed_videos_lose_their_original_after_the_retention_period_too(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_creator(&app, "t@x.in").await;
        let id = processing_video(&app, &jar).await;
        sqlx::query("update videos set status = 'failed', error_code = 'unsupported_media', updated_at = now() - interval '8 days' where id = $1")
            .bind(id).execute(&pool).await.unwrap();
        assert_eq!(purge_old_sources(&app.state).await.unwrap(), 1);
        assert!(!objects(&app, id).await.0);
    }
}
