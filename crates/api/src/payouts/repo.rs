use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgExecutor};
use uuid::Uuid;
use zeno_core::model::PayoutStatus;

const COLUMNS: &str = "id, creator_id, amount_paise, status, method, reference, paid_at, \
    cancelled_at, cancel_reason, decided_by, requested_at";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PayoutRow {
    pub id: Uuid,
    pub creator_id: Uuid,
    pub amount_paise: i64,
    pub status: PayoutStatus,
    pub method: Option<String>,
    pub reference: Option<String>,
    pub paid_at: Option<DateTime<Utc>>,
    pub cancelled_at: Option<DateTime<Utc>>,
    pub cancel_reason: Option<String>,
    pub decided_by: Option<Uuid>,
    pub requested_at: DateTime<Utc>,
}

/// `None` if the creator already has a payout waiting.
pub async fn insert(
    conn: &mut PgConnection,
    creator_id: Uuid,
    amount_paise: i64,
) -> Result<Option<PayoutRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "insert into payouts (creator_id, amount_paise) values ($1, $2) \
         on conflict do nothing returning {COLUMNS}"
    ))
    .bind(creator_id)
    .bind(amount_paise)
    .fetch_optional(conn)
    .await
}

pub async fn open_for_creator<'e>(
    db: impl PgExecutor<'e>,
    creator_id: Uuid,
) -> Result<Option<PayoutRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "select {COLUMNS} from payouts where creator_id = $1 and status = 'requested'"
    ))
    .bind(creator_id)
    .fetch_optional(db)
    .await
}

/// The payout, locked until the transaction ends.
pub async fn lock(conn: &mut PgConnection, id: Uuid) -> Result<Option<PayoutRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "select {COLUMNS} from payouts where id = $1 for update"
    ))
    .bind(id)
    .fetch_optional(conn)
    .await
}

pub async fn list_for_creator<'e>(
    db: impl PgExecutor<'e>,
    creator_id: Uuid,
    limit: i64,
) -> Result<Vec<PayoutRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "select {COLUMNS} from payouts where creator_id = $1 \
         order by requested_at desc, id desc limit $2"
    ))
    .bind(creator_id)
    .bind(limit)
    .fetch_all(db)
    .await
}

#[derive(Debug, sqlx::FromRow)]
pub struct AdminPayoutRow {
    #[sqlx(flatten)]
    pub payout: PayoutRow,
    pub creator_name: String,
    pub creator_email: Option<String>,
}

/// Every creator's payouts, oldest request first when looking at the queue.
pub async fn list_all<'e>(
    db: impl PgExecutor<'e>,
    status: Option<PayoutStatus>,
    limit: i64,
) -> Result<Vec<AdminPayoutRow>, sqlx::Error> {
    sqlx::query_as(
        "select p.id, p.creator_id, p.amount_paise, p.status, p.method, p.reference, p.paid_at, \
                p.cancelled_at, p.cancel_reason, p.decided_by, p.requested_at, \
                u.name as creator_name, u.email as creator_email \
         from payouts p join users u on u.id = p.creator_id \
         where ($1::payout_status is null or p.status = $1) \
         order by case when p.status = 'requested' then 0 else 1 end, \
                  case when p.status = 'requested' then p.requested_at end asc, \
                  p.requested_at desc, p.id desc \
         limit $2",
    )
    .bind(status)
    .bind(limit)
    .fetch_all(db)
    .await
}

/// Records that the money has been sent. Only a payout still waiting can be paid.
pub async fn mark_paid(
    conn: &mut PgConnection,
    id: Uuid,
    method: &str,
    reference: &str,
    admin_id: Uuid,
) -> Result<Option<PayoutRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "update payouts set status = 'paid', method = $2, reference = $3, paid_at = now(), \
            decided_by = $4, updated_at = now() \
         where id = $1 and status = 'requested' returning {COLUMNS}"
    ))
    .bind(id)
    .bind(method)
    .bind(reference)
    .bind(admin_id)
    .fetch_optional(conn)
    .await
}

pub async fn mark_cancelled(
    conn: &mut PgConnection,
    id: Uuid,
    reason: Option<&str>,
    decided_by: Uuid,
) -> Result<Option<PayoutRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "update payouts set status = 'cancelled', cancelled_at = now(), cancel_reason = $2, \
            decided_by = $3, updated_at = now() \
         where id = $1 and status = 'requested' returning {COLUMNS}"
    ))
    .bind(id)
    .bind(reason)
    .bind(decided_by)
    .fetch_optional(conn)
    .await
}
