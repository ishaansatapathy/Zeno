//! Durable email delivery. Messages are written to `email_outbox` first and sent afterwards,
//! so a provider outage or a crash delays an email instead of losing it.
//!
//! Delivery is at-least-once: a row is leased while an instance works on it, and picked up
//! again if that instance dies. Bodies carry live one-time links, so they are blanked as soon
//! as a row is sent or abandoned, and a row stops retrying when its link would have expired.

use std::time::Duration;

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::mail::OutgoingEmail;
use crate::state::AppState;

const BATCH: i64 = 20;
/// How long a claimed row is hidden from other workers while a send is in flight.
const LEASE_SECS: f64 = 120.0;
const SEND_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_ATTEMPTS: i32 = 8;
const RETRY_POLL: Duration = Duration::from_secs(15);
const MAX_ERROR_CHARS: usize = 500;

#[derive(sqlx::FromRow)]
struct Due {
    id: Uuid,
    to_address: String,
    subject: String,
    body: String,
    attempts: i32,
    expires_at: DateTime<Utc>,
}

/// Queues an email and kicks off a first delivery attempt in the background. `ttl` is how
/// long the message is worth delivering (the lifetime of the link inside it).
pub async fn enqueue(
    state: &AppState,
    email: OutgoingEmail,
    ttl: ChronoDuration,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "insert into email_outbox (to_address, subject, body, expires_at) values ($1, $2, $3, $4)",
    )
    .bind(email.to)
    .bind(email.subject)
    .bind(email.text)
    .bind(Utc::now() + ttl)
    .execute(&state.db)
    .await?;

    let state = state.clone();
    state
        .background
        .clone()
        .spawn(async move { dispatch_due(&state).await });
    Ok(())
}

/// Sends everything that is due. Safe to run from many instances at once.
pub async fn dispatch_due(state: &AppState) {
    let due = match claim(&state.db).await {
        Ok(due) => due,
        Err(err) => {
            tracing::error!(error = ?err, "could not read the email outbox");
            return;
        }
    };
    for row in due {
        deliver(state, row).await;
    }
}

/// Retries failed deliveries and recovers rows whose worker died.
pub fn spawn_retry_loop(state: AppState) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(RETRY_POLL);
        loop {
            ticker.tick().await;
            dispatch_due(&state).await;
        }
    });
}

async fn claim(db: &PgPool) -> Result<Vec<Due>, sqlx::Error> {
    sqlx::query_as(
        "update email_outbox set next_attempt_at = now() + make_interval(secs => $1) \
         where id in ( \
             select id from email_outbox \
             where sent_at is null and failed_at is null and next_attempt_at <= now() \
             order by next_attempt_at limit $2 for update skip locked) \
         returning id, to_address, subject, body, attempts, expires_at",
    )
    .bind(LEASE_SECS)
    .bind(BATCH)
    .fetch_all(db)
    .await
}

async fn deliver(state: &AppState, row: Due) {
    if row.expires_at <= Utc::now() {
        give_up(
            &state.db,
            row.id,
            "expired before it could be delivered",
            row.attempts,
        )
        .await;
        return;
    }
    let email = OutgoingEmail {
        to: row.to_address.clone(),
        subject: row.subject.clone(),
        text: row.body.clone(),
    };
    let outcome = match tokio::time::timeout(SEND_TIMEOUT, state.mailer.send(email)).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(err)) => Err(err.to_string()),
        Err(_) => Err("delivery timed out".to_owned()),
    };
    match outcome {
        Ok(()) => mark_sent(&state.db, row.id).await,
        Err(reason) => record_failure(&state.db, &row, &reason).await,
    }
}

async fn mark_sent(db: &PgPool, id: Uuid) {
    let result = sqlx::query(
        "update email_outbox set sent_at = now(), body = '', attempts = attempts + 1, last_error = null where id = $1",
    )
    .bind(id)
    .execute(db)
    .await;
    if let Err(err) = result {
        tracing::error!(error = ?err, "could not mark email as sent");
    }
}

async fn record_failure(db: &PgPool, row: &Due, reason: &str) {
    let attempts = row.attempts + 1;
    if attempts >= MAX_ATTEMPTS {
        tracing::error!(attempts, "email delivery failed permanently");
        give_up(db, row.id, reason, attempts).await;
        return;
    }
    tracing::warn!(attempts, "email delivery failed; will retry");
    let backoff_secs = (30u64 << attempts.min(6)).min(900) as f64;
    let result = sqlx::query(
        "update email_outbox set attempts = $2, last_error = $3, \
         next_attempt_at = now() + make_interval(secs => $4) where id = $1",
    )
    .bind(row.id)
    .bind(attempts)
    .bind(truncate(reason))
    .bind(backoff_secs)
    .execute(db)
    .await;
    if let Err(err) = result {
        tracing::error!(error = ?err, "could not schedule email retry");
    }
}

async fn give_up(db: &PgPool, id: Uuid, reason: &str, attempts: i32) {
    let result = sqlx::query(
        "update email_outbox set failed_at = now(), body = '', last_error = $2, attempts = $3 where id = $1",
    )
    .bind(id)
    .bind(truncate(reason))
    .bind(attempts)
    .execute(db)
    .await;
    if let Err(err) = result {
        tracing::error!(error = ?err, "could not abandon email");
    }
}

fn truncate(reason: &str) -> String {
    reason.chars().take(MAX_ERROR_CHARS).collect()
}
