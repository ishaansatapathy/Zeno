use chrono::NaiveDate;
use sqlx::{PgConnection, PgExecutor};
use uuid::Uuid;

/// Records that a batch is being applied, in the caller's transaction. `false`: it already was,
/// by this instance earlier or by another one, and its counts must not be added again.
pub async fn claim_batch(db: &mut PgConnection, batch: Uuid) -> Result<bool, sqlx::Error> {
    let result =
        sqlx::query("insert into view_flushes (batch_id) values ($1) on conflict do nothing")
            .bind(batch)
            .execute(db)
            .await?;
    Ok(result.rows_affected() == 1)
}

/// Locks the recordings that still exist, in id order, and returns their ids. Taking the locks
/// in one order is what keeps two flushers working on overlapping recordings from deadlocking;
/// `no key update` is the weakest lock that still excludes another flusher, and it does not
/// block anyone reading or inserting a row that points at the recording.
pub async fn lock_existing(
    db: &mut PgConnection,
    recordings: &[Uuid],
) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar("select id from recordings where id = any($1) order by id for no key update")
        .bind(recordings)
        .fetch_all(db)
        .await
}

/// Adds a chunk of daily counts: new days are inserted, known days grow. The unique count of a
/// day is an estimate that only ever rises as the day goes on, so the larger one wins.
pub async fn add_days(
    db: &mut PgConnection,
    recordings: &[Uuid],
    days: &[NaiveDate],
    views: &[i64],
    viewers: &[i64],
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "insert into recording_views (recording_id, day, views, viewers) \
         select * from unnest($1::uuid[], $2::date[], $3::bigint[], $4::bigint[]) \
         on conflict (recording_id, day) do update set \
             views = recording_views.views + excluded.views, \
             viewers = greatest(recording_views.viewers, excluded.viewers)",
    )
    .bind(recordings)
    .bind(days)
    .bind(views)
    .bind(viewers)
    .execute(db)
    .await?;
    Ok(())
}

/// Adds to the all-time totals on the recordings: one update per recording however many days
/// it has. Deliberately leaves `version` and `updated_at` alone: a view is not an edit.
pub async fn add_totals(
    db: &mut PgConnection,
    recordings: &[Uuid],
    views: &[i64],
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "update recordings r set view_count = r.view_count + s.views \
         from unnest($1::uuid[], $2::bigint[]) as s(id, views) where r.id = s.id",
    )
    .bind(recordings)
    .bind(views)
    .execute(db)
    .await?;
    Ok(())
}

/// Forgets batch markers old enough that the batch cannot come back (Redis drops an unfinished
/// batch after seven days).
pub async fn purge_markers<'e>(db: impl PgExecutor<'e>) -> Result<u64, sqlx::Error> {
    let result =
        sqlx::query("delete from view_flushes where applied_at < now() - interval '14 days'")
            .execute(db)
            .await?;
    Ok(result.rows_affected())
}

#[derive(Debug, sqlx::FromRow)]
pub struct DayRow {
    pub day: NaiveDate,
    pub views: i64,
    pub viewers: i64,
}

/// The days that have counts, from `since` on. Days nobody watched have no row.
pub async fn days_since<'e>(
    db: impl PgExecutor<'e>,
    recording: Uuid,
    since: NaiveDate,
) -> Result<Vec<DayRow>, sqlx::Error> {
    sqlx::query_as(
        "select day, views, viewers from recording_views \
         where recording_id = $1 and day >= $2 order by day",
    )
    .bind(recording)
    .bind(since)
    .fetch_all(db)
    .await
}
