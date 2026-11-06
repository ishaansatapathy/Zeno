//! Postgres-backed job queue. Workers claim a job under a lease and must keep extending it
//! while they work; if a worker dies the lease lapses and the job is claimed again.
//!
//! Every update after a claim is fenced by the attempt number the worker was given, so a
//! worker whose lease expired and whose job was handed to someone else can no longer change
//! it. This is the whole interface the services rely on (`enqueue`, `claim`, `heartbeat`,
//! `complete`, `fail`, `retry_later`), so it can be replaced by NATS or SQS later.

use std::time::Duration;

use serde_json::Value;
use sqlx::PgExecutor;
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Job {
    pub id: Uuid,
    pub kind: String,
    pub payload: Value,
    /// Number of the attempt this claim represents (1 for the first). Used as a fence.
    pub attempts: i32,
    pub max_attempts: i32,
}

impl Job {
    pub fn out_of_attempts(&self) -> bool {
        self.attempts > self.max_attempts
    }
}

/// Returns `None` if an equivalent live job already exists (see the unique index on
/// transcode jobs), which makes retrying the request that enqueues it harmless.
pub async fn enqueue<'e>(
    db: impl PgExecutor<'e>,
    kind: &str,
    payload: &Value,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        "insert into jobs (kind, payload) values ($1, $2) on conflict do nothing returning id",
    )
    .bind(kind)
    .bind(payload)
    .fetch_optional(db)
    .await
}

pub async fn claim<'e>(
    db: impl PgExecutor<'e>,
    kinds: &[&str],
    lease: Duration,
) -> Result<Option<Job>, sqlx::Error> {
    let kinds: Vec<String> = kinds.iter().map(|k| (*k).to_owned()).collect();
    sqlx::query_as(
        "update jobs set attempts = attempts + 1, locked_until = now() + make_interval(secs => $2) \
         where id = ( \
             select id from jobs \
             where kind = any($1) and completed_at is null and failed_at is null \
               and run_at <= now() and (locked_until is null or locked_until < now()) \
             order by run_at limit 1 for update skip locked) \
         returning id, kind, payload, attempts, max_attempts",
    )
    .bind(kinds)
    .bind(lease.as_secs_f64())
    .fetch_optional(db)
    .await
}

/// Extends the lease. `false` means this attempt no longer owns the job and must stop.
pub async fn heartbeat<'e>(
    db: impl PgExecutor<'e>,
    job: &Job,
    lease: Duration,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "update jobs set locked_until = now() + make_interval(secs => $3) \
         where id = $1 and attempts = $2 and completed_at is null and failed_at is null",
    )
    .bind(job.id)
    .bind(job.attempts)
    .bind(lease.as_secs_f64())
    .execute(db)
    .await?;
    Ok(result.rows_affected() == 1)
}

pub async fn complete<'e>(db: impl PgExecutor<'e>, job: &Job) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "update jobs set completed_at = now(), locked_until = null, last_error = null \
         where id = $1 and attempts = $2 and completed_at is null and failed_at is null",
    )
    .bind(job.id)
    .bind(job.attempts)
    .execute(db)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Gives up for good.
pub async fn fail<'e>(
    db: impl PgExecutor<'e>,
    job: &Job,
    error: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "update jobs set failed_at = now(), locked_until = null, last_error = $3 \
         where id = $1 and attempts = $2 and completed_at is null and failed_at is null",
    )
    .bind(job.id)
    .bind(job.attempts)
    .bind(error)
    .execute(db)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Releases the job to be tried again after `delay`.
pub async fn retry_later<'e>(
    db: impl PgExecutor<'e>,
    job: &Job,
    error: &str,
    delay: Duration,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "update jobs set locked_until = null, last_error = $3, run_at = now() + make_interval(secs => $4) \
         where id = $1 and attempts = $2 and completed_at is null and failed_at is null",
    )
    .bind(job.id)
    .bind(job.attempts)
    .bind(error)
    .bind(delay.as_secs_f64())
    .execute(db)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Exponential backoff with a cap, in seconds: 30, 60, 120, ... up to 15 minutes.
pub fn backoff(attempt: i32) -> Duration {
    let exponent = attempt.clamp(1, 6) as u32 - 1;
    Duration::from_secs((30u64 << exponent).min(900))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_then_caps() {
        let secs: Vec<u64> = (1..=8).map(|a| backoff(a).as_secs()).collect();
        assert_eq!(secs, [30, 60, 120, 240, 480, 900, 900, 900]);
        assert_eq!(backoff(0).as_secs(), 30);
    }
}
