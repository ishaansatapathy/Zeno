//! Claims transcode jobs and drives each one to a conclusion: done, retried later, or failed
//! for good. A heartbeat keeps the lease alive while ffmpeg works; if the lease is lost or the
//! worker is shutting down, the job is cancelled and its ffmpeg process killed.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use zeno_core::jobs::{self, Job};

use crate::error::JobError;
use crate::image_cleanup;
use crate::pipeline::{self, Context, REST_KIND};

pub const JOB_KIND: &str = "transcode";
const LEASE: Duration = Duration::from_secs(120);
const HEARTBEAT_EVERY: Duration = Duration::from_secs(30);
const MAX_HEARTBEAT_FAILURES: u32 = 3;
/// How often a worker looks for a job when nobody has told it of one. New jobs are announced (see
/// `jobs::WAKE_CHANNEL`), so this only matters for a job that is due later (a retry) or an announcement
/// that was missed.
pub const IDLE_POLL: Duration = Duration::from_secs(2);
const ERROR_POLL: Duration = Duration::from_secs(5);
/// An encoder crash on a file that probed fine is retried this many times in total.
const MAX_ENCODE_ATTEMPTS: i32 = 3;

/// Which jobs a runner takes. A video's first rendition is what people are waiting for; the rest of its
/// ladder is not, and can take as long as a long video needs. They run in lanes of their own so the second
/// never holds up the first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lane {
    /// First renditions, and housekeeping.
    First,
    /// The remaining renditions of videos that can already be watched.
    Rest,
}

impl Lane {
    fn kinds(self) -> &'static [&'static str] {
        match self {
            Lane::First => &[JOB_KIND],
            Lane::Rest => &[REST_KIND],
        }
    }
}

/// Claims and handles at most one job of any kind, first renditions before the rest. Returns whether there
/// was one. (The worker itself runs lanes; this is the whole queue in one call, for tests.)
pub async fn run_once(ctx: &Context, shutdown: &CancellationToken) -> Result<bool, sqlx::Error> {
    let job = claim(ctx, &[Lane::First, Lane::Rest]).await?;
    let Some(job) = job else {
        return Ok(false);
    };
    dispatch(ctx, job, shutdown).await;
    Ok(true)
}

async fn claim(ctx: &Context, lanes: &[Lane]) -> Result<Option<Job>, sqlx::Error> {
    for lane in lanes {
        if let Some(job) = jobs::claim(&ctx.db, lane.kinds(), LEASE).await? {
            return Ok(Some(job));
        }
        // Housekeeping rides with the first lane, after the work people are waiting for.
        if *lane == Lane::First {
            if let Some(job) = jobs::claim(&ctx.db, &[image_cleanup::JOB_KIND], LEASE).await? {
                return Ok(Some(job));
            }
        }
    }
    Ok(None)
}

async fn dispatch(ctx: &Context, job: Job, shutdown: &CancellationToken) {
    if job.kind == image_cleanup::JOB_KIND {
        image_cleanup::handle(ctx, &job).await;
    } else {
        handle(ctx, job, shutdown).await;
    }
}

/// One runner of one lane, until shutdown. `wake` is poked whenever a job is announced.
pub async fn run_lane(
    ctx: Context,
    shutdown: CancellationToken,
    lane: Lane,
    wake: Arc<Notify>,
    idle_poll: Duration,
) {
    while !shutdown.is_cancelled() {
        // Listen before looking, so an announcement that arrives between the look and the wait is not lost.
        let announced = wake.notified();
        tokio::pin!(announced);
        announced.as_mut().enable();
        let pause = match claim(&ctx, &[lane]).await {
            Ok(Some(job)) => {
                dispatch(&ctx, job, &shutdown).await;
                continue;
            }
            Ok(None) => idle_poll,
            Err(err) => {
                tracing::error!(error = ?err, "could not claim a job");
                ERROR_POLL
            }
        };
        tokio::select! {
            () = &mut announced => {}
            () = tokio::time::sleep(pause) => {}
            () = shutdown.cancelled() => {}
        }
    }
}

/// Pokes `wake` whenever Postgres announces a job. If the connection drops it is made again, and in the
/// meantime the runners' own polling finds the jobs.
pub async fn listen_for_jobs(db: sqlx::PgPool, wake: Arc<Notify>, shutdown: CancellationToken) {
    while !shutdown.is_cancelled() {
        let listener = async {
            let mut listener = sqlx::postgres::PgListener::connect_with(&db).await?;
            listener.listen(jobs::WAKE_CHANNEL).await?;
            loop {
                listener.recv().await?;
                wake.notify_waiters();
            }
            #[allow(unreachable_code)]
            Ok::<(), sqlx::Error>(())
        };
        tokio::select! {
            result = listener => {
                if let Err(err) = result {
                    tracing::warn!(error = ?err, "lost the job announcements; polling until they are back");
                }
            }
            () = shutdown.cancelled() => return,
        }
        tokio::select! {
            () = tokio::time::sleep(ERROR_POLL) => {}
            () = shutdown.cancelled() => return,
        }
    }
}

