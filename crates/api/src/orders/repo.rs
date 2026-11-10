use std::time::Duration;

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgExecutor};
use uuid::Uuid;
use zeno_core::model::{CourseStatus, OrderStatus};

/// Every column of an order, plus whether an open order has run past its expiry. The expiry
/// sweep runs on a schedule, so reads work it out themselves rather than showing a stale
/// "pending".
const ORDER_SELECT: &str = "id, user_id, course_id, amount_paise, currency, price_snapshot, \
    status, provider, provider_order_id, idempotency_key, provider_attempts, expires_at, \
    created_at, paid_at, (status in ('created','pending') and expires_at <= now()) as lapsed";

/// The claim outlives the entire bounded provider recovery operation, including receipt lookup
/// retries and a possible create call. A timed-out operation is cancelled before the claim lapses.
const ATTEMPT_LEASE_SECS: i32 = 60;
pub const ATTEMPT_WORK_TIMEOUT: Duration = Duration::from_secs(45);
const _: () = assert!(ATTEMPT_LEASE_SECS as u64 > ATTEMPT_WORK_TIMEOUT.as_secs());

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct OrderRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub course_id: Uuid,
    pub amount_paise: i64,
    pub currency: String,
    pub price_snapshot: serde_json::Value,
    pub status: OrderStatus,
    pub provider: String,
    pub provider_order_id: Option<String>,
    pub idempotency_key: Option<String>,
    pub provider_attempts: i32,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub paid_at: Option<DateTime<Utc>>,
    pub lapsed: bool,
}

impl OrderRow {
    /// The status to show: an open order past its expiry is expired, whatever the sweep says.
    pub fn effective_status(&self) -> OrderStatus {
        if self.lapsed {
            OrderStatus::Expired
        } else {
            self.status
        }
    }
}

#[derive(Debug, sqlx::FromRow)]
pub struct CourseForCheckout {
    pub id: Uuid,
    pub creator_id: Uuid,
    pub title: String,
    pub slug: String,
    pub price_paise: i64,
    pub currency: String,
    pub status: CourseStatus,
    pub refund_window_days: i32,
    pub deleted: bool,
}

pub async fn course<'e>(
    db: impl PgExecutor<'e>,
    course_id: Uuid,
) -> Result<Option<CourseForCheckout>, sqlx::Error> {
    sqlx::query_as(
        "select id, creator_id, title, slug, price_paise, currency, status, refund_window_days, \
                deleted_at is not null as deleted \
         from courses where id = $1",
    )
    .bind(course_id)
    .fetch_optional(db)
    .await
}

#[derive(Debug, sqlx::FromRow)]
pub struct EnrollmentGate {
    pub live: bool,
    pub revoked: bool,
    pub revocation_kind: Option<String>,
}

pub async fn enrollment_gate<'e>(
    db: impl PgExecutor<'e>,
    user_id: Uuid,
    course_id: Uuid,
) -> Result<Option<EnrollmentGate>, sqlx::Error> {
    sqlx::query_as(
        "select (revoked_at is null and (expires_at is null or expires_at > now())) as live, \
                revoked_at is not null as revoked, revocation_kind \
         from enrollments where user_id = $1 and course_id = $2",
    )
    .bind(user_id)
    .bind(course_id)
    .fetch_optional(db)
    .await
}

pub async fn by_id<'e>(
    db: impl PgExecutor<'e>,
    order_id: Uuid,
) -> Result<Option<OrderRow>, sqlx::Error> {
    sqlx::query_as(&format!("select {ORDER_SELECT} from orders where id = $1"))
        .bind(order_id)
        .fetch_optional(db)
        .await
}

pub async fn by_id_for_user<'e>(
    db: impl PgExecutor<'e>,
    order_id: Uuid,
    user_id: Uuid,
) -> Result<Option<OrderRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "select {ORDER_SELECT} from orders where id = $1 and user_id = $2"
    ))
    .bind(order_id)
    .bind(user_id)
    .fetch_optional(db)
    .await
}

pub async fn by_key<'e>(
    db: impl PgExecutor<'e>,
    user_id: Uuid,
    key: &str,
) -> Result<Option<OrderRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "select {ORDER_SELECT} from orders where user_id = $1 and idempotency_key = $2"
    ))
    .bind(user_id)
    .bind(key)
    .fetch_optional(db)
    .await
}

