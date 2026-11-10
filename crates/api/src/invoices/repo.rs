use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgConnection, PgExecutor};
use uuid::Uuid;
use zeno_core::model::OrderStatus;

const COLUMNS: &str =
    "id, order_id, user_id, fy, sequence, number, amount_paise, currency, data, issued_at";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct InvoiceRow {
    pub id: Uuid,
    pub order_id: Uuid,
    pub user_id: Uuid,
    pub fy: String,
    pub sequence: i32,
    pub number: String,
    pub amount_paise: i64,
    pub currency: String,
    pub data: Value,
    pub issued_at: DateTime<Utc>,
}

pub async fn by_order<'e>(
    db: impl PgExecutor<'e>,
    order_id: Uuid,
) -> Result<Option<InvoiceRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "select {COLUMNS} from invoices where order_id = $1"
    ))
    .bind(order_id)
    .fetch_optional(db)
    .await
}

#[derive(Debug, sqlx::FromRow)]
pub struct OrderFacts {
    pub id: Uuid,
    pub user_id: Uuid,
    pub amount_paise: i64,
    pub currency: String,
    pub status: OrderStatus,
    pub price_snapshot: Value,
}

/// The order, locked: two requests to issue the invoice for it take turns, so the second finds
/// the first one's invoice instead of spending another number.
pub async fn lock_order(
    conn: &mut PgConnection,
    order_id: Uuid,
) -> Result<Option<OrderFacts>, sqlx::Error> {
    sqlx::query_as(
        "select id, user_id, amount_paise, currency, status, price_snapshot \
         from orders where id = $1 for update",
    )
    .bind(order_id)
    .fetch_optional(conn)
    .await
}

pub async fn buyer(
    conn: &mut PgConnection,
    user_id: Uuid,
) -> Result<(String, Option<String>), sqlx::Error> {
    sqlx::query_as("select name, email from users where id = $1")
        .bind(user_id)
        .fetch_one(conn)
        .await
}

pub async fn confirmed_payment_ref(
    conn: &mut PgConnection,
    order_id: Uuid,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar(
        "select provider_ref from payments where order_id = $1 and status = 'confirmed'",
    )
    .bind(order_id)
    .fetch_optional(conn)
    .await
}

pub async fn db_now(conn: &mut PgConnection) -> Result<DateTime<Utc>, sqlx::Error> {
    sqlx::query_scalar("select now()").fetch_one(conn).await
}

/// Takes the next number of the financial year. The row is locked until the transaction ends, so
/// numbers are handed out one at a time, and a transaction that rolls back gives its number back.
pub async fn next_sequence(conn: &mut PgConnection, fy: &str) -> Result<i32, sqlx::Error> {
    sqlx::query_scalar(
        "insert into invoice_counters (fy, last_sequence) values ($1, 1) \
         on conflict (fy) do update set last_sequence = invoice_counters.last_sequence + 1 \
         returning last_sequence",
    )
    .bind(fy)
    .fetch_one(conn)
    .await
}

pub struct NewInvoice<'a> {
    pub order_id: Uuid,
    pub user_id: Uuid,
    pub fy: &'a str,
    pub sequence: i32,
    pub number: &'a str,
    pub amount_paise: i64,
    pub currency: &'a str,
    pub data: &'a Value,
    pub issued_at: DateTime<Utc>,
}

pub async fn insert(
    conn: &mut PgConnection,
    invoice: &NewInvoice<'_>,
) -> Result<InvoiceRow, sqlx::Error> {
    sqlx::query_as(&format!(
        "insert into invoices \
            (order_id, user_id, fy, sequence, number, amount_paise, currency, data, issued_at) \
         values ($1,$2,$3,$4,$5,$6,$7,$8,$9) returning {COLUMNS}"
    ))
    .bind(invoice.order_id)
    .bind(invoice.user_id)
    .bind(invoice.fy)
    .bind(invoice.sequence)
    .bind(invoice.number)
    .bind(invoice.amount_paise)
    .bind(invoice.currency)
    .bind(invoice.data)
    .bind(invoice.issued_at)
    .fetch_one(conn)
    .await
}