async fn handle(ctx: &Context, job: Job, shutdown: &CancellationToken) {
    if job.out_of_attempts() {
        tracing::error!(job_id = %job.id, attempts = job.attempts, "job keeps crashing its worker; giving up");
        give_up(
            ctx,
            &job,
            "processing_failed",
            "exceeded its attempts without finishing",
        )
        .await;
        return;
    }

    let cancel = CancellationToken::new();
    let heartbeat = tokio::spawn(heartbeat(ctx.clone(), job.clone(), cancel.clone()));
    let relay = {
        let (shutdown, cancel) = (shutdown.clone(), cancel.clone());
        tokio::spawn(async move {
            shutdown.cancelled().await;
            cancel.cancel();
        })
    };

    let result = if job.kind == REST_KIND {
        pipeline::process_rest(ctx, &job, &cancel).await
    } else {
        pipeline::process(ctx, &job, &cancel).await
    };
    heartbeat.abort();
    relay.abort();
    settle(ctx, &job, result, shutdown.is_cancelled()).await;
}

async fn heartbeat(ctx: Context, job: Job, cancel: CancellationToken) {
    let mut failures = 0;
    loop {
        tokio::time::sleep(HEARTBEAT_EVERY).await;
        match jobs::heartbeat(&ctx.db, &job, LEASE).await {
            Ok(true) => failures = 0,
            Ok(false) => {
                tracing::warn!(job_id = %job.id, "lost the job lease; stopping");
                cancel.cancel();
                return;
            }
            Err(err) => {
                failures += 1;
                tracing::error!(error = ?err, failures, "could not extend the job lease");
                if failures >= MAX_HEARTBEAT_FAILURES {
                    cancel.cancel();
                    return;
                }
            }
        }
    }
}

async fn settle(ctx: &Context, job: &Job, result: Result<(), JobError>, shutting_down: bool) {
    match result {
        Ok(()) => {}
        Err(JobError::Cancelled) => {
            if shutting_down {
                // Hand the job straight back so another worker can start it.
                if let Err(err) =
                    jobs::retry_later(&ctx.db, job, "worker shut down", Duration::ZERO).await
                {
                    tracing::error!(error = ?err, "could not release the job");
                }
            }
        }
        Err(JobError::Permanent { code, detail }) => {
            tracing::warn!(job_id = %job.id, code, %detail, "video rejected");
            give_up(ctx, job, code, &detail).await;
        }
        Err(JobError::Transient(detail)) => {
            if job.attempts >= job.max_attempts {
                tracing::error!(job_id = %job.id, %detail, "giving up after repeated failures");
                give_up(ctx, job, "processing_failed", &detail).await;
            } else {
                retry(ctx, job, &detail).await;
            }
        }
        Err(JobError::Encode(detail)) => {
            if job.attempts >= MAX_ENCODE_ATTEMPTS {
                tracing::error!(job_id = %job.id, %detail, "encoder keeps failing; giving up");
                give_up(ctx, job, "processing_failed", &detail).await;
            } else {
                retry(ctx, job, &detail).await;
            }
        }
    }
}

async fn retry(ctx: &Context, job: &Job, detail: &str) {
    let delay = jobs::backoff(job.attempts);
    tracing::warn!(job_id = %job.id, attempt = job.attempts, ?delay, %detail, "will retry");
    if let Err(err) = jobs::retry_later(&ctx.db, job, detail, delay).await {
        tracing::error!(error = ?err, "could not schedule a retry");
    }
}

/// Marks the video failed and the job finished in one transaction, so the two never disagree.
async fn give_up(ctx: &Context, job: &Job, code: &str, detail: &str) {
    let result = async {
        let mut tx = ctx.db.begin().await?;
        if let Some(video_id) = job.payload.get("video_id").and_then(|v| v.as_str()) {
            sqlx::query(
                "update videos set status = 'failed', error_code = $2, updated_at = now() \
                 where id = $1::uuid and status = 'processing'",
            )
            .bind(video_id)
            .bind(code)
            .execute(&mut *tx)
            .await?;
        }
        jobs::fail(&mut *tx, job, detail).await?;
        tx.commit().await
    }
    .await;
    if let Err(err) = result {
        tracing::error!(error = ?err, job_id = %job.id, "could not record the failure");
    }
}
