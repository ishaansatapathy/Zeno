use axum::http::StatusCode;
use uuid::Uuid;
use zeno_core::model::OrderStatus;

use crate::auth::extractor::AuthUser;
use crate::error::ApiError;
use crate::invoices::dto::{Buyer, InvoiceData, InvoiceEnvelope, InvoiceResponse, Line, Seller};
use crate::invoices::fy::{financial_year, invoice_number};
use crate::invoices::repo::{self, InvoiceRow, NewInvoice};
use crate::invoices::InvoicesConfig;
use crate::orders::dto::PriceSnapshot;
use crate::state::AppState;

/// Issues the invoice for a paid order, or returns the one already issued. Safe to call any number
/// of times, from anywhere, concurrently: the order row is locked, there is one invoice per order,
/// and a number is taken only when an invoice is actually written, in the same transaction.
///
/// `None` when invoices are off, or the order is not paid and has no invoice.
pub async fn issue(
    conn: &mut sqlx::PgConnection,
    config: &InvoicesConfig,
    order_id: Uuid,
) -> Result<Option<InvoiceRow>, sqlx::Error> {
    if !config.enabled {
        return Ok(None);
    }
    let Some(order) = repo::lock_order(conn, order_id).await? else {
        return Ok(None);
    };
    if let Some(existing) = repo::by_order(&mut *conn, order_id).await? {
        return Ok(Some(existing));
    }
    if order.status != OrderStatus::Paid {
        return Ok(None);
    }
    let snapshot: PriceSnapshot = serde_json::from_value(order.price_snapshot.clone())
        .map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
    let (name, email) = repo::buyer(conn, order.user_id).await?;
    let payment_ref = repo::confirmed_payment_ref(conn, order.id).await?;

    let issued_at = repo::db_now(conn).await?;
    let fy = financial_year(issued_at);
    let sequence = repo::next_sequence(conn, &fy).await?;
    let number = invoice_number(&config.prefix, &fy, sequence);

    let data = InvoiceData {
        v: 1,
        number: number.clone(),
        issued_at,
        order_id: order.id,
        payment_ref,
        seller: Seller {
            name: config.seller_name.clone(),
            address: config.seller_address.clone(),
        },
        buyer: Buyer { name, email },
        lines: vec![Line {
            description: snapshot.course_title.clone(),
            list_price_paise: snapshot.list_price_paise,
            discount_paise: snapshot.discount_paise,
            coupon_code: snapshot.coupon.as_ref().map(|coupon| coupon.code.clone()),
            amount_paise: order.amount_paise,
        }],
        total_paise: order.amount_paise,
        currency: order.currency.clone(),
        tax: None,
    };
    let data_json = serde_json::to_value(&data).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
    let row = repo::insert(
        conn,
        &NewInvoice {
            order_id: order.id,
            user_id: order.user_id,
            fy: &fy,
            sequence,
            number: &number,
            amount_paise: order.amount_paise,
            currency: &order.currency,
            data: &data_json,
            issued_at,
        },
    )
    .await?;
    Ok(Some(row))
}

fn not_found(subject: &'static str) -> ApiError {
    ApiError::Http {
        status: StatusCode::NOT_FOUND,
        code: "not_found",
        message: format!("{subject} not found."),
    }
}

/// The invoice for the buyer's own order, issuing it now if the order is paid and has none (for
/// example because invoices were switched on after it was paid).
pub async fn for_buyer(
    state: &AppState,
    user: &AuthUser,
    order_id: Uuid,
) -> Result<InvoiceRow, ApiError> {
    // Someone else's order looks exactly like one that does not exist.
    let order = crate::orders::repo::by_id_for_user(&state.db, order_id, user.id)
        .await?
        .ok_or_else(|| not_found("Order"))?;
    if !state.config.invoices.enabled {
        return Err(ApiError::Http {
            status: StatusCode::CONFLICT,
            code: "invoices_unavailable",
            message: "Invoices are not available yet. Your payment receipt was emailed to you."
                .into(),
        });
    }
    if let Some(existing) = repo::by_order(&state.db, order.id).await? {
        return Ok(existing);
    }
    let mut tx = state.db.begin().await?;
    let issued = issue(&mut tx, &state.config.invoices, order.id).await?;
    tx.commit().await?;
    issued.ok_or_else(|| ApiError::Http {
        status: StatusCode::CONFLICT,
        code: "not_invoiceable",
        message: "Only a paid order has an invoice.".into(),
    })
}

pub fn envelope(row: &InvoiceRow) -> Result<InvoiceEnvelope, ApiError> {
    let invoice: InvoiceData =
        serde_json::from_value(row.data.clone()).map_err(anyhow::Error::from)?;
    Ok(InvoiceEnvelope {
        invoice: InvoiceResponse {
            number: row.number.clone(),
            issued_at: row.issued_at,
            order_id: row.order_id,
            amount_paise: row.amount_paise,
            currency: row.currency.clone(),
            invoice,
        },
    })
}
