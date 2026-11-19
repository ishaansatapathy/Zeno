//! View counts: counted in Redis, written to Postgres in batches. The promises that matter are
//! that a flood of views is a handful of writes, that no view is counted twice or silently lost
//! to a crash, and that nobody is ever stopped from watching because counting broke.

mod common;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use axum::http::StatusCode;
use common::media::ready_video;
use common::*;
use futures_util::future::join_all;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;
use zeno_api::mail::MemoryMailer;
use zeno_api::views::flush::{run_once, Report};
use zeno_core::views::{ViewCounter, SHARDS, STALE_AFTER_SECS};

macro_rules! db_test {
    ($(#[$meta:meta])* async fn $name:ident($pool:ident: PgPool) $body:block) => {
        #[sqlx::test(migrator = "zeno_core::db::MIGRATOR")]
        $(#[$meta])*
        async fn $name($pool: PgPool) $body
    };
}

async fn recording(app: &App, jar: &Jar, video: Option<Uuid>) -> Uuid {
    let made = app
        .post("/v1/recordings")
        .jar(jar)
        .json(json!({"source": "extension", "title": "Week 3", "video_id": video}))
        .send()
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{:?}", made.body);
    Uuid::parse_str(made.body["recording"]["id"].as_str().unwrap()).unwrap()
}

async fn views_of(app: &App, jar: &Jar, recording: Uuid, query: &str) -> Res {
    app.get(&format!("/v1/recordings/{recording}/views{query}"))
        .jar(jar)
        .send()
        .await
}

async fn total(pool: &PgPool, recording: Uuid) -> i64 {
    sqlx::query_scalar("select view_count from recordings where id = $1")
        .bind(recording)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn day_rows(pool: &PgPool) -> i64 {
    count(pool, "select count(*) from recording_views").await
}

async fn day_sum(pool: &PgPool) -> i64 {
    sqlx::query_scalar("select coalesce(sum(views), 0)::bigint from recording_views")
        .fetch_one(pool)
        .await
        .unwrap()
}

fn now_ms() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap()
}

/// An app counting views with a clock the test moves.
fn app_with_clock(pool: PgPool) -> (App, Arc<AtomicU64>) {
    let clock = Arc::new(AtomicU64::new(now_ms()));
    let handle = clock.clone();
    let mailer = MemoryMailer::default();
    let state = make_state(pool.clone(), false, 0, Arc::new(mailer.clone())).with_views(
        ViewCounter::memory_with_clock(move || handle.load(Ordering::SeqCst)),
    );
    let app = App {
        router: zeno_api::build_app(state.clone()),
        mailer,
        pool,
        state,
    };
    (app, clock)
}

/// The shard a recording's views are pending in, for taking a batch by hand.
fn shard(recording: Uuid) -> usize {
    zeno_core::views::shard_of(recording)
}

db_test! {
    async fn ten_thousand_views_are_a_handful_of_writes(pool: PgPool) {
        let app = app(pool.clone());
        let owner = signup_creator(&app, "teacher@x.in").await;
        let mut recordings = Vec::new();
        for _ in 0..5 {
            recordings.push(recording(&app, &owner, None).await);
        }

        // Ten thousand views from two thousand distinct people per recording, all at once.
        let started = Instant::now();
        let counting: Vec<_> = (0..10_000usize)
            .map(|i| {
                let views = app.state.views.clone();
                let recording = recordings[i % 5];
                async move { views.record(recording, &format!("viewer-{}", i / 5)).await.unwrap() }
            })
            .collect();
        join_all(counting).await;
        let counted_in = started.elapsed();

        let started = Instant::now();
        let report = run_once(&app.state).await.unwrap();
        let flushed_in = started.elapsed();
        eprintln!(
            "10000 views counted in {counted_in:?}; flushed in {flushed_in:?}: {report:?}"
        );

        // The numbers are exact...
        assert_eq!(report.views, 10_000);
        assert_eq!(day_sum(&pool).await, 10_000);
        for recording in &recordings {
            assert_eq!(total(&pool, *recording).await, 2_000);
        }
        // ...and the writes are one row per recording per day, not one per view. (Two days only
        // if the test happens to straddle UTC midnight.)
        let rows = day_rows(&pool).await;
        assert_eq!(report.rows as i64, rows);
        assert!((5..=10).contains(&rows), "{rows} rows for 10000 views");
        assert!(report.batches as usize <= SHARDS);
        assert_eq!(report.failed, 0);
        // Unique viewers are exact in memory (an estimate in Redis).
        let viewers: i64 = sqlx::query_scalar("select max(viewers) from recording_views")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(viewers, 2_000);

        // Nothing is left to write.
        assert_eq!(run_once(&app.state).await.unwrap(), Report::default());
        assert_eq!(day_sum(&pool).await, 10_000);
    }
}

db_test! {
    async fn the_same_viewer_again_soon_is_not_another_view(pool: PgPool) {
        let (app, clock) = app_with_clock(pool.clone());
        let owner = signup_creator(&app, "teacher@x.in").await;
        let rec = recording(&app, &owner, None).await;
        let views = app.state.views.clone();
        for _ in 0..50 {
            views.record(rec, "same-person").await.unwrap();
        }
        // Half an hour on, they are back: that is a new view.
        clock.fetch_add((zeno_core::views::REPEAT_WINDOW_SECS + 1) * 1000, Ordering::SeqCst);
        views.record(rec, "same-person").await.unwrap();
        views.record(rec, "someone-else").await.unwrap();
        let report = run_once(&app.state).await.unwrap();
        assert_eq!(report.views, 3);
        assert_eq!(total(&pool, rec).await, 3);
    }
}

db_test! {
    async fn pressing_play_on_a_share_link_counts_one_view_per_viewer(pool: PgPool) {
        let app = app(pool.clone());
        let owner = signup_creator(&app, "teacher@x.in").await;
        let video = ready_video(&app, &owner, &pool).await;
        let rec = recording(&app, &owner, Some(video)).await;
        let made = app
            .post(&format!("/v1/recordings/{rec}/links"))
            .jar(&owner)
            .json(json!({}))
            .send()
            .await;
        let code = made.body["link"]["code"].as_str().unwrap().to_owned();

        // Three browsers, one of which presses play twice.
        for agent in ["Firefox/130", "Chrome/131", "Safari/18", "Chrome/131"] {
            let res = app
                .post(&format!("/v1/s/{code}/play"))
                .header("user-agent", agent)
                .json(json!({}))
                .send()
                .await;
            assert_eq!(res.status, StatusCode::CREATED, "{:?}", res.body);
        }
        app.settle().await;

        // Nothing has been written yet: that is the point.
        assert_eq!(day_rows(&pool).await, 0);
        assert_eq!(views_of(&app, &owner, rec, "").await.body["views"]["total"], 0);

        let report = run_once(&app.state).await.unwrap();
        assert_eq!(report.views, 3);

        let res = views_of(&app, &owner, rec, "?days=7").await;
        assert_eq!(res.status, StatusCode::OK, "{:?}", res.body);
        assert_eq!(res.body["views"]["total"], 3);
        let days = res.body["views"]["days"].as_array().unwrap();
        assert_eq!(days.len(), 7);
        let today = days.last().unwrap();
        assert_eq!(today["views"], 3);
        assert_eq!(today["viewers"], 3);
        assert!(days[..6].iter().all(|d| d["views"] == 0 && d["viewers"] == 0));

        // The library shows the same number, and a view is not an edit.
        let shown = app.get(&format!("/v1/recordings/{rec}")).jar(&owner).send().await;
        assert_eq!(shown.body["recording"]["view_count"], 3);
        let version_before: i32 = sqlx::query_scalar("select version from recordings where id = $1")
            .bind(rec)
            .fetch_one(&pool)
            .await
            .unwrap();
        app.state.views.record(rec, "late-viewer").await.unwrap();
        run_once(&app.state).await.unwrap();
        let version_after: i32 = sqlx::query_scalar("select version from recordings where id = $1")
            .bind(rec)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(version_before, version_after);
        assert_eq!(total(&pool, rec).await, 4);
    }
}

db_test! {
    async fn only_the_owner_reads_view_counts_and_the_window_is_bounded(pool: PgPool) {
        let app = app(pool);
        let owner = signup_creator(&app, "teacher@x.in").await;
        let other = signup_creator(&app, "other@x.in").await;
        let rec = recording(&app, &owner, None).await;

        assert_eq!(views_of(&app, &owner, rec, "").await.status, StatusCode::OK);
        let stranger = views_of(&app, &other, rec, "").await;
        assert_eq!(stranger.status, StatusCode::NOT_FOUND);
        assert_eq!(
            app.get(&format!("/v1/recordings/{rec}/views")).send().await.status,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            views_of(&app, &owner, Uuid::new_v4(), "").await.status,
            StatusCode::NOT_FOUND
        );
        let not_an_id = app.get("/v1/recordings/nope/views").jar(&owner).send().await;
        assert_eq!(not_an_id.status, StatusCode::NOT_FOUND);

        let default = views_of(&app, &owner, rec, "").await;
        assert_eq!(default.body["views"]["days"].as_array().unwrap().len(), 30);
        for bad in ["?days=0", "?days=91", "?days=-1", "?days=many", "?days=7&x=1"] {
            let res = views_of(&app, &owner, rec, bad).await;
            assert!(
                res.status == StatusCode::UNPROCESSABLE_ENTITY || res.status == StatusCode::BAD_REQUEST,
                "{bad}: {} {:?}",
                res.status,
                res.body
            );
        }
        assert_eq!(views_of(&app, &owner, rec, "?days=90").await.status, StatusCode::OK);
        assert_eq!(views_of(&app, &owner, rec, "?days=1").await.status, StatusCode::OK);
    }
}

db_test! {
    async fn a_batch_that_was_written_but_never_confirmed_is_not_counted_twice(pool: PgPool) {
        let (app, clock) = app_with_clock(pool.clone());
        let owner = signup_creator(&app, "teacher@x.in").await;
        let rec = recording(&app, &owner, None).await;
        for i in 0..40 {
            app.state.views.record(rec, &format!("v{i}")).await.unwrap();
        }

        // The flusher takes the batch and writes it, then dies before telling Redis.
        let batch = app.state.views.take(shard(rec)).await.unwrap().unwrap();
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("insert into view_flushes (batch_id) values ($1)")
            .bind(batch.id)
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query("update recordings set view_count = view_count + 40 where id = $1")
            .bind(rec)
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query("insert into recording_views (recording_id, day, views, viewers) values ($1, $2, 40, 40)")
            .bind(rec)
            .bind(batch.tallies[0].day)
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();

        // Too fresh to be someone else's: left alone.
        assert_eq!(run_once(&app.state).await.unwrap(), Report::default());

        // Two minutes on it is found again, recognised, and only confirmed.
        clock.fetch_add((STALE_AFTER_SECS + 1) * 1000, Ordering::SeqCst);
        let report = run_once(&app.state).await.unwrap();
        assert_eq!(report.already_applied, 1);
        assert_eq!(report.batches, 0);
        assert_eq!(total(&pool, rec).await, 40);
        assert_eq!(day_sum(&pool).await, 40);

        // And it is gone for good.
        assert_eq!(run_once(&app.state).await.unwrap(), Report::default());
    }
}

db_test! {
    async fn a_batch_taken_but_never_written_is_written_once_later(pool: PgPool) {
        let (app, clock) = app_with_clock(pool.clone());
        let owner = signup_creator(&app, "teacher@x.in").await;
        let rec = recording(&app, &owner, None).await;
        for i in 0..25 {
            app.state.views.record(rec, &format!("v{i}")).await.unwrap();
        }
        // The flusher takes the batch and dies before the commit: nothing is in Postgres.
        app.state.views.take(shard(rec)).await.unwrap().unwrap();
        assert_eq!(total(&pool, rec).await, 0);

        clock.fetch_add((STALE_AFTER_SECS + 1) * 1000, Ordering::SeqCst);
        let report = run_once(&app.state).await.unwrap();
        assert_eq!((report.batches, report.views), (1, 25));
        assert_eq!(total(&pool, rec).await, 25);
        assert_eq!(run_once(&app.state).await.unwrap(), Report::default());
        assert_eq!(total(&pool, rec).await, 25);
    }
}

db_test! {
    async fn two_instances_finding_the_same_stale_batch_write_it_once(pool: PgPool) {
        let (app, clock) = app_with_clock(pool.clone());
        let owner = signup_creator(&app, "teacher@x.in").await;
        let rec = recording(&app, &owner, None).await;
        for i in 0..60 {
            app.state.views.record(rec, &format!("v{i}")).await.unwrap();
        }
        app.state.views.take(shard(rec)).await.unwrap().unwrap();
        clock.fetch_add((STALE_AFTER_SECS + 1) * 1000, Ordering::SeqCst);

        let (a, b) = tokio::join!(run_once(&app.state), run_once(&app.state));
        let (a, b) = (a.unwrap(), b.unwrap());
        assert_eq!(a.batches + b.batches, 1, "{a:?} {b:?}");
        assert_eq!(a.views + b.views, 60);
        assert_eq!(total(&pool, rec).await, 60);
        assert_eq!(day_sum(&pool).await, 60);
    }
}

db_test! {
    async fn views_of_a_recording_that_is_gone_do_not_poison_the_batch(pool: PgPool) {
        let app = app(pool.clone());
        let owner = signup_creator(&app, "teacher@x.in").await;
        let rec = recording(&app, &owner, None).await;
        let ghost = Uuid::new_v4();
        for i in 0..5 {
            app.state.views.record(ghost, &format!("g{i}")).await.unwrap();
            app.state.views.record(rec, &format!("v{i}")).await.unwrap();
        }
        let report = run_once(&app.state).await.unwrap();
        assert_eq!(report.failed, 0, "{report:?}");
        assert_eq!(report.views, 5, "only the recording that exists is written");
        assert_eq!(total(&pool, rec).await, 5);
        assert_eq!(day_rows(&pool).await, 1);
        // Confirmed, not retried forever.
        assert_eq!(run_once(&app.state).await.unwrap(), Report::default());
    }
}

db_test! {
    async fn deleting_a_recording_deletes_its_view_rows(pool: PgPool) {
        let app = app(pool.clone());
        let owner = signup_creator(&app, "teacher@x.in").await;
        let rec = recording(&app, &owner, None).await;
        app.state.views.record(rec, "v").await.unwrap();
        run_once(&app.state).await.unwrap();
        assert_eq!(day_rows(&pool).await, 1);
        sqlx::query("delete from recordings where id = $1")
            .bind(rec)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(day_rows(&pool).await, 0);
    }
}

db_test! {
    async fn a_counter_that_is_down_never_stops_anyone_watching(pool: PgPool) {
        let mailer = MemoryMailer::default();
        let state = make_state(pool.clone(), false, 0, Arc::new(mailer.clone()))
            .with_views(ViewCounter::failing());
        let app = App {
            router: zeno_api::build_app(state.clone()),
            mailer,
            pool: pool.clone(),
            state,
        };
        let owner = signup_creator(&app, "teacher@x.in").await;
        let video = ready_video(&app, &owner, &pool).await;
        let rec = recording(&app, &owner, Some(video)).await;
        let made = app
            .post(&format!("/v1/recordings/{rec}/links"))
            .jar(&owner)
            .json(json!({}))
            .send()
            .await;
        let code = made.body["link"]["code"].as_str().unwrap().to_owned();

        let res = app.post(&format!("/v1/s/{code}/play")).json(json!({})).send().await;
        assert_eq!(res.status, StatusCode::CREATED, "{:?}", res.body);
        app.settle().await;

        // The flush notices and reports, without panicking or writing anything.
        let report = run_once(&app.state).await.unwrap();
        assert_eq!(report.failed, 1);
        assert_eq!(day_rows(&pool).await, 0);
        // The owner's page still works.
        assert_eq!(views_of(&app, &owner, rec, "").await.body["views"]["total"], 0);
    }
}

db_test! {
    /// The same flood against a real Redis, when there is one (CI sets TEST_REDIS_URL). The
    /// counts are exact there too; unique viewers are the sketch's estimate, so within 2%.
    async fn the_flood_against_real_redis(pool: PgPool) {
        let Ok(url) = std::env::var("TEST_REDIS_URL") else {
            eprintln!("skipped: TEST_REDIS_URL not set");
            return;
        };
        let client = redis::Client::open(url).unwrap();
        let conn = redis::aio::ConnectionManager::new(client).await.unwrap();
        let mailer = MemoryMailer::default();
        let state = make_state(pool.clone(), false, 0, Arc::new(mailer.clone()))
            .with_views(ViewCounter::redis(conn));
        let app = App {
            router: zeno_api::build_app(state.clone()),
            mailer,
            pool: pool.clone(),
            state,
        };
        let owner = signup_creator(&app, "teacher@x.in").await;
        let mut recordings = Vec::new();
        for _ in 0..5 {
            recordings.push(recording(&app, &owner, None).await);
        }

        let started = Instant::now();
        // 64 tasks, as 64 busy connections would be.
        let tasks: Vec<_> = (0..64usize)
            .map(|worker| {
                let views = app.state.views.clone();
                let recordings = recordings.clone();
                tokio::spawn(async move {
                    for i in (worker..10_000).step_by(64) {
                        views
                            .record(recordings[i % 5], &format!("viewer-{}", i / 5))
                            .await
                            .unwrap();
                    }
                })
            })
            .collect();
        join_all(tasks).await.into_iter().for_each(|t| t.unwrap());
        let counted_in = started.elapsed();

        let started = Instant::now();
        let report = run_once(&app.state).await.unwrap();
        let flushed_in = started.elapsed();
        eprintln!("redis: 10000 views counted in {counted_in:?}; flushed in {flushed_in:?}: {report:?}");

        // Other runs' leftovers name recordings that are not in this database and are skipped,
        // so only ours are counted.
        for recording in &recordings {
            assert_eq!(total(&pool, *recording).await, 2_000);
        }
        assert_eq!(day_sum(&pool).await, 10_000);
        let rows = day_rows(&pool).await;
        assert!((5..=10).contains(&rows), "{rows} rows for 10000 views");
        let viewers: Vec<i64> = sqlx::query_scalar("select viewers from recording_views")
            .fetch_all(&pool)
            .await
            .unwrap();
        for estimate in viewers {
            assert!(
                (1_960..=2_040).contains(&estimate),
                "unique viewers estimated {estimate}, true value 2000"
            );
        }
    }
}
