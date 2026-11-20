//! Share links under concurrency: things that are fine one request at a time and go wrong when two
//! happen together. Each test names the promise, and makes the race happen on purpose, either by
//! holding one side's transaction open (so the other side meets it at exactly the wrong moment) or
//! by sending many requests at once.

mod common;

use std::time::Duration;

use axum::http::{Method, StatusCode};
use common::media::ready_video;
use common::*;
use futures_util::future::join_all;
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

/// A shared recording of a real, playable video, with one open link.
struct Shared {
    app: App,
    pool: PgPool,
    owner: Jar,
    recording: Uuid,
    link: Uuid,
    code: String,
}

async fn shared(pool: PgPool) -> Shared {
    let app = app(pool.clone());
    let owner = signup_creator(&app, "teacher@x.in").await;
    let video = ready_video(&app, &owner, &pool).await;
    let made = app
        .post("/v1/recordings")
        .jar(&owner)
        .json(json!({"source": "extension", "title": "Week 3", "video_id": video}))
        .send()
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{:?}", made.body);
    let recording = Uuid::parse_str(made.body["recording"]["id"].as_str().unwrap()).unwrap();
    let link = app
        .post(&format!("/v1/recordings/{recording}/links"))
        .jar(&owner)
        .json(json!({}))
        .send()
        .await;
    assert_eq!(link.status, StatusCode::CREATED, "{:?}", link.body);
    Shared {
        link: Uuid::parse_str(link.body["link"]["id"].as_str().unwrap()).unwrap(),
        code: link.body["link"]["code"].as_str().unwrap().to_owned(),
        app,
        pool,
        owner,
        recording,
    }
}

