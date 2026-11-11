use std::collections::HashMap;

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgExecutor};
use uuid::Uuid;

/// Where a creator stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    /// Everything the book says Zeno owes them (negative: they owe Zeno, after a refund that came
    /// once the money had been paid out; future sales pay it off).
    pub total_paise: i64,
    /// Part of that still inside the refund window of the sale it came from: a refund could still
    /// take it back, so it cannot be paid out yet.
    pub pending_paise: i64,
}

impl Position {
    /// What may be paid out now. Never negative.
    pub fn available_paise(self) -> i64 {
        (self.total_paise - self.pending_paise).max(0)
    }
}

/// A creator's balance, and how much of it is still inside a refund window. The window is the one
/// promised to the buyer when they paid (frozen in the order), counted from the moment of payment
/// by the database's clock.
pub async fn position<'e>(
    db: impl PgExecutor<'e>,
    creator_id: Uuid,
) -> Result<Position, sqlx::Error> {
    let (total_paise, pending_paise): (i64, i64) = sqlx::query_as(
        "select \
            coalesce((select sum(amount_paise) from ledger_entries where creator_id = $1), 0)::bigint, \
            coalesce((select sum(greatest(net, 0)) from ( \
                select sum(e.amount_paise) as net \
                from orders o \
                join ledger_journals j on j.order_id = o.id \
                join ledger_entries e on e.journal_id = j.id and e.creator_id = $1 \
                where o.status = 'paid' and o.paid_at is not null \
                  and coalesce((o.price_snapshot->>'refund_window_days')::int, 7) > 0 \
                  and o.paid_at + make_interval( \
                        days => coalesce((o.price_snapshot->>'refund_window_days')::int, 7)) > now() \
                group by o.id) windowed), 0)::bigint",
    )
    .bind(creator_id)
    .fetch_one(db)
    .await?;
    Ok(Position {
        total_paise,
        pending_paise,
    })
}

/// The creator's entries added up by the kind of journal they belong to.
pub async fn totals_by_kind<'e>(
    db: impl PgExecutor<'e>,
    creator_id: Uuid,
) -> Result<HashMap<String, i64>, sqlx::Error> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "select j.kind, coalesce(sum(e.amount_paise), 0)::bigint \
         from ledger_entries e join ledger_journals j on j.id = e.journal_id \
         where e.creator_id = $1 group by j.kind",
    )
    .bind(creator_id)
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().collect())
}

/// Takes the creator's row lock, so their payout requests are handled one at a time.
pub async fn lock_creator(conn: &mut PgConnection, creator_id: Uuid) -> Result<bool, sqlx::Error> {
    Ok(
        sqlx::query_scalar::<_, Uuid>("select id from users where id = $1 for update")
            .bind(creator_id)
            .fetch_optional(conn)
            .await?
            .is_some(),
    )
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct EntryRow {
    pub id: Uuid,
    pub journal_id: Uuid,
    pub kind: String,
    pub amount_paise: i64,
    pub order_id: Option<Uuid>,
    pub course_title: Option<String>,
    pub memo: Option<String>,
    pub created_at: DateTime<Utc>,
}

const ENTRY_SELECT: &str = "select e.id, e.journal_id, j.kind, e.amount_paise, j.order_id, \
    o.price_snapshot->>'course_title' as course_title, j.memo, e.created_at \
    from ledger_entries e \
    join ledger_journals j on j.id = e.journal_id \
    left join orders o on o.id = j.order_id";

/// The creator's entries, newest first, after `cursor` (the last entry of the previous page).
pub async fn entries<'e>(
    db: impl PgExecutor<'e>,
    creator_id: Uuid,
    cursor: Option<(DateTime<Utc>, Uuid)>,
    limit: i64,
) -> Result<Vec<EntryRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "{ENTRY_SELECT} where e.creator_id = $1 \
           and ($2::timestamptz is null or (e.created_at, e.id) < ($2, $3)) \
         order by e.created_at desc, e.id desc limit $4"
    ))
    .bind(creator_id)
    .bind(cursor.map(|(at, _)| at))
    .bind(cursor.map(|(_, id)| id))
    .bind(limit)
    .fetch_all(db)
    .await
}

/// The creator's entries from `from` (inclusive) to `to` (exclusive), oldest first.
pub async fn entries_between<'e>(
    db: impl PgExecutor<'e>,
    creator_id: Uuid,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<EntryRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "{ENTRY_SELECT} where e.creator_id = $1 and e.created_at >= $2 and e.created_at < $3 \
         order by e.created_at, e.id limit $4"
    ))
    .bind(creator_id)
    .bind(from)
    .bind(to)
    .bind(limit)
    .fetch_all(db)
    .await
}

pub async fn is_creator<'e>(db: impl PgExecutor<'e>, user_id: Uuid) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("select exists (select 1 from users where id = $1 and role = 'creator')")
        .bind(user_id)
        .fetch_one(db)
        .await
}

pub async fn insert_adjustment(
    conn: &mut PgConnection,
    creator_id: Uuid,
    amount_paise: i64,
    reason: &str,
    created_by: Uuid,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar(
        "insert into ledger_adjustments (creator_id, amount_paise, reason, created_by) \
         values ($1, $2, $3, $4) returning id",
    )
    .bind(creator_id)
    .bind(amount_paise)
    .bind(reason)
    .bind(created_by)
    .fetch_one(conn)
    .await
}