/// The user's open order for the course, if any (`created` or `pending`).
pub async fn open_order<'e>(
    db: impl PgExecutor<'e>,
    user_id: Uuid,
    course_id: Uuid,
) -> Result<Option<OrderRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "select {ORDER_SELECT} from orders \
         where user_id = $1 and course_id = $2 and status in ('created','pending')"
    ))
    .bind(user_id)
    .bind(course_id)
    .fetch_optional(db)
    .await
}

/// Marks an open order expired and gives back the coupon it was holding, in one statement. Does
/// nothing if the order already moved on (a payment may have landed).
pub async fn expire<'e>(db: impl PgExecutor<'e>, order_id: Uuid) -> Result<u64, sqlx::Error> {
    let expired: i64 = sqlx::query_scalar(
        "with expired as ( \
            update orders set status = 'expired', attempt_lease_until = null, updated_at = now() \
            where id = $1 and status in ('created','pending') returning id), \
         released as ( \
            update coupon_redemptions r set released_at = now() from expired e \
            where r.order_id = e.id and r.released_at is null returning r.coupon_id), \
         given_back as ( \
            update coupons c set redeemed_count = c.redeemed_count - n.uses, updated_at = now() \
            from (select coupon_id, count(*)::int as uses from released group by coupon_id) n \
            where c.id = n.coupon_id returning c.id) \
         select count(*) from expired",
    )
    .bind(order_id)
    .fetch_one(db)
    .await?;
    Ok(u64::try_from(expired).unwrap_or(0))
}

pub struct NewOrder<'a> {
    pub id: Uuid,
    pub user_id: Uuid,
    pub course_id: Uuid,
    pub amount_paise: i64,
    pub currency: &'a str,
    pub price_snapshot: &'a serde_json::Value,
    pub provider: &'a str,
    pub idempotency_key: Option<&'a str>,
    pub ttl_minutes: i32,
}

/// Inserts the order unless the user already has an open one for the course or has used this
/// idempotency key (`None`: the caller should look the existing order up).
pub async fn insert(
    db: &mut PgConnection,
    new: &NewOrder<'_>,
) -> Result<Option<OrderRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "insert into orders \
            (id, user_id, course_id, amount_paise, currency, price_snapshot, provider, \
             idempotency_key, expires_at) \
         values ($1,$2,$3,$4,$5,$6,$7,$8, now() + make_interval(mins => $9)) \
         on conflict do nothing \
         returning {ORDER_SELECT}"
    ))
    .bind(new.id)
    .bind(new.user_id)
    .bind(new.course_id)
    .bind(new.amount_paise)
    .bind(new.currency)
    .bind(new.price_snapshot)
    .bind(new.provider)
    .bind(new.idempotency_key)
    .bind(new.ttl_minutes)
    .fetch_optional(db)
    .await
}

/// Takes the right to ask the provider about this order. `Some(n)` means we hold it and `n`
/// earlier attempts were made (so an answer may already exist at the provider); `None` means
/// someone else holds it, or the order has moved on.
pub async fn claim_attempt<'e>(
    db: impl PgExecutor<'e>,
    order_id: Uuid,
) -> Result<Option<i32>, sqlx::Error> {
    sqlx::query_scalar(
        "update orders set provider_attempts = provider_attempts + 1, \
            attempt_lease_until = now() + make_interval(secs => $2), updated_at = now() \
         where id = $1 and status = 'created' and provider_order_id is null \
           and expires_at > now() \
           and (attempt_lease_until is null or attempt_lease_until <= now()) \
         returning provider_attempts - 1",
    )
    .bind(order_id)
    .bind(f64::from(ATTEMPT_LEASE_SECS))
    .fetch_optional(db)
    .await
}

pub async fn release_attempt<'e>(
    db: impl PgExecutor<'e>,
    order_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query("update orders set attempt_lease_until = null, updated_at = now() where id = $1")
        .bind(order_id)
        .execute(db)
        .await?;
    Ok(())
}

/// Records the provider's order and opens the order for payment. `None` if the order was
/// expired (or already attached) in the meantime.
pub async fn attach_provider_order<'e>(
    db: impl PgExecutor<'e>,
    order_id: Uuid,
    provider_order_id: &str,
) -> Result<Option<OrderRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "update orders set provider_order_id = $2, status = 'pending', \
            attempt_lease_until = null, updated_at = now() \
         where id = $1 and status = 'created' and provider_order_id is null \
         returning {ORDER_SELECT}"
    ))
    .bind(order_id)
    .bind(provider_order_id)
    .fetch_optional(db)
    .await
}
