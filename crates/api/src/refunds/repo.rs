use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgConnection, PgExecutor};
use uuid::Uuid;
use zeno_core::model::{OrderStatus, RefundStatus};

const COLUMNS: &str = "id, order_id, payment_id, amount_paise, status, origin, requested_by, \
    decided_by, reason, decision_note, provider_refund_id, attempts, requested_at, decided_at, \
    completed_at";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RefundRow {
    pub id: Uuid,
    pub order_id: Uuid,
    pub payment_id: Uuid,
    pub amount_paise: i64,
    pub status: RefundStatus,
    pub origin: String,
    pub requested_by: Option<Uuid>,
    pub decided_by: Option<Uuid>,
    pub reason: Option<String>,
    pub decision_note: Option<String>,
    pub provider_refund_id: Option<String>,
    pub attempts: i32,
    pub requested_at: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

pub async fn latest_for_order<'e>(
    db: impl PgExecutor<'e>,
    order_id: Uuid,
) -> Result<Option<RefundRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "select {COLUMNS} from refunds where order_id = $1 \
         order by requested_at desc, id desc limit 1"
    ))
    .bind(order_id)
    .fetch_optional(db)
    .await
}

pub async fn by_id<'e>(
    db: impl PgExecutor<'e>,
    id: Uuid,
) -> Result<Option<RefundRow>, sqlx::Error> {
    sqlx::query_as(&format!("select {COLUMNS} from refunds where id = $1"))
        .bind(id)
        .fetch_optional(db)
        .await
}

/// The refund in flight for the order (a request, or one with the provider), locked.
pub async fn active_for_order(
    conn: &mut PgConnection,
    order_id: Uuid,
) -> Result<Option<RefundRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "select {COLUMNS} from refunds \
         where order_id = $1 and status in ('requested','processing') for update"
    ))
    .bind(order_id)
    .fetch_optional(conn)
    .await
}

// ---- the order a refund is about -------------------------------------------------------------

/// What deciding about a refund needs to know about its order, read under the order's lock.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct OrderContext {
    pub id: Uuid,
    pub user_id: Uuid,
    pub course_id: Uuid,
    pub status: OrderStatus,
    pub price_snapshot: Value,
    pub paid_at: Option<DateTime<Utc>>,
    pub creator_id: Uuid,
    pub course_title: String,
    /// The database's clock, so windows are judged by one clock however many servers run.
    pub db_now: DateTime<Utc>,
}

const CONTEXT_SELECT: &str = "select o.id, o.user_id, o.course_id, o.status, o.price_snapshot, \
    o.paid_at, c.creator_id, c.title as course_title, now() as db_now \
    from orders o join courses c on c.id = o.course_id where o.id = $1";

/// Reads the order; no lock.
pub async fn order_context<'e>(
    db: impl PgExecutor<'e>,
    order_id: Uuid,
) -> Result<Option<OrderContext>, sqlx::Error> {
    sqlx::query_as(CONTEXT_SELECT)
        .bind(order_id)
        .fetch_optional(db)
        .await
}

/// Reads the order and locks its row, so a refund decision and a payment event for the same order
/// take turns.
pub async fn lock_order_context(
    conn: &mut PgConnection,
    order_id: Uuid,
) -> Result<Option<OrderContext>, sqlx::Error> {
    sqlx::query_as(&format!("{CONTEXT_SELECT} for update of o"))
        .bind(order_id)
        .fetch_optional(conn)
        .await
}

#[derive(Debug, sqlx::FromRow)]
pub struct ConfirmedPayment {
    pub id: Uuid,
    pub provider_ref: String,
    pub amount_paise: i64,
    pub refunded_paise: i64,
}

/// The payment that paid the order, and how much of it has already been returned.
pub async fn confirmed_payment(
    conn: &mut PgConnection,
    order_id: Uuid,
) -> Result<Option<ConfirmedPayment>, sqlx::Error> {
    sqlx::query_as(
        "select p.id, p.provider_ref, p.amount_paise, \
                coalesce((select sum(r.amount_paise) from refunds r \
                          where r.payment_id = p.id and r.status = 'succeeded'), 0)::bigint \
                    as refunded_paise \
         from payments p where p.order_id = $1 and p.status = 'confirmed'",
    )
    .bind(order_id)
    .fetch_optional(conn)
    .await
}

// ---- changing a refund -------------------------------------------------------------------------

pub struct NewRefund<'a> {
    pub order_id: Uuid,
    pub payment_id: Uuid,
    pub amount_paise: i64,
    pub status: RefundStatus,
    pub origin: &'a str,
    pub requested_by: Option<Uuid>,
    pub decided_by: Option<Uuid>,
    pub reason: Option<&'a str>,
}

