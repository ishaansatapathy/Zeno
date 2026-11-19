//! Moving counts from Redis to Postgres.
//!
//! One pass takes each shard's pending counts (an atomic move inside Redis, see
//! [`zeno_core::views::ViewCounter::take`]), writes them in **one transaction per batch**, and
//! only then tells Redis the batch is done. The batch's id goes into `view_flushes` in that same
//! transaction, which is what makes the whole thing safe to repeat:
//!
//! * the process dies after the commit and before `done`: the batch is found again later, its id
//!   is already marked, nothing is added twice;
//! * the process dies before the commit: nothing was written, the batch is found again and
//!   written;
//! * two instances find the same batch at once: the second one's marker insert waits for the
//!   first, then finds it taken.
//!
//! So a view is counted at most once, and, unless Redis itself loses data, at least once.

use std::collections::BTreeMap;
use std::time::Duration;

use uuid::Uuid;
use zeno_core::views::{Batch, SHARDS};

use crate::error::ApiError;
use crate::state::AppState;
use crate::views::repo;

/// Rows per statement. Keeps one statement's arrays and locks bounded however busy a minute was.
const CHUNK: usize = 1000;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Report {
    /// Batches written by this pass.
    pub batches: u32,
    /// Batches found already written (a crash between the commit and the confirmation).
    pub already_applied: u32,
    /// Rows written to `recording_views`: one per recording per day, not one per view.
    pub rows: u64,
    /// Views those rows stand for.
    pub views: u64,
    /// Batches that could not be written; they stay in Redis and are retried.
    pub failed: u32,
}

/// One pass: every shard's pending counts, then any batch an earlier pass left unfinished.
pub async fn run_once(state: &AppState) -> Result<Report, ApiError> {
    let mut report = Report::default();
    for shard in 0..SHARDS {
        match state.views.take(shard).await {
            Ok(Some(batch)) => apply_and_confirm(state, batch, &mut report).await,
            Ok(None) => {}
            Err(error) => {
                // Redis is down: no point trying the other shards, and nothing is lost, the
                // counts are where they were.
                tracing::warn!(%error, "view flush could not reach the counter");
                report.failed += 1;
                return Ok(report);
            }
        }
    }
    match state.views.stale_batches().await {
        Ok(batches) => {
            for batch in batches {
                apply_and_confirm(state, batch, &mut report).await;
            }
        }
        Err(error) => tracing::warn!(%error, "view flush could not list unfinished batches"),
    }
    if let Err(error) = repo::purge_markers(&state.db).await {
        tracing::warn!(%error, "could not purge old view flush markers");
    }
    Ok(report)
}

async fn apply_and_confirm(state: &AppState, batch: Batch, report: &mut Report) {
    let id = batch.id;
    match apply(state, &batch).await {
        Ok(Applied::Written { rows, views }) => {
            report.batches += 1;
            report.rows += rows;
            report.views += views;
        }
        Ok(Applied::AlreadyDone) => report.already_applied += 1,
        Err(error) => {
            // Left in Redis. It turns stale in a couple of minutes and is picked up again.
            tracing::error!(?error, batch = %id, "view batch not written");
            report.failed += 1;
            return;
        }
    }
    if let Err(error) = state.views.done(id).await {
        // Harmless: the marker makes the next sighting of this batch a no-op.
        tracing::warn!(%error, batch = %id, "view batch written but not confirmed");
    }
}

enum Applied {
    Written { rows: u64, views: u64 },
    AlreadyDone,
}

fn to_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

async fn apply(state: &AppState, batch: &Batch) -> Result<Applied, sqlx::Error> {
    let mut tallies: Vec<_> = batch.tallies.iter().collect();
    // One order everywhere: it is also the order the rows are locked in.
    tallies.sort_by_key(|t| (t.recording, t.day));

    let mut tx = state.db.begin().await?;
    if !repo::claim_batch(&mut tx, batch.id).await? {
        tx.rollback().await?;
        return Ok(Applied::AlreadyDone);
    }
    let (mut rows, mut views) = (0u64, 0u64);
    for chunk in tallies.chunks(CHUNK) {
        let mut ids: Vec<Uuid> = chunk.iter().map(|t| t.recording).collect();
        ids.dedup();
        // A recording hard-deleted since its views were counted is simply not there any more.
        let existing = repo::lock_existing(&mut tx, &ids).await?;
        let (mut r, mut d, mut v, mut u) = (vec![], vec![], vec![], vec![]);
        let mut totals: BTreeMap<Uuid, i64> = BTreeMap::new();
        for tally in chunk {
            if existing.binary_search(&tally.recording).is_err() {
                continue;
            }
            r.push(tally.recording);
            d.push(tally.day);
            v.push(to_i64(tally.views));
            u.push(to_i64(tally.viewers));
            let total = totals.entry(tally.recording).or_default();
            *total = total.saturating_add(to_i64(tally.views));
            views += tally.views;
        }
        if r.is_empty() {
            continue;
        }
        rows += r.len() as u64;
        repo::add_days(&mut tx, &r, &d, &v, &u).await?;
        let (ids, sums): (Vec<_>, Vec<_>) = totals.into_iter().unzip();
        repo::add_totals(&mut tx, &ids, &sums).await?;
    }
    tx.commit().await?;
    Ok(Applied::Written { rows, views })
}

/// Runs [`run_once`] on a timer, for as long as the process lives.
pub fn spawn_periodic(state: AppState, every: Duration) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(every);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            match run_once(&state).await {
                Ok(report) if report == Report::default() => {}
                Ok(report) => tracing::info!(?report, "view flush"),
                Err(error) => tracing::error!(?error, "view flush failed"),
            }
        }
    });
}
