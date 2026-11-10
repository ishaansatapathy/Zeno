use serde_json::Value;
use sqlx::PgConnection;
use uuid::Uuid;
use zeno_core::model::{EnrollmentSource, OrderStatus};

// ---- the record of events received ---------------------------------------------------

/// Records that an event arrived. `None` means this exact event was already recorded (and, since
/// recording and handling share a transaction, already handled): a duplicate. A second delivery
/// racing the first waits here until the first commits or rolls back.
pub async fn insert_event(
    conn: &mut PgConnection,
    provider: &str,
    provider_event_id: &str,
    event_type: &str,
    payload: &Value,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        "insert into webhook_events (provider, provider_event_id, event_type, payload, outcome) \
         values ($1, $2, $3, $4, 'received') \
         on conflict (provider, provider_event_id) do nothing \
         returning id",
    )
    .bind(provider)
    .bind(provider_event_id)
    .bind(event_type)
    .bind(payload)
    .fetch_optional(conn)
    .await
}

pub async fn finish_event(
    conn: &mut PgConnection,
    id: Uuid,
    order_id: Option<Uuid>,
    outcome: &str,
    needs_review: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "update webhook_events set order_id = $2, outcome = $3, needs_review = $4, \
            processed_at = now() where id = $1",
    )
    .bind(id)
    .bind(order_id)
    .bind(outcome)
    .bind(needs_review)
    .execute(conn)
    .await?;
    Ok(())
}

// ---- orders ---------------------------------------------------------------------------

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct LockedOrder {
    pub id: Uuid,
    pub user_id: Uuid,
    pub course_id: Uuid,
    pub amount_paise: i64,
    pub currency: String,
    pub status: OrderStatus,
    pub price_snapshot: Value,
}

const ORDER_COLUMNS: &str =
    "id, user_id, course_id, amount_paise, currency, status, price_snapshot";

/// The order behind a provider's order id, locked until the transaction ends so two events about
/// the same order take turns.
pub async fn lock_order_by_provider_id(
    conn: &mut PgConnection,
    provider: &str,
    provider_order_id: &str,
) -> Result<Option<LockedOrder>, sqlx::Error> {
    sqlx::query_as(&format!(
        "select {ORDER_COLUMNS} from orders \
         where provider = $1 and provider_order_id = $2 for update"
    ))
    .bind(provider)
    .bind(provider_order_id)
    .fetch_optional(conn)
    .await
}

pub async fn lock_order_by_id(
    conn: &mut PgConnection,
    id: Uuid,
) -> Result<Option<LockedOrder>, sqlx::Error> {
    sqlx::query_as(&format!(
        "select {ORDER_COLUMNS} from orders where id = $1 for update"
    ))
    .bind(id)
    .fetch_optional(conn)
    .await
}

pub async fn set_order_status(
    conn: &mut PgConnection,
    id: Uuid,
    status: OrderStatus,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "update orders set status = $2, attempt_lease_until = null, updated_at = now(), \
            paid_at = case when $2 = 'paid'::order_status then coalesce(paid_at, now()) else paid_at end \
         where id = $1",
    )
    .bind(id)
    .bind(status)
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn add_order_event(
    conn: &mut PgConnection,
    order_id: Uuid,
    kind: &str,
    payment_id: Option<Uuid>,
    details: Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "insert into order_events (order_id, kind, payment_id, details) values ($1, $2, $3, $4)",
    )
    .bind(order_id)
    .bind(kind)
    .bind(payment_id)
    .bind(details)
    .execute(conn)
    .await?;
    Ok(())
}

// ---- payments -------------------------------------------------------------------------

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PaymentRow {
    pub id: Uuid,
    pub order_id: Option<Uuid>,
    pub amount_paise: i64,
    pub status: String,
}

pub async fn payment_by_ref(
    conn: &mut PgConnection,
    provider_ref: &str,
    lock: bool,
) -> Result<Option<PaymentRow>, sqlx::Error> {
    let suffix = if lock { " for update" } else { "" };
    sqlx::query_as(&format!(
        "select id, order_id, amount_paise, status::text as status from payments \
         where provider_ref = $1{suffix}"
    ))
    .bind(provider_ref)
    .fetch_optional(conn)
    .await
}

