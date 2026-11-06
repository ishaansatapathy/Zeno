use std::time::Duration;

use serde_json::json;
use sqlx::PgPool;
use zeno_core::jobs;

macro_rules! db_test {
    ($(#[$meta:meta])* async fn $name:ident($pool:ident: PgPool) $body:block) => {
        $(#[$meta])*
        #[sqlx::test(migrator = "zeno_core::db::MIGRATOR")]
        async fn $name($pool: PgPool) $body
    };
}

const LEASE: Duration = Duration::from_secs(60);

async fn expire_lease(pool: &PgPool) {
    sqlx::query(
        "update jobs set locked_until = now() - interval '1 second' where locked_until is not null",
    )
    .execute(pool)
    .await
    .unwrap();
}

db_test! {
    async fn a_job_is_claimed_once_and_only_by_its_kind(pool: PgPool) {
        jobs::enqueue(&pool, "transcode", &json!({"video_id": "a"})).await.unwrap().unwrap();

        assert!(jobs::claim(&pool, &["email"], LEASE).await.unwrap().is_none());
        let job = jobs::claim(&pool, &["transcode"], LEASE).await.unwrap().unwrap();
        assert_eq!((job.kind.as_str(), job.attempts), ("transcode", 1));
        assert_eq!(job.payload["video_id"], "a");
        assert!(jobs::claim(&pool, &["transcode"], LEASE).await.unwrap().is_none(), "leased");
    }
}

db_test! {
    async fn racing_workers_never_get_the_same_job(pool: PgPool) {
        for i in 0..5 {
            jobs::enqueue(&pool, "transcode", &json!({"video_id": i.to_string()})).await.unwrap();
        }
        let workers: Vec<_> = (0..12)
            .map(|_| {
                let pool = pool.clone();
                tokio::spawn(async move { jobs::claim(&pool, &["transcode"], LEASE).await.unwrap() })
            })
            .collect();
        let mut ids = vec![];
        for worker in workers {
            if let Some(job) = worker.await.unwrap() {
                ids.push(job.id);
            }
        }
        assert_eq!(ids.len(), 5);
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 5, "no job was handed out twice");
    }
}

db_test! {
    async fn a_second_live_transcode_job_for_one_video_is_refused_but_a_new_one_is_fine_after_it_finishes(pool: PgPool) {
        let payload = json!({"video_id": "v1"});
        assert!(jobs::enqueue(&pool, "transcode", &payload).await.unwrap().is_some());
        assert!(jobs::enqueue(&pool, "transcode", &payload).await.unwrap().is_none());

        let job = jobs::claim(&pool, &["transcode"], LEASE).await.unwrap().unwrap();
        assert!(jobs::complete(&pool, &job).await.unwrap());
        assert!(jobs::enqueue(&pool, "transcode", &payload).await.unwrap().is_some());
    }
}

db_test! {
    async fn a_crashed_workers_job_is_reclaimed_after_the_lease_and_the_old_worker_is_fenced_off(pool: PgPool) {
        jobs::enqueue(&pool, "transcode", &json!({"video_id": "v"})).await.unwrap();
        let first = jobs::claim(&pool, &["transcode"], LEASE).await.unwrap().unwrap();

        expire_lease(&pool).await;
        let second = jobs::claim(&pool, &["transcode"], LEASE).await.unwrap().unwrap();
        assert_eq!((second.id, second.attempts), (first.id, 2));

        // The first worker wakes up and tries to report: every call is refused.
        assert!(!jobs::heartbeat(&pool, &first, LEASE).await.unwrap());
        assert!(!jobs::complete(&pool, &first).await.unwrap());
        assert!(!jobs::fail(&pool, &first, "late").await.unwrap());
        assert!(!jobs::retry_later(&pool, &first, "late", Duration::ZERO).await.unwrap());

        assert!(jobs::heartbeat(&pool, &second, LEASE).await.unwrap());
        assert!(jobs::complete(&pool, &second).await.unwrap());
    }
}

db_test! {
    async fn heartbeats_keep_a_long_job_from_being_stolen(pool: PgPool) {
        jobs::enqueue(&pool, "transcode", &json!({"video_id": "v"})).await.unwrap();
        let job = jobs::claim(&pool, &["transcode"], Duration::from_secs(1)).await.unwrap().unwrap();
        assert!(jobs::heartbeat(&pool, &job, LEASE).await.unwrap());
        tokio::time::sleep(Duration::from_millis(1300)).await;
        assert!(jobs::claim(&pool, &["transcode"], LEASE).await.unwrap().is_none());
    }
}

db_test! {
    async fn retry_later_hides_the_job_until_its_time_and_failure_is_final(pool: PgPool) {
        jobs::enqueue(&pool, "transcode", &json!({"video_id": "v"})).await.unwrap();
        let job = jobs::claim(&pool, &["transcode"], LEASE).await.unwrap().unwrap();
        assert!(jobs::retry_later(&pool, &job, "storage hiccup", Duration::from_secs(300)).await.unwrap());
        assert!(jobs::claim(&pool, &["transcode"], LEASE).await.unwrap().is_none());

        sqlx::query("update jobs set run_at = now()").execute(&pool).await.unwrap();
        let again = jobs::claim(&pool, &["transcode"], LEASE).await.unwrap().unwrap();
        assert_eq!(again.attempts, 2);
        assert!(jobs::fail(&pool, &again, "unsupported media").await.unwrap());
        assert!(jobs::claim(&pool, &["transcode"], LEASE).await.unwrap().is_none());
        let error: String = sqlx::query_scalar("select last_error from jobs").fetch_one(&pool).await.unwrap();
        assert_eq!(error, "unsupported media");
    }
}

db_test! {
    async fn a_job_that_keeps_crashing_reports_that_it_ran_out_of_attempts(pool: PgPool) {
        jobs::enqueue(&pool, "transcode", &json!({"video_id": "v"})).await.unwrap();
        sqlx::query("update jobs set max_attempts = 2").execute(&pool).await.unwrap();
        let mut last = None;
        for _ in 0..3 {
            last = jobs::claim(&pool, &["transcode"], LEASE).await.unwrap();
            expire_lease(&pool).await;
        }
        let job = last.unwrap();
        assert_eq!(job.attempts, 3);
        assert!(job.out_of_attempts());
    }
}