async fn open_sessions(pool: &PgPool, link: Uuid) -> i64 {
    sqlx::query_scalar(
        "select count(*) from playback_sessions where share_link_id = $1 and ended_at is null",
    )
    .bind(link)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Presses play on a link, in a task of its own so the test can do something else meanwhile.
fn start_playing(app: &App, code: &str) -> tokio::task::JoinHandle<Res> {
    let request = app
        .post(&format!("/v1/s/{code}/play"))
        .json(json!({}))
        .send();
    tokio::spawn(request)
}

/// How long a task is given to reach the database and meet whatever is being held there.
const LET_IT_ARRIVE: Duration = Duration::from_millis(400);

// ---- a viewing that starts as the link closes ---------------------------------------------------

// Closing a link (revoking it, making the recording private, deleting it) ends the viewings under
// way. A viewing that is just starting is not "under way" yet when that happens, so unless starting
// and closing exclude each other, it slips in after the sweep and stays open on a closed link.
// Nobody could watch through it (every playlist and key asks again), but it is a session that
// lies about being open, and it is exactly what "revoke ends everything at once" must not leave.
//
// Here the owner's side is held open, uncommitted, while a visitor presses play; then it commits.

db_test! {
    async fn a_viewing_that_starts_as_the_link_is_revoked_does_not_survive(pool: PgPool) {
        let s = shared(pool).await;

        // The owner's revoke, up to but not including its commit.
        let mut tx = s.pool.begin().await.unwrap();
        sqlx::query("update share_links set revoked_at = now() where id = $1")
            .bind(s.link)
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query("update playback_sessions set ended_at = now(), end_reason = 'access_revoked' where share_link_id = $1 and ended_at is null")
            .bind(s.link)
            .execute(&mut *tx)
            .await
            .unwrap();

        // A visitor presses play while the revoke is still in flight. The link still looks live to them.
        let visitor = start_playing(&s.app, &s.code);
        tokio::time::sleep(LET_IT_ARRIVE).await;
        tx.commit().await.unwrap();

        let played = visitor.await.unwrap();
        assert_eq!(played.status, StatusCode::GONE, "the visitor lost the race, and is told so: {:?}", played.body);
        assert_eq!(open_sessions(&s.pool, s.link).await, 0, "no viewing is left open on a revoked link");
    }
}

db_test! {
    async fn a_viewing_that_starts_as_the_recording_goes_private_does_not_survive(pool: PgPool) {
        let s = shared(pool).await;

        let mut tx = s.pool.begin().await.unwrap();
        sqlx::query("update recordings set visibility = 'private', version = version + 1 where id = $1")
            .bind(s.recording)
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query("update playback_sessions set ended_at = now(), end_reason = 'access_revoked' where ended_at is null and share_link_id in (select id from share_links where recording_id = $1)")
            .bind(s.recording)
            .execute(&mut *tx)
            .await
            .unwrap();

        let visitor = start_playing(&s.app, &s.code);
        tokio::time::sleep(LET_IT_ARRIVE).await;
        tx.commit().await.unwrap();

        let played = visitor.await.unwrap();
        assert_eq!(played.status, StatusCode::GONE, "{:?}", played.body);
        assert_eq!(open_sessions(&s.pool, s.link).await, 0);
    }
}

db_test! {
    async fn a_viewing_that_starts_as_the_recording_is_deleted_does_not_survive(pool: PgPool) {
        let s = shared(pool).await;

        let mut tx = s.pool.begin().await.unwrap();
        sqlx::query("update recordings set deleted_at = now(), version = version + 1 where id = $1")
            .bind(s.recording)
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query("update playback_sessions set ended_at = now(), end_reason = 'access_revoked' where ended_at is null and share_link_id in (select id from share_links where recording_id = $1)")
            .bind(s.recording)
            .execute(&mut *tx)
            .await
            .unwrap();

        let visitor = start_playing(&s.app, &s.code);
        tokio::time::sleep(LET_IT_ARRIVE).await;
        tx.commit().await.unwrap();

        let played = visitor.await.unwrap();
        assert_eq!(played.status, StatusCode::GONE, "{:?}", played.body);
        assert_eq!(open_sessions(&s.pool, s.link).await, 0);
    }
}

// ---- the same, with real requests and no help ---------------------------------------------------

db_test! {
    async fn a_flood_of_visitors_while_the_owner_revokes_leaves_nothing_open_and_nothing_failing(pool: PgPool) {
        let s = shared(pool).await;
        let revoke = format!("/v1/recordings/{}/links/{}", s.recording, s.link);

        // Sixty visitors start, and the owner revokes somewhere in the middle of them.
        let mut tasks = Vec::new();
        for n in 0..60 {
            tasks.push(start_playing(&s.app, &s.code));
            if n == 25 {
                tasks.push(tokio::spawn(
                    s.app.req(Method::DELETE, &revoke).jar(&s.owner).send(),
                ));
            }
        }
        let results: Vec<Res> = join_all(tasks).await.into_iter().map(|r| r.unwrap()).collect();

        let mut started = Vec::new();
        for res in &results {
            match res.status {
                // Got in before the revoke.
                StatusCode::CREATED => started.push(res.body["playback"]["playlist_url"].as_str().unwrap().to_owned()),
                // Met the revoke (or the visitor limit): told so, not failed.
                StatusCode::GONE | StatusCode::TOO_MANY_REQUESTS | StatusCode::NO_CONTENT => {}
                other => panic!("neither a start nor a clean refusal: {other} {:?}", res.body),
            }
        }
        assert!(results.iter().any(|r| r.status == StatusCode::NO_CONTENT), "the revoke went through");

        // However it fell out, afterwards the link is closed for everyone who started.
        assert_eq!(open_sessions(&s.pool, s.link).await, 0, "no viewing is left open on a revoked link");
        for url in started {
            let res = s.app.get(&url).send().await;
            assert!(res.status.is_client_error(), "a viewing that started before the revoke cannot fetch a playlist after it: {}", res.status);
        }
    }
}

// ---- the cap on live links ----------------------------------------------------------------------

db_test! {
    async fn a_recording_never_has_more_live_links_than_the_cap_however_many_are_asked_for_at_once(pool: PgPool) {
        let s = shared(pool).await;
        // One link exists already; ask for twenty more at the same moment.
        let tasks: Vec<_> = (0..20)
            .map(|_| {
                tokio::spawn(
                    s.app
                        .post(&format!("/v1/recordings/{}/links", s.recording))
                        .jar(&s.owner)
                        .json(json!({}))
                        .send(),
                )
            })
            .collect();
        let results: Vec<Res> = join_all(tasks).await.into_iter().map(|r| r.unwrap()).collect();

        for res in &results {
            assert!(
                res.status == StatusCode::CREATED || res.status == StatusCode::CONFLICT,
                "{} {:?}",
                res.status,
                res.body
            );
        }
        let live: i64 = sqlx::query_scalar(
            "select count(*) from share_links where recording_id = $1 and revoked_at is null",
        )
        .bind(s.recording)
        .fetch_one(&s.pool)
        .await
        .unwrap();
        assert_eq!(live, 10, "exactly the cap, no more and no fewer");
        assert_eq!(results.iter().filter(|r| r.status == StatusCode::CREATED).count(), 9);
    }
}

// ---- deleting while editing --------------------------------------------------------------------
//
// Two renames from one version, and one client id arriving many times, are already held to exactly
// one winner in tests/recordings.rs.

db_test! {
    async fn renaming_and_deleting_at_once_ends_in_a_clean_answer_for_both(pool: PgPool) {
        let s = shared(pool).await;
        let uri = format!("/v1/recordings/{}", s.recording);
        let mut tasks = Vec::new();
        for n in 0..6 {
            tasks.push(tokio::spawn(
                s.app.req(Method::PATCH, &uri).jar(&s.owner).json(json!({"version": 3, "title": format!("Rename {n}")})).send(),
            ));
            if n == 3 {
                tasks.push(tokio::spawn(s.app.req(Method::DELETE, &uri).jar(&s.owner).send()));
            }
        }
        let results: Vec<Res> = join_all(tasks).await.into_iter().map(|r| r.unwrap()).collect();
        for res in &results {
            assert!(
                [StatusCode::OK, StatusCode::NO_CONTENT, StatusCode::NOT_FOUND, StatusCode::CONFLICT].contains(&res.status),
                "{} {:?}",
                res.status,
                res.body
            );
        }
        assert_eq!(results.iter().filter(|r| r.status == StatusCode::NO_CONTENT).count(), 1, "the recording is deleted exactly once");
        // And it stays deleted, whatever order they ran in.
        assert_eq!(s.app.get(&uri).jar(&s.owner).send().await.status, StatusCode::NOT_FOUND);
    }
}