/// A payment to record, as the provider reported it.
pub struct NewPayment<'a> {
    pub provider: &'a str,
    pub provider_ref: &'a str,
    pub order_id: Uuid,
    pub amount_paise: i64,
    pub currency: &'a str,
    /// One of the `payment_status` values.
    pub status: &'a str,
    pub raw_event: &'a Value,
}

pub async fn insert_payment(
    conn: &mut PgConnection,
    payment: &NewPayment<'_>,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar(
        "insert into payments             (provider, provider_ref, order_id, amount_paise, currency, status, raw_event)          values ($1, $2, $3, $4, $5, $6::payment_status, $7) returning id",
    )
    .bind(payment.provider)
    .bind(payment.provider_ref)
    .bind(payment.order_id)
    .bind(payment.amount_paise)
    .bind(payment.currency)
    .bind(payment.status)
    .bind(payment.raw_event)
    .fetch_one(conn)
    .await
}

pub async fn set_payment_status(
    conn: &mut PgConnection,
    id: Uuid,
    status: &str,
    raw_event: Option<&Value>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "update payments set status = $2::payment_status, raw_event = coalesce($3, raw_event), \
            updated_at = now() where id = $1",
    )
    .bind(id)
    .bind(status)
    .bind(raw_event)
    .execute(conn)
    .await?;
    Ok(())
}

// ---- enrollments ----------------------------------------------------------------------

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct EnrollmentState {
    pub id: Uuid,
    pub payment_id: Option<Uuid>,
    pub revoked: bool,
    pub revocation_kind: Option<String>,
    pub expired: bool,
}

pub async fn enrollment_for_update(
    conn: &mut PgConnection,
    user_id: Uuid,
    course_id: Uuid,
) -> Result<Option<EnrollmentState>, sqlx::Error> {
    sqlx::query_as(
        "select id, payment_id, revoked_at is not null as revoked, revocation_kind, \
                coalesce(expires_at <= now(), false) as expired \
         from enrollments where user_id = $1 and course_id = $2 for update",
    )
    .bind(user_id)
    .bind(course_id)
    .fetch_optional(conn)
    .await
}

/// `None` when someone else created the enrollment first.
pub async fn insert_payment_enrollment(
    conn: &mut PgConnection,
    user_id: Uuid,
    course_id: Uuid,
    payment_id: Uuid,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        "insert into enrollments (user_id, course_id, source, payment_id) \
         values ($1, $2, $3, $4) on conflict (user_id, course_id) do nothing returning id",
    )
    .bind(user_id)
    .bind(course_id)
    .bind(EnrollmentSource::Payment)
    .bind(payment_id)
    .fetch_optional(conn)
    .await
}

/// Gives back access that had ended (a refund, an expiry) because the person has paid again.
pub async fn reactivate_with_payment(
    conn: &mut PgConnection,
    enrollment_id: Uuid,
    payment_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "update enrollments set source = $3, payment_id = $2, granted_by = null, \
            expires_at = null, revoked_at = null, revoked_by = null, revocation_kind = null, \
            revoke_reason = null, updated_at = now() where id = $1",
    )
    .bind(enrollment_id)
    .bind(payment_id)
    .bind(EnrollmentSource::Payment)
    .execute(conn)
    .await?;
    Ok(())
}

/// Remembers which payment someone's existing access was also paid for.
pub async fn link_payment(
    conn: &mut PgConnection,
    enrollment_id: Uuid,
    payment_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "update enrollments set payment_id = coalesce(payment_id, $2), updated_at = now() \
         where id = $1",
    )
    .bind(enrollment_id)
    .bind(payment_id)
    .execute(conn)
    .await?;
    Ok(())
}

#[derive(Debug, sqlx::FromRow)]
pub struct Revoked {
    pub id: Uuid,
    pub user_id: Uuid,
    pub source: EnrollmentSource,
}

/// Ends the access that this payment bought (and only that).
pub async fn revoke_for_payment(
    conn: &mut PgConnection,
    payment_id: Uuid,
    reason: &str,
) -> Result<Vec<Revoked>, sqlx::Error> {
    sqlx::query_as(
        "update enrollments set revoked_at = now(), revoke_reason = $2, \
            revocation_kind = 'payment', updated_at = now() \
         where payment_id = $1 and revoked_at is null \
         returning id, user_id, source",
    )
    .bind(payment_id)
    .bind(reason)
    .fetch_all(conn)
    .await
}
