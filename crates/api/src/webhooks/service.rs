use chrono::Duration;
use serde_json::{json, Value};

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
    if let Some(receipt) = result.receipt {
        send_receipt(state, receipt).await;
    }
    Ok(Received::Handled(result.outcome))
}

fn rupees(paise: i64) -> String {
    format!("Rs {}.{:02}", paise / 100, paise % 100)
}

/// Queued after the payment is committed. If queueing fails the payment still stands; the
/// failure is logged, not turned into a retry, because a retry would find the event handled.
async fn send_receipt(state: &AppState, receipt: Receipt) {
    let email: Result<Option<String>, sqlx::Error> =
        sqlx::query_scalar("select email from users where id = $1")
            .bind(receipt.user_id)
            .fetch_optional(&state.db)
            .await
            .map(Option::flatten);
    let to = match email {
        Ok(Some(to)) => to,
        Ok(None) => return,
        Err(error) => {
            tracing::error!(?error, order = %receipt.order_id, "could not look up a receipt address");
            return;
        }
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
    if let Err(error) = outbox::enqueue(state, message, Duration::days(RECEIPT_TTL_DAYS)).await {
        tracing::error!(?error, order = %receipt.order_id, "could not queue a receipt");
    }
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
