//! Recordings: a person's library. Ownership, retry-safe creation, optimistic concurrency, the
//! video underneath, deletion that follows the video, and keyset pagination.

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

async fn make(app: &App, jar: &Jar, body: Value) -> Res {
    app.post("/v1/recordings").jar(jar).json(body).send().await
}

async fn show(app: &App, jar: &Jar, id: Uuid) -> Res {
    app.get(&format!("/v1/recordings/{id}"))
        .jar(jar)
        .send()
        .await
}

async fn patch(app: &App, jar: &Jar, id: Uuid, body: Value) -> Res {
    app.req(Method::PATCH, &format!("/v1/recordings/{id}"))
        .jar(jar)
        .json(body)
        .send()
        .await
}

async fn remove(app: &App, jar: &Jar, id: Uuid) -> Res {
    app.req(Method::DELETE, &format!("/v1/recordings/{id}"))
        .jar(jar)
        .send()
        .await
}

async fn attach(app: &App, jar: &Jar, id: Uuid, video: Uuid) -> Res {
    app.req(Method::PUT, &format!("/v1/recordings/{id}/video"))
        .jar(jar)
        .json(json!({ "video_id": video }))
        .send()
        .await
}

fn rid(res: &Res) -> Uuid {
    Uuid::parse_str(res.body["recording"]["id"].as_str().unwrap()).unwrap()
}

