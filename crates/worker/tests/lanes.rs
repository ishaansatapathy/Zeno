//! How jobs reach a worker: announced at once, and in lanes so one video's remaining renditions never hold
//! up the next video's first play.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;
use sqlx::PgPool;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use zeno_core::jobs;
use zeno_worker::runner::{self, Lane};

macro_rules! db_test {
    ($(#[$meta:meta])* async fn $name:ident($pool:ident: PgPool) $body:block) => {
        $(#[$meta])*
        #[sqlx::test(migrator = "zeno_core::db::MIGRATOR")]
        async fn $name($pool: PgPool) $body
    };
}

/// A day: the poll can never be what finds a job in these tests.
const NEVER: Duration = Duration::from_secs(86_400);

async fn status_of(pool: &PgPool, id: Uuid) -> String {
    sqlx::query_scalar("select status::text from videos where id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

db_test! {
    async fn a_new_job_is_picked_up_at_once_not_at_the_next_poll(pool: PgPool) {
        let ctx = context(pool.clone()).await;
        let shutdown = CancellationToken::new();
        let wake = Arc::new(Notify::new());
        tokio::spawn(runner::listen_for_jobs(pool.clone(), wake.clone(), shutdown.clone()));
        let lane = tokio::spawn(runner::run_lane(ctx.clone(), shutdown.clone(), Lane::First, wake, NEVER));
        // Let the listener subscribe and the runner find nothing and start waiting.
        tokio::time::sleep(Duration::from_millis(500)).await;

        // A file that is not a video is refused in a moment, which is all that is needed to see it was picked up.
        let queued = Instant::now();
        let id = seed(&ctx, b"not a video at all".to_vec()).await;
        while status_of(&pool, id).await == "processing" && queued.elapsed() < Duration::from_secs(20) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let waited = queued.elapsed();
        shutdown.cancel();
        let _ = lane.await;
        assert_eq!(status_of(&pool, id).await, "failed", "it was picked up and handled (and refused, being no video)");
        assert!(waited < Duration::from_secs(5), "picked up by the announcement, not by a poll that never comes: {waited:?}");
    }
}

db_test! {
    async fn the_first_lane_leaves_the_rest_of_a_ladder_to_the_other(pool: PgPool) {
        let ctx = context(pool.clone()).await;
        let scratch = scratch_dir();
        let id = seed(&ctx, make_video(scratch.path(), "in.mp4", &Spec::default())).await;
        // The first job is done by hand; its rest job is now queued.
        assert!(runner::run_once(&ctx, &CancellationToken::new()).await.unwrap());
        let rest: Uuid = sqlx::query_scalar("select id from jobs where kind = 'transcode_rest' and completed_at is null")
            .fetch_one(&pool)
            .await
            .unwrap();

        // A lane for first renditions, running, does not touch it.
        let shutdown = CancellationToken::new();
        let wake = Arc::new(Notify::new());
        let lane = tokio::spawn(runner::run_lane(ctx.clone(), shutdown.clone(), Lane::First, wake.clone(), Duration::from_millis(50)));
        tokio::time::sleep(Duration::from_millis(600)).await;
        let attempts: i32 = sqlx::query_scalar("select attempts from jobs where id = $1").bind(rest).fetch_one(&pool).await.unwrap();
        assert_eq!(attempts, 0, "nothing in the first lane takes the rest of a ladder");
        shutdown.cancel();
        let _ = lane.await;

        // The other lane does.
        let shutdown = CancellationToken::new();
        let lane = tokio::spawn(runner::run_lane(ctx.clone(), shutdown.clone(), Lane::Rest, wake, Duration::from_millis(50)));
        let started = Instant::now();
        while video_state(&pool, id).await.renditions.as_array().map_or(0, Vec::len) < 6 && started.elapsed() < Duration::from_secs(60) {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        shutdown.cancel();
        let _ = lane.await;
        assert_eq!(video_state(&pool, id).await.renditions.as_array().unwrap().len(), 6);
    }
}

db_test! {
    async fn a_job_announced_while_nobody_listens_is_still_found_by_the_poll(pool: PgPool) {
        let ctx = context(pool.clone()).await;
        let shutdown = CancellationToken::new();
        let wake = Arc::new(Notify::new());
        // No listener at all: the announcement goes nowhere.
        let lane = tokio::spawn(runner::run_lane(ctx.clone(), shutdown.clone(), Lane::First, wake, Duration::from_millis(100)));
        let id = seed(&ctx, b"not a video at all".to_vec()).await;
        let started = Instant::now();
        while status_of(&pool, id).await != "failed" {
            assert!(started.elapsed() < Duration::from_secs(20), "the poll never found it");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        shutdown.cancel();
        let _ = lane.await;
    }
}

db_test! {
    async fn enqueueing_announces_a_job_only_when_one_was_made(pool: PgPool) {
        let mut listener = sqlx::postgres::PgListener::connect_with(&pool).await.unwrap();
        listener.listen(jobs::WAKE_CHANNEL).await.unwrap();
        let payload = serde_json::json!({ "video_id": Uuid::new_v4() });

        assert!(jobs::enqueue(&pool, "transcode", &payload).await.unwrap().is_some());
        let heard = tokio::time::timeout(Duration::from_secs(5), listener.recv()).await.expect("announced").unwrap();
        assert_eq!((heard.channel(), heard.payload()), (jobs::WAKE_CHANNEL, "transcode"));

        // The same video again, of the other kind: refused, and so not announced.
        assert!(jobs::enqueue(&pool, "transcode_rest", &payload).await.unwrap().is_none());
        assert!(tokio::time::timeout(Duration::from_millis(400), listener.recv()).await.is_err(), "no job, no announcement");
    }
}

db_test! {
    async fn an_announcement_waits_for_the_transaction_that_made_the_job(pool: PgPool) {
        let mut listener = sqlx::postgres::PgListener::connect_with(&pool).await.unwrap();
        listener.listen(jobs::WAKE_CHANNEL).await.unwrap();
        let mut tx = pool.begin().await.unwrap();
        jobs::enqueue(&mut *tx, "transcode", &serde_json::json!({ "video_id": Uuid::new_v4() })).await.unwrap().unwrap();
        assert!(tokio::time::timeout(Duration::from_millis(400), listener.recv()).await.is_err(), "not before it can be found");
        tx.commit().await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), listener.recv()).await.expect("announced at commit").unwrap();
    }
}