/// `None` if the order already has a refund in flight.
pub async fn insert(
    conn: &mut PgConnection,
    new: &NewRefund<'_>,
) -> Result<Option<RefundRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "insert into refunds \
            (order_id, payment_id, amount_paise, status, origin, requested_by, decided_by, \
             reason, decided_at) \
         values ($1,$2,$3,$4,$5,$6,$7,$8, case when $7::uuid is null then null else now() end) \
         on conflict do nothing returning {COLUMNS}"
    ))
    .bind(new.order_id)
    .bind(new.payment_id)
    .bind(new.amount_paise)
    .bind(new.status)
    .bind(new.origin)
    .bind(new.requested_by)
    .bind(new.decided_by)
    .bind(new.reason)
    .fetch_optional(conn)
    .await
}

/// Accepts a student's request: from now it is the provider's to make.
pub async fn approve_request(
    conn: &mut PgConnection,
    id: Uuid,
    decided_by: Uuid,
    reason: Option<&str>,
) -> Result<RefundRow, sqlx::Error> {
    sqlx::query_as(&format!(
        "update refunds set status = 'processing', decided_by = $2, decided_at = now(), \
            reason = coalesce(reason, $3), updated_at = now() \
         where id = $1 and status = 'requested' returning {COLUMNS}"
    ))
    .bind(id)
    .bind(decided_by)
    .bind(reason)
    .fetch_one(conn)
    .await
}

pub async fn decline(
    conn: &mut PgConnection,
    id: Uuid,
    decided_by: Uuid,
    note: &str,
) -> Result<RefundRow, sqlx::Error> {
    sqlx::query_as(&format!(
        "update refunds set status = 'declined', decided_by = $2, decided_at = now(), \
            decision_note = $3, updated_at = now() \
         where id = $1 and status = 'requested' returning {COLUMNS}"
    ))
    .bind(id)
    .bind(decided_by)
    .bind(note)
    .fetch_one(conn)
    .await
}

/// Counts an attempt to ask the provider.
pub async fn count_attempt<'e>(db: impl PgExecutor<'e>, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("update refunds set attempts = attempts + 1, updated_at = now() where id = $1")
        .bind(id)
        .execute(db)
        .await?;
    Ok(())
}

/// Remembers the provider's id for the refund while it is still being made.
pub async fn set_provider_refund<'e>(
    db: impl PgExecutor<'e>,
    id: Uuid,
    provider_refund_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "update refunds set provider_refund_id = $2, updated_at = now() \
         where id = $1 and status = 'processing' \
           and (provider_refund_id is null or provider_refund_id = $2)",
    )
    .bind(id)
    .bind(provider_refund_id)
    .execute(db)
    .await?;
    Ok(())
}

/// The provider could not make it. The order stays paid and the note stays visible.
pub async fn fail<'e>(
    db: impl PgExecutor<'e>,
    id: Uuid,
    note: &str,
) -> Result<Option<RefundRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "update refunds set status = 'failed', decision_note = $2, updated_at = now() \
         where id = $1 and status = 'processing' returning {COLUMNS}"
    ))
    .bind(id)
    .bind(note)
    .fetch_optional(db)
    .await
}

/// Failed refund reported by the provider: only ours when its provider id or receipt identifies
/// our request. A different dashboard refund on the same payment cannot fail our request.
pub async fn fail_reported(
    conn: &mut PgConnection,
    provider_refund_id: &str,
    payment_id: Uuid,
    note: &str,
    local_refund_id: Option<Uuid>,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        "update refunds set status = 'failed', decision_note = $3, \
            provider_refund_id = coalesce(provider_refund_id, $1), updated_at = now() \
         where status = 'processing' and payment_id = $2 and (provider_refund_id = $1 \
            or (id = $4 and provider_refund_id is null)) \
         returning id",
    )
    .bind(provider_refund_id)
    .bind(payment_id)
    .bind(note)
    .bind(local_refund_id)
    .fetch_optional(conn)
    .await
}

/// A refund the provider has completed, now on record.
#[derive(Debug, Clone, Copy)]
pub struct Recorded {
    /// Our row for it (made by us, or created now for a refund made in the provider's dashboard).
    pub refund_id: Uuid,
    /// Everything returned on the payment so far, this refund included.
    pub total_paise: i64,
}