async fn user_id(pool: &PgPool, email: &str) -> Uuid {
    sqlx::query_scalar("select id from users where email = $1")
        .bind(email)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// A video row in the given state, owned by `owner`. No storage involved.
async fn video(pool: &PgPool, owner: Uuid, status: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "insert into videos (id, owner_id, title, original_filename, content_type, size_bytes, \
                part_size, source_key, status, duration_seconds) \
         values ($1,$2,'Clip','clip.mp4','video/mp4',1,5242880,$3,$4::video_status,
                 case when $4 = 'ready' then 125 end)",
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

async fn video_deleted(pool: &PgPool, id: Uuid) -> bool {
    sqlx::query_scalar("select deleted_at is not null from videos where id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn recordings_with_id(pool: &PgPool, id: Uuid) -> i64 {
    sqlx::query_scalar("select count(*) from recordings where id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

db_test! {
    async fn a_creator_makes_reads_renames_and_deletes_a_recording(pool: PgPool) {
        let app = app(pool);
        let jar = signup_verified_as(&app, "teacher@x.in", "creator").await;

        let made = make(&app, &jar, json!({"source": "Extension"})).await;
        assert_eq!(made.status, StatusCode::CREATED, "{:?}", made.body);
        let rec = &made.body["recording"];
        assert_eq!(rec["title"], "Untitled recording");
        assert_eq!(rec["source"], "extension");
        assert_eq!(rec["visibility"], "private");
        assert_eq!(rec["status"], "capturing");
        assert_eq!(rec["version"], 1);
        assert!(rec["video_id"].is_null());
        let id = rid(&made);

        let read = show(&app, &jar, id).await;
        assert_eq!((read.status, &read.body["recording"]["id"]), (StatusCode::OK, &json!(id)));

        let renamed = patch(&app, &jar, id, json!({"version": 1, "title": "  Week 3: Limits  "})).await;
        assert_eq!(renamed.status, StatusCode::OK, "{:?}", renamed.body);
        assert_eq!(renamed.body["recording"]["title"], "Week 3: Limits");
        assert_eq!(renamed.body["recording"]["version"], 2);

        // The same change over If-Match works too.
        let by_header = app
            .req(Method::PATCH, &format!("/v1/recordings/{id}"))
            .jar(&jar)
            .header("if-match", "\"2\"")
            .json(json!({"visibility": "link"}))
            .send()
            .await;
        assert_eq!(by_header.status, StatusCode::OK, "{:?}", by_header.body);
        assert_eq!(by_header.body["recording"]["visibility"], "link");

        assert_eq!(remove(&app, &jar, id).await.status, StatusCode::NO_CONTENT);
        assert_eq!(show(&app, &jar, id).await.status, StatusCode::NOT_FOUND);
        assert_eq!(remove(&app, &jar, id).await.status, StatusCode::NOT_FOUND);
        let list = app.get("/v1/recordings").jar(&jar).send().await;
        assert!(list.body["recordings"].as_array().unwrap().is_empty());
    }
}

db_test! {
    async fn creating_with_the_same_id_again_returns_the_first_and_makes_one(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_verified_as(&app, "teacher@x.in", "creator").await;
        let other = signup_verified_as(&app, "other@x.in", "creator").await;
        let id = Uuid::new_v4();
        let body = json!({"id": id, "source": "studio", "title": "Live class"});

        let first = make(&app, &jar, body.clone()).await;
        assert_eq!((first.status, rid(&first)), (StatusCode::CREATED, id));
        let again = make(&app, &jar, body.clone()).await;
        assert_eq!((again.status, rid(&again)), (StatusCode::OK, id));
        assert_eq!(again.body, first.body);
        assert_eq!(recordings_with_id(&pool, id).await, 1);

        // What differs is a conflict, not a silent overwrite; so is someone else's id.
        let other_source = make(&app, &jar, json!({"id": id, "source": "stream"})).await;
        assert_eq!((other_source.status, other_source.code()), (StatusCode::CONFLICT, "id_conflict"));
        let theirs = make(&app, &other, body.clone()).await;
        assert_eq!((theirs.status, theirs.code()), (StatusCode::CONFLICT, "id_conflict"));
        assert_eq!(recordings_with_id(&pool, id).await, 1);

        // A deleted recording's id is not handed out again.
        remove(&app, &jar, id).await;
        let reuse = make(&app, &jar, body).await;
        assert_eq!((reuse.status, reuse.code()), (StatusCode::CONFLICT, "id_conflict"));
    }
}

db_test! {
    async fn parallel_retries_of_one_create_make_exactly_one_recording(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_verified_as(&app, "teacher@x.in", "creator").await;
        let id = Uuid::new_v4();
        let attempts = (0..10).map(|_| make(&app, &jar, json!({"id": id, "source": "extension"})));
        let results = join_all(attempts).await;
        assert_eq!(results.iter().filter(|res| res.status == StatusCode::CREATED).count(), 1);
        assert_eq!(results.iter().filter(|res| res.status == StatusCode::OK).count(), 9);
        assert!(results.iter().all(|res| rid(res) == id));
        assert_eq!(recordings_with_id(&pool, id).await, 1);
    }
}

db_test! {
    async fn recordings_belong_to_their_owner_and_nobody_else_can_touch_them(pool: PgPool) {
        let app = app(pool.clone());
        let owner = signup_verified_as(&app, "owner@x.in", "creator").await;
        let other = signup_verified_as(&app, "other@x.in", "creator").await;
        let student = signup_verified_as(&app, "student@x.in", "student").await;
        let anonymous = Jar::default();
        let theirs = user_id(&pool, "other@x.in").await;

        let made = make(&app, &owner, json!({"source": "upload"})).await;
        let id = rid(&made);
        // Even marked public, it is still only the owner's: nothing reads visibility yet.
        patch(&app, &owner, id, json!({"version": 1, "visibility": "public"})).await;

        for (who, name) in [(&other, "another creator")] {
            assert_eq!(show(&app, who, id).await.status, StatusCode::NOT_FOUND, "{name} read");
            assert_eq!(patch(&app, who, id, json!({"version": 2, "title": "Mine now"})).await.status, StatusCode::NOT_FOUND, "{name} patch");
            assert_eq!(remove(&app, who, id).await.status, StatusCode::NOT_FOUND, "{name} delete");
            let their_video = video(&pool, theirs, "ready").await;
            assert_eq!(attach(&app, who, id, their_video).await.status, StatusCode::NOT_FOUND, "{name} attach");
        }
        assert_eq!(show(&app, &student, id).await.status, StatusCode::FORBIDDEN);
        assert_eq!(make(&app, &student, json!({"source": "upload"})).await.status, StatusCode::FORBIDDEN);
        assert_eq!(app.get("/v1/recordings").jar(&student).send().await.status, StatusCode::FORBIDDEN);
        assert_eq!(show(&app, &anonymous, id).await.status, StatusCode::UNAUTHORIZED);

        let theirs_list = app.get("/v1/recordings").jar(&other).send().await;
        assert!(theirs_list.body["recordings"].as_array().unwrap().is_empty());
        let still = show(&app, &owner, id).await;
        assert_eq!(still.body["recording"]["title"], "Untitled recording");
        assert_eq!(still.body["recording"]["visibility"], "public");
    }
}

db_test! {
    async fn concurrent_edits_from_the_same_version_let_exactly_one_win(pool: PgPool) {
        let app = app(pool);
        let jar = signup_verified_as(&app, "teacher@x.in", "creator").await;
        let id = rid(&make(&app, &jar, json!({"source": "upload"})).await);
        let attempts = ["First", "Second", "Third"].map(|t| patch(&app, &jar, id, json!({"version": 1, "title": t})));
        let results = join_all(attempts).await;
        assert_eq!(results.iter().filter(|r| r.status == StatusCode::OK).count(), 1);
        assert_eq!(results.iter().filter(|r| r.code() == "version_conflict").count(), 2);
        let missing = patch(&app, &jar, id, json!({"title": "No version"})).await;
        assert_eq!((missing.status, missing.code()), (StatusCode::PRECONDITION_REQUIRED, "version_required"));
        assert!(missing.body["error"]["message"].as_str().unwrap().contains("recording"));
        let nothing = patch(&app, &jar, id, json!({"version": 2})).await;
        assert!(nothing.status.is_client_error(), "{:?}", nothing.body);
    }
}

db_test! {
    async fn bad_input_is_refused_with_every_problem_named(pool: PgPool) {
        let app = app(pool);
        let jar = signup_verified_as(&app, "teacher@x.in", "creator").await;
        for body in [
            json!({"source": "telepathy"}),
            json!({"source": "upload", "title": "   "}),
            json!({"source": "upload", "title": "x".repeat(121)}),
            json!({"source": "upload", "title": "tab\there"}),
            json!({"source": "upload", "id": Uuid::nil()}),
            json!({"source": "upload", "surprise": true}),
            json!({"title": "no source"}),
        ] {
            let res = make(&app, &jar, body.clone()).await;
            assert!(res.status.is_client_error(), "{body}: {:?}", res.body);
        }
        let both = make(&app, &jar, json!({"source": "nope", "title": ""})).await;
        let fields: Vec<&str> = both.body["error"]["fields"]
            .as_array()
            .map(|f| f.iter().filter_map(|x| x["field"].as_str()).collect())
            .unwrap_or_default();
        assert!(fields.contains(&"source") && fields.contains(&"title"), "{:?}", both.body);
        assert_eq!(show(&app, &jar, Uuid::new_v4()).await.status, StatusCode::NOT_FOUND);
        let junk = app.get("/v1/recordings/not-a-uuid").jar(&jar).send().await;
        assert_eq!(junk.status, StatusCode::NOT_FOUND);
    }
}

db_test! {
    async fn the_status_follows_the_video_underneath(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_verified_as(&app, "teacher@x.in", "creator").await;
        let me = user_id(&pool, "teacher@x.in").await;
        let id = rid(&make(&app, &jar, json!({"source": "extension"})).await);
        let clip = video(&pool, me, "processing").await;

        let attached = attach(&app, &jar, id, clip).await;
        assert_eq!(attached.status, StatusCode::OK, "{:?}", attached.body);
        assert_eq!(attached.body["recording"]["status"], "processing");
        assert_eq!(attached.body["recording"]["video_id"], json!(clip));
        assert!(attached.body["recording"]["duration_seconds"].is_null());

        sqlx::query("update videos set status = 'ready', duration_seconds = 125 where id = $1")
            .bind(clip)
            .execute(&pool)
            .await
            .unwrap();
        let ready = show(&app, &jar, id).await;
        assert_eq!(ready.body["recording"]["status"], "ready");
        assert_eq!(ready.body["recording"]["duration_seconds"], 125);

        sqlx::query("update videos set status = 'failed' where id = $1").bind(clip).execute(&pool).await.unwrap();
        assert_eq!(show(&app, &jar, id).await.body["recording"]["status"], "failed");

        // An outright removed video (an aborted upload) sends the recording back to capturing.
        sqlx::query("delete from videos where id = $1").bind(clip).execute(&pool).await.unwrap();
        let back = show(&app, &jar, id).await;
        assert_eq!(back.body["recording"]["status"], "capturing");
        assert!(back.body["recording"]["video_id"].is_null());
    }
}

db_test! {
    async fn a_video_can_be_attached_once_and_to_one_recording(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_verified_as(&app, "teacher@x.in", "creator").await;
        let other = signup_verified_as(&app, "other@x.in", "creator").await;
        let me = user_id(&pool, "teacher@x.in").await;
        let theirs = user_id(&pool, "other@x.in").await;
        let first = rid(&make(&app, &jar, json!({"source": "upload"})).await);
        let second = rid(&make(&app, &jar, json!({"source": "upload"})).await);
        let clip = video(&pool, me, "ready").await;
        let spare = video(&pool, me, "ready").await;

        assert_eq!(attach(&app, &jar, first, clip).await.status, StatusCode::OK);
        // Repeating it is fine; replacing it is not.
        assert_eq!(attach(&app, &jar, first, clip).await.status, StatusCode::OK);
        let swap = attach(&app, &jar, first, spare).await;
        assert_eq!((swap.status, swap.code()), (StatusCode::CONFLICT, "video_already_attached"));
        // One video, one recording.
        let twice = attach(&app, &jar, second, clip).await;
        assert_eq!((twice.status, twice.code()), (StatusCode::CONFLICT, "video_in_use"));
        // Someone else's video looks like no video at all.
        let foreign = video(&pool, theirs, "ready").await;
        let stolen = attach(&app, &jar, second, foreign).await;
        assert_eq!((stolen.status, stolen.code()), (StatusCode::NOT_FOUND, "video_not_found"));
        let nobody = attach(&app, &other, second, spare).await;
        assert_eq!(nobody.status, StatusCode::NOT_FOUND);

        // The same rules hold when the video comes with the create.
        let with = make(&app, &jar, json!({"source": "upload", "video_id": spare})).await;
        assert_eq!(with.status, StatusCode::CREATED, "{:?}", with.body);
        assert_eq!(with.body["recording"]["video_id"], json!(spare));
        let dup = make(&app, &jar, json!({"source": "upload", "video_id": spare})).await;
        assert_eq!((dup.status, dup.code()), (StatusCode::CONFLICT, "video_in_use"));
        let missing = make(&app, &jar, json!({"source": "upload", "video_id": Uuid::new_v4()})).await;
        assert_eq!((missing.status, missing.code()), (StatusCode::NOT_FOUND, "video_not_found"));
        let count: i64 = sqlx::query_scalar("select count(*) from recordings where owner_id = $1")
            .bind(me)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 3, "the refused creates left nothing behind");
    }
}

db_test! {
    async fn deleting_a_recording_deletes_its_video_unless_a_course_uses_it(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_verified_as(&app, "teacher@x.in", "creator").await;
        let me = user_id(&pool, "teacher@x.in").await;

        let solo = video(&pool, me, "ready").await;
        let alone = rid(&make(&app, &jar, json!({"source": "upload", "video_id": solo})).await);
        assert_eq!(remove(&app, &jar, alone).await.status, StatusCode::NO_CONTENT);
        assert!(video_deleted(&pool, solo).await, "an unused video goes with its recording");

        let taught = video(&pool, me, "ready").await;
        let shared = rid(&make(&app, &jar, json!({"source": "upload", "video_id": taught})).await);
        let course = Uuid::new_v4();
        sqlx::query("insert into courses (id, creator_id, title, slug, price_paise) values ($1,$2,'Course','course-x',0)")
            .bind(course).bind(me).execute(&pool).await.unwrap();
        sqlx::query("insert into lessons (course_id, position, video_id) values ($1,1,$2)")
            .bind(course).bind(taught).execute(&pool).await.unwrap();
        assert_eq!(remove(&app, &jar, shared).await.status, StatusCode::NO_CONTENT);
        assert!(!video_deleted(&pool, taught).await, "a lesson still plays it");
    }
}

db_test! {
    async fn deleting_a_video_takes_its_recording_with_it(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_verified_as(&app, "teacher@x.in", "creator").await;
        let me = user_id(&pool, "teacher@x.in").await;
        let clip = video(&pool, me, "ready").await;
        let id = rid(&make(&app, &jar, json!({"source": "upload", "video_id": clip})).await);

        let gone = app.req(Method::DELETE, &format!("/v1/videos/{clip}")).jar(&jar).send().await;
        assert_eq!(gone.status, StatusCode::NO_CONTENT, "{:?}", gone.body);
        assert_eq!(show(&app, &jar, id).await.status, StatusCode::NOT_FOUND);
        assert!(app.get("/v1/recordings").jar(&jar).send().await.body["recordings"].as_array().unwrap().is_empty());
    }
}

db_test! {
    async fn lists_are_newest_first_filterable_and_stable_under_ties(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_verified_as(&app, "teacher@x.in", "creator").await;
        let me = user_id(&pool, "teacher@x.in").await;

        // Five rows made in the same instant: only the id can order them.
        let stamp = chrono::Utc::now();
        let mut ids: Vec<Uuid> = (0..5).map(|_| Uuid::new_v4()).collect();
        for id in &ids {
            sqlx::query("insert into recordings (id, owner_id, source, title, created_at) values ($1,$2,'studio','Tie',$3)")
                .bind(id).bind(me).bind(stamp).execute(&pool).await.unwrap();
        }
        let recent = rid(&make(&app, &jar, json!({"source": "extension", "title": "Newest"})).await);

        let mut seen = vec![];
        let mut cursor: Option<String> = None;
        let mut pages = 0;
        loop {
            let url = match &cursor {
                Some(c) => format!("/v1/recordings?limit=2&cursor={c}"),
                None => "/v1/recordings?limit=2".to_owned(),
            };
            let page = app.get(&url).jar(&jar).send().await;
            assert_eq!(page.status, StatusCode::OK, "{:?}", page.body);
            for r in page.body["recordings"].as_array().unwrap() {
                seen.push(Uuid::parse_str(r["id"].as_str().unwrap()).unwrap());
            }
            pages += 1;
            match page.body["next_cursor"].as_str() {
                Some(next) => cursor = Some(next.to_owned()),
                None => break,
            }
        }
        assert_eq!(pages, 3);
        assert_eq!(seen.len(), 6, "every row exactly once");
        assert_eq!(seen[0], recent, "newest first");
        ids.sort_unstable();
        ids.reverse();
        assert_eq!(&seen[1..], &ids[..], "ties break by id, newest id first");

        let streams = app.get("/v1/recordings?source=EXTENSION").jar(&jar).send().await;
        assert_eq!(streams.body["recordings"].as_array().unwrap().len(), 1);
        assert!(app.get("/v1/recordings?source=nope").jar(&jar).send().await.status.is_client_error());
        assert_eq!(app.get("/v1/recordings?cursor=garbage").jar(&jar).send().await.code(), "invalid_cursor");
        // The page size is clamped, not trusted.
        let zero = app.get("/v1/recordings?limit=0").jar(&jar).send().await;
        assert_eq!(zero.body["recordings"].as_array().unwrap().len(), 1);
        let huge = app.get("/v1/recordings?limit=100000").jar(&jar).send().await;
        assert_eq!(huge.body["recordings"].as_array().unwrap().len(), 6);
    }
}

db_test! {
    async fn a_flood_of_writes_from_one_account_is_slowed(pool: PgPool) {
        let app = app(pool);
        let jar = signup_verified_as(&app, "teacher@x.in", "creator").await;
        let mut limited = 0;
        for _ in 0..70 {
            let res = make(&app, &jar, json!({"source": "upload"})).await;
            if res.status == StatusCode::TOO_MANY_REQUESTS {
                limited += 1;
            } else {
                assert_eq!(res.status, StatusCode::CREATED, "{:?}", res.body);
            }
        }
        assert!(limited >= 10, "the 61st to 70th create should be refused, got {limited} refusals");
        // Reading is not what is limited.
        assert_eq!(app.get("/v1/recordings").jar(&jar).send().await.status, StatusCode::OK);
    }
}
