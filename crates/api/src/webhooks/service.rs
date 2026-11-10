use chrono::Duration;
use serde_json::{json, Value};
use sqlx::PgConnection;

use crate::error::ApiError;
use crate::mail::OutgoingEmail;
use crate::outbox;
use crate::state::AppState;
use crate::webhooks::event::{self, Event};
use crate::webhooks::repo;
use crate::webhooks::settlement::{self, Outcome, Receipt, RefundNotice, Settled};

/// A receipt is worth sending for a week; after that the account page is the record.
const RECEIPT_TTL_DAYS: i64 = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Received {
    /// Handled now, with this result.
    Handled(Outcome),
    /// Seen before and already handled.
    Duplicate,
}

/// Handles one verified delivery from the provider's webhook.
pub async fn receive(state: &AppState, event_id: &str, body: &[u8]) -> Result<Received, ApiError> {
    let parsed = event::parse(body);
    // The payload is kept as received (it has been verified by signature), or a note that it was
    // not JSON so the record still says something arrived.
    let payload: Value = serde_json::from_slice(body)
        .unwrap_or_else(|_| json!({"unparseable_body_bytes": body.len()}));
    handle(state, event_id, &parsed.name, parsed.event, payload).await
}

/// Applies one payment event, whether it came from the provider's webhook or was found by asking
/// the provider (reconciliation, a refund that finished while we waited): the same code, the same
/// record, the same guarantees. Recording the event and everything it causes share a single
/// transaction, so the answer to "was this event handled?" is never "halfway":
///
/// * a crash or database error rolls it all back, including the record of receipt, and the
///   retry starts clean;
/// * a second delivery of the same event, even at the same instant, waits for the first to
///   finish and then finds it recorded.
///
/// An error here becomes a `5xx` on the webhook, which is what makes the provider retry.
pub async fn handle(
    state: &AppState,
    event_id: &str,
    name: &str,
    event: Result<Event, &'static str>,
    payload: Value,
) -> Result<Received, ApiError> {
    let provider = state.payments.name();

    let mut tx = state.db.begin().await?;
    let Some(record) = repo::insert_event(&mut tx, provider, event_id, name, &payload).await?
    else {
        tx.rollback().await?;
        return Ok(Received::Duplicate);
    };

    let mut result: Settled = match &event {
        Err(reason) => {
            tracing::warn!(event = %name, reason, "unreadable webhook event");
            settlement::settled(Outcome::Malformed, None)
        }
        Ok(Event::PaymentCaptured(payment) | Event::OrderPaid(payment)) => {
            settlement::apply_captured(&mut tx, provider, payment, &payload).await?
        }
        Ok(Event::PaymentFailed(payment)) => {
            settlement::apply_failed(&mut tx, provider, payment, &payload).await?
        }
        Ok(Event::RefundProcessed { refund, payment }) => {
            settlement::apply_refund_processed(
                &mut tx,
                provider,
                refund,
                payment.as_ref(),
                &payload,
            )
            .await?
        }
        Ok(Event::RefundFailed { refund, .. }) => {
            settlement::note_refund_failed(&mut tx, refund).await?
        }
        Ok(Event::Dispute(dispute)) => settlement::note_dispute(&mut tx, name, dispute).await?,
        Ok(Event::Other) => settlement::settled(Outcome::Ignored, None),
    };

    // A paid order gets its invoice in the same transaction, so the payment, the enrollment, the
    // invoice number and the receipt that quotes it all exist together or not at all. Off by
    // default; see `crate::invoices`.
    if matches!(event, Ok(Event::PaymentCaptured(_) | Event::OrderPaid(_))) {
        if let Some(order_id) = result.order_id {
            if let Some(invoice) =
                crate::invoices::service::issue(&mut tx, &state.config.invoices, order_id).await?
            {
                if let Some(receipt) = result.receipt.as_mut() {
                    receipt.invoice_number = Some(invoice.number);
                }
            }
        }
    }

    let mut queued = false;
    if let Some(receipt) = result.receipt {
        queued |= queue_receipt(&mut tx, state, receipt).await?;
    }
    if let Some(notice) = result.refund_notice {
        queued |= queue_refund_notice(&mut tx, state, notice).await?;
    }

    repo::finish_event(
        &mut tx,
        record,
        result.order_id,
        result.outcome.as_str(),
        result.outcome.needs_review(),
    )
    .await?;
    tx.commit().await?;

    tracing::info!(
        event = %name,
        outcome = result.outcome.as_str(),
        needs_review = result.outcome.needs_review(),
        "webhook handled"
    );
    if queued {
        outbox::kick(state);
    }
    Ok(Received::Handled(result.outcome))
}

fn rupees(paise: i64) -> String {
    format!("Rs {}.{:02}", paise / 100, paise % 100)
}

/// The receipt joins the payment transaction. A failed outbox write rolls the payment and event
/// back, so the provider can retry the same event without losing the receipt.
async fn queue_receipt(
    conn: &mut PgConnection,
    state: &AppState,
    receipt: Receipt,
) -> Result<bool, sqlx::Error> {
    let to: Option<String> = sqlx::query_scalar("select email from users where id = $1")
        .bind(receipt.user_id)
        .fetch_optional(&mut *conn)
        .await?
        .flatten();
    let Some(to) = to else {
        return Ok(false);
    };
    let invoice = receipt
        .invoice_number
        .as_ref()
        .map(|number| format!("\nInvoice: {number}"))
        .unwrap_or_default();
    let message = OutgoingEmail {
        to,
        subject: format!("Payment received for {}", receipt.course_title),
        text: format!(
            "Thank you. We received {} for \"{}\" and your access is ready:\n{}/my-courses\n\nOrder: {}{invoice}",
            rupees(receipt.amount_paise),
            receipt.course_title,
            state.config.client_origin,
            receipt.order_id
        ),
    };
    outbox::enqueue_in(conn, message, Duration::days(RECEIPT_TTL_DAYS)).await?;
    Ok(true)
}

/// Tells the buyer their money is on its way back, in the same transaction that ended their
/// access, so the notice is never lost to a crash between the two.
async fn queue_refund_notice(
    conn: &mut PgConnection,
    state: &AppState,
    notice: RefundNotice,
) -> Result<bool, sqlx::Error> {
    crate::refunds::service::queue_mail(
        conn,
        notice.user_id,
        format!("Your refund for {} is complete", notice.course_title),
        format!(
            "We have refunded {} for \"{}\". It reaches your account when your bank completes it, usually within a few days.\n\nOrder: {}\n{}/my-courses",
            rupees(notice.amount_paise),
            notice.course_title,
            notice.order_id,
            state.config.client_origin
        ),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::rupees;

    #[test]
    fn amounts_print_as_rupees_and_paise() {
        assert_eq!(rupees(49_900), "Rs 499.00");
        assert_eq!(rupees(100), "Rs 1.00");
        assert_eq!(rupees(12_345), "Rs 123.45");
        assert_eq!(rupees(5), "Rs 0.05");
    }
}