/// Records that the provider has returned `amount_paise` of this payment. Closes our own refund
/// for it if there is one; otherwise it was made outside Zeno (the provider's dashboard) and is
/// recorded as such. The provider's observed amount replaces our requested amount when they
/// differ.
///
/// A refund that would take the payment's returns past the payment itself is not recorded at all
/// (the caller flags it for a person); the database refuses it too, but by then a retry loop would
/// be the answer to a question no retry can fix. The caller holds the order's lock, so two refunds
/// of one payment are counted one after the other.
pub async fn record_success(
    conn: &mut PgConnection,
    order_id: Uuid,
    payment_id: Uuid,
    amount_paise: i64,
    provider_refund_id: &str,
    local_refund_id: Option<Uuid>,
) -> Result<Recording, sqlx::Error> {
    let known: Option<Uuid> = sqlx::query_scalar(
        "select id from refunds where provider_refund_id = $1 and status = 'succeeded'",
    )
    .bind(provider_refund_id)
    .fetch_optional(&mut *conn)
    .await?;
    if known.is_some() {
        return Ok(Recording::AlreadyRecorded);
    }
    let fits: bool = sqlx::query_scalar(
        "select $2 + coalesce((select sum(r.amount_paise) from refunds r \
                               where r.payment_id = p.id and r.status = 'succeeded'), 0) \
                <= p.amount_paise \
         from payments p where p.id = $1",
    )
    .bind(payment_id)
    .bind(amount_paise)
    .fetch_one(&mut *conn)
    .await?;
    if !fits {
        return Ok(Recording::ExceedsPayment);
    }
    let closed: Option<Uuid> = sqlx::query_scalar(
        "update refunds set status = 'succeeded', provider_refund_id = $2, \
            amount_paise = $3, completed_at = now(), updated_at = now() \
         where id = (select id from refunds \
                     where payment_id = $1 and status in ('processing','requested') \
                       and (provider_refund_id = $2 or \
                            (id = $4 and provider_refund_id is null)) \
                     order by (provider_refund_id = $2) desc nulls last, requested_at \
                     limit 1 for update) \
         returning id",
    )
    .bind(payment_id)
    .bind(provider_refund_id)
    .bind(amount_paise)
    .bind(local_refund_id)
    .fetch_optional(&mut *conn)
    .await?;
    let refund_id = match closed {
        Some(id) => id,
        None => {
            sqlx::query_scalar(
                "insert into refunds \
                    (order_id, payment_id, amount_paise, status, origin, provider_refund_id, \
                     decided_at, completed_at) \
                 values ($1, $2, $3, 'succeeded', 'provider', $4, now(), now()) returning id",
            )
            .bind(order_id)
            .bind(payment_id)
            .bind(amount_paise)
            .bind(provider_refund_id)
            .fetch_one(&mut *conn)
            .await?
        }
    };
    let total: i64 = sqlx::query_scalar(
        "select coalesce(sum(amount_paise), 0)::bigint from refunds \
         where payment_id = $1 and status = 'succeeded'",
    )
    .bind(payment_id)
    .fetch_one(conn)
    .await?;
    Ok(Recording::New(Recorded {
        refund_id,
        total_paise: total,
    }))
}

/// What `record_success` did.
#[derive(Debug, Clone, Copy)]
pub enum Recording {
    /// Recorded now.
    New(Recorded),
    /// This exact refund was recorded before (a redelivery).
    AlreadyRecorded,
    /// It would return more than the payment was: nothing was recorded.
    ExceedsPayment,
}

// ---- what a creator sees -------------------------------------------------------------------------

#[derive(Debug, sqlx::FromRow)]
pub struct StaffRefundRow {
    #[sqlx(flatten)]
    pub refund: RefundRow,
    pub buyer_id: Uuid,
    pub buyer_name: String,
}

pub async fn list_for_course<'e>(
    db: impl PgExecutor<'e>,
    course_id: Uuid,
    status: Option<RefundStatus>,
    limit: i64,
) -> Result<Vec<StaffRefundRow>, sqlx::Error> {
    sqlx::query_as(
        "select r.id, r.order_id, r.payment_id, r.amount_paise, r.status, r.origin, \
                r.requested_by, r.decided_by, r.reason, r.decision_note, r.provider_refund_id, \
                r.attempts, r.requested_at, r.decided_at, r.completed_at, \
                u.id as buyer_id, u.name as buyer_name \
         from refunds r join orders o on o.id = r.order_id join users u on u.id = o.user_id \
         where o.course_id = $1 and ($2::refund_status is null or r.status = $2) \
         order by r.requested_at desc, r.id desc limit $3",
    )
    .bind(course_id)
    .bind(status)
    .bind(limit)
    .fetch_all(db)
    .await
}

pub async fn course_owner<'e>(
    db: impl PgExecutor<'e>,
    course_id: Uuid,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar("select creator_id from courses where id = $1")
        .bind(course_id)
        .fetch_optional(db)
        .await
}

pub async fn email_of<'e>(
    db: impl PgExecutor<'e>,
    user_id: Uuid,
) -> Result<Option<String>, sqlx::Error> {
    Ok(
        sqlx::query_scalar::<_, Option<String>>("select email from users where id = $1")
            .bind(user_id)
            .fetch_optional(db)
            .await?
            .flatten(),
    )
}
