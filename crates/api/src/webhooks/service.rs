use chrono::Duration;
use serde_json::{json, Value};
use sqlx::PgConnection;

use crate::error::ApiError;
use crate::mail::OutgoingEmail;
use crate::outbox;
use crate::state::AppState;
use crate::webhooks::event::{self, Event};
use crate::webhooks::repo;
use crate::webhooks::settlement::{self, Outcome, Receipt, Settled};

/// A receipt is worth sending for a week; after that the account page is the record.
const RECEIPT_TTL_DAYS: i64 = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Received {
    /// Handled now, with this result.
    Handled(Outcome),
    /// Seen before and already handled.
    Duplicate,
}

/// Handles one verified delivery. Recording the event and everything it causes share a single
/// transaction, so the answer to "was this event handled?" is never "halfway":
///
/// * a crash or database error rolls it all back, including the record of receipt, and the
///   provider's retry starts clean;
/// * a second delivery of the same event, even at the same instant, waits for the first to
///   finish and then finds it recorded.
///
/// An error here becomes a `5xx`, which is what makes the provider retry.
pub async fn receive(state: &AppState, event_id: &str, body: &[u8]) -> Result<Received, ApiError> {
    let provider = state.payments.name();
    let parsed = event::parse(body);
    // The payload is kept as received (it has been verified by signature), or a note that it was
    // not JSON so the record still says something arrived.
    let payload: Value = serde_json::from_slice(body)
        .unwrap_or_else(|_| json!({"unparseable_body_bytes": body.len()}));

    let mut tx = state.db.begin().await?;
    let Some(record) =
        repo::insert_event(&mut tx, provider, event_id, &parsed.name, &payload).await?
    else {
        tx.rollback().await?;
        return Ok(Received::Duplicate);
    };

    let result: Settled = match &parsed.event {
        Err(reason) => {
            tracing::warn!(event = %parsed.name, reason, "unreadable webhook event");
            Settled {
                outcome: Outcome::Malformed,
                order_id: None,
                receipt: None,
            }
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
        Ok(Event::Other) => Settled {
            outcome: Outcome::Ignored,
            order_id: None,
            receipt: None,
        },
    };

    let receipt_queued = match result.receipt {
        Some(receipt) => queue_receipt(&mut tx, state, receipt).await?,
        None => false,
    };

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
        event = %parsed.name,
        outcome = result.outcome.as_str(),
        needs_review = result.outcome.needs_review(),
        "webhook handled"
    );
    if receipt_queued {
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
    let message = OutgoingEmail {
        to,
        subject: format!("Payment received for {}", receipt.course_title),
        text: format!(
            "Thank you. We received {} for \"{}\" and your access is ready:\n{}/my-courses\n\nOrder: {}",
            rupees(receipt.amount_paise),
            receipt.course_title,
            state.config.client_origin,
            receipt.order_id
        ),
    };
    outbox::enqueue_in(conn, message, Duration::days(RECEIPT_TTL_DAYS)).await?;
    Ok(true)
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
