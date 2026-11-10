//! Reconciliation: money states that never get stuck.
//!
//! Webhooks are how we normally learn that a payment happened, but webhooks can be lost: the
//! provider gives up after a day, our server is down, a deploy swallows a request. This job asks
//! the provider instead, for anything that has been waiting too long, and feeds the answer through
//! the very same code the webhook uses ([`crate::webhooks::service::handle`]), so there is one
//! state machine, not two.
//!
//! It also finishes what time alone decides:
//!
//! * an unpaid order past its expiry becomes `expired` (after checking the provider, so a payment
//!   made in the last moments is not lost; an order with a payment attempt in flight is given an
//!   extra hour);
//! * recently expired orders are looked at again for a day, in case a late payment's webhook
//!   never arrived;
//! * refunds the provider was asked for and never answered are asked again (the refund's own id is
//!   the idempotency key, so this cannot refund twice), and ones still pending are checked.
//!
//! Safe to run on every instance at once: each pass first *claims* its batch (rows are skipped if
//! another instance holds them, and pushed into the future so a crashed run is retried later),
//! and everything it does is idempotent.

use std::time::Duration;

use chrono::{DateTime, Utc};
use rand::Rng;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;
use zeno_core::model::{OrderStatus, RefundStatus};
use zeno_core::payments::{
    ProviderError, ProviderOrderStatus, ProviderPayment, ProviderPaymentStatus,
    ProviderRefundStatus,
};

use crate::error::ApiError;
use crate::orders::repo as orders;
use crate::refunds::repo as refunds;
use crate::refunds::service as refund_service;
use crate::state::AppState;
use crate::webhooks::event::{Event, PaymentEntity, RefundEntity};
use crate::webhooks::service::{handle, Received};
use crate::webhooks::settlement::Outcome;

const BATCH: i64 = 50;
/// A pending order is first checked this long after it was made: long enough that the webhook,
/// when it works, has long since done the job.
const FIRST_CHECK_MINUTES: i32 = 15;
const PENDING_RECHECK_MINUTES: i32 = 5;
const EXPIRED_RECHECK_MINUTES: i32 = 120;
/// How long a row stays claimed by the instance working on it.
const CLAIM_MINUTES: i32 = 10;
/// An order whose customer tried to pay is kept open this long past its expiry before giving up.
const ATTEMPT_GRACE: chrono::Duration = chrono::Duration::hours(1);
const PENDING_REFUND_RECHECK_MINUTES: i32 = 30;
const MAX_BACKOFF_SECS: u64 = 3600;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Report {
    pub orders_checked: u32,
    pub orders_settled: u32,
    pub orders_expired: u32,
    pub order_errors: u32,
    pub refunds_checked: u32,
    pub refunds_asked_again: u32,
    pub refunds_settled: u32,
    pub refunds_failed: u32,
    pub refund_errors: u32,
}

/// One pass over everything that is due.
pub async fn run_once(state: &AppState) -> Result<Report, ApiError> {
    let mut report = Report::default();
    reconcile_orders(state, &mut report).await?;
    reconcile_refunds(state, &mut report).await?;
    Ok(report)
}

/// Runs [`run_once`] on a timer, for as long as the process lives.
pub fn spawn_periodic(state: AppState, every: Duration) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(every);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            match run_once(&state).await {
                Ok(report) if report == Report::default() => {}
                Ok(report) => tracing::info!(?report, "payment reconciliation"),
                Err(error) => tracing::error!(?error, "payment reconciliation failed"),
            }
        }
    });
}

/// Seconds to wait after the `failures`-th failure in a row: a minute, doubling, up to an hour,
/// give or take a quarter so instances that failed together do not retry together.
fn backoff(failures: i32) -> u64 {
    let base = 60u64
        .saturating_mul(1u64 << failures.clamp(0, 6))
        .min(MAX_BACKOFF_SECS);
    let jitter = rand::thread_rng().gen_range(0.75..1.25);
    ((base as f64) * jitter) as u64
}

// ---- orders -------------------------------------------------------------------------------------

#[derive(Debug, sqlx::FromRow)]
struct ClaimedOrder {
    id: Uuid,
    status: OrderStatus,
    provider_order_id: Option<String>,
    lapsed: bool,
    expires_at: DateTime<Utc>,
    reconcile_failures: i32,
    db_now: DateTime<Utc>,
}

async fn claim_orders(db: &PgPool) -> Result<Vec<ClaimedOrder>, sqlx::Error> {
    sqlx::query_as(
        "update orders set reconcile_after = now() + make_interval(mins => $2) \
         where id in ( \
            select id from orders \
            where ( (status = 'pending' and created_at <= now() - make_interval(mins => $3)) \
                 or (status = 'created' and expires_at <= now()) \
                 or (status = 'expired' and provider_order_id is not null \
                     and expires_at > now() - interval '24 hours') ) \
              and (reconcile_after is null or reconcile_after <= now()) \
            order by coalesce(reconcile_after, created_at) \
            limit $1 \
            for update skip locked) \
         returning id, status, provider_order_id, expires_at, reconcile_failures, \
                   (expires_at <= now()) as lapsed, now() as db_now",
    )
    .bind(BATCH)
    .bind(CLAIM_MINUTES)
    .bind(FIRST_CHECK_MINUTES)
    .fetch_all(db)
    .await
}

async fn look_again_in(db: &PgPool, order: Uuid, minutes: i32) -> Result<(), sqlx::Error> {
    sqlx::query(
        "update orders set reconcile_after = now() + make_interval(mins => $2), \
            reconcile_failures = 0 where id = $1",
    )
    .bind(order)
    .bind(minutes)
    .execute(db)
    .await?;
    Ok(())
}

async fn order_failed(db: &PgPool, order: &ClaimedOrder) -> Result<(), sqlx::Error> {
    sqlx::query(
        "update orders set reconcile_after = now() + make_interval(secs => $2), \
            reconcile_failures = reconcile_failures + 1 where id = $1",
    )
    .bind(order.id)
    .bind(backoff(order.reconcile_failures) as f64)
    .execute(db)
    .await?;
    Ok(())
}

async fn reconcile_orders(state: &AppState, report: &mut Report) -> Result<(), ApiError> {
    for order in claim_orders(&state.db).await? {
        report.orders_checked += 1;
        match reconcile_order(state, &order, report).await {
            Ok(()) => {}
            Err(error) => {
                report.order_errors += 1;
                tracing::warn!(order = %order.id, ?error, "could not reconcile an order; will retry");
                order_failed(&state.db, &order).await?;
            }
        }
    }
    Ok(())
}

async fn reconcile_order(
    state: &AppState,
    order: &ClaimedOrder,
    report: &mut Report,
) -> Result<(), ApiError> {
    // Never reached the provider and now too old to matter.
    let Some(provider_order_id) = order.provider_order_id.as_deref() else {
        if order.lapsed && orders::expire(&state.db, order.id).await? > 0 {
            report.orders_expired += 1;
        }
        return Ok(());
    };

    let remote = state
        .payments
        .fetch_order(provider_order_id)
        .await
        .map_err(provider_error)?;
    let paid = remote.status == ProviderOrderStatus::Paid || remote.amount_paid_paise > 0;
    if paid {
        let payments = state
            .payments
            .fetch_order_payments(provider_order_id)
            .await
            .map_err(provider_error)?;
        let mut settled_any = false;
        for payment in &payments {
            settled_any |= settle_payment(state, payment).await?;
        }
        if settled_any {
            report.orders_settled += 1;
        }
        look_again_in(&state.db, order.id, PENDING_RECHECK_MINUTES).await?;
        return Ok(());
    }

    match order.status {
        OrderStatus::Pending | OrderStatus::Created if order.lapsed => {
            let attempt_in_flight = remote.status == ProviderOrderStatus::Attempted
                && order.db_now < order.expires_at + ATTEMPT_GRACE;
            if attempt_in_flight {
                // The customer tried to pay as time ran out; give a slow payment a chance.
                look_again_in(&state.db, order.id, PENDING_RECHECK_MINUTES).await?;
            } else {
                if orders::expire(&state.db, order.id).await? > 0 {
                    report.orders_expired += 1;
                }
                look_again_in(&state.db, order.id, EXPIRED_RECHECK_MINUTES).await?;
            }
        }
        OrderStatus::Expired => {
            look_again_in(&state.db, order.id, EXPIRED_RECHECK_MINUTES).await?;
        }
        _ => look_again_in(&state.db, order.id, PENDING_RECHECK_MINUTES).await?,
    }
    Ok(())
}

fn provider_error(error: ProviderError) -> ApiError {
    crate::orders::service::provider_failure(error)
}

fn entity(payment: &ProviderPayment, status: &str) -> PaymentEntity {
    PaymentEntity {
        id: payment.id.clone(),
        order_id: payment.order_id.clone(),
        amount_paise: payment.amount_paise,
        currency: payment.currency.clone(),
        status: status.to_owned(),
        error_code: None,
        error_description: None,
    }
}

/// Applies what the provider says about one payment, if there is anything to apply. `true` if it
/// changed anything.
async fn settle_payment(state: &AppState, payment: &ProviderPayment) -> Result<bool, ApiError> {
    let changed = |received: Received| {
        matches!(received, Received::Handled(outcome)
            if !matches!(outcome, Outcome::AlreadyApplied | Outcome::Ignored | Outcome::AlreadyRefunded))
    };
    let payload = json!({
        "source": "reconcile",
        "payment": {"entity": {
            "id": payment.id, "order_id": payment.order_id, "amount": payment.amount_paise,
            "currency": payment.currency, "status": format!("{:?}", payment.status).to_lowercase()
        }}
    });
    match payment.status {
        ProviderPaymentStatus::Captured => {
            let event = Event::PaymentCaptured(entity(payment, "captured"));
            let received = handle(
                state,
                &format!("reconcile:payment:{}", payment.id),
                "payment.captured",
                Ok(event),
                payload,
            )
            .await?;
            Ok(changed(received))
        }
        // Paid and returned in full before we ever heard of it: record it as refunded, grant
        // nothing.
        ProviderPaymentStatus::Refunded
            if payment.amount_refunded_paise >= payment.amount_paise =>
        {
            let event = Event::RefundProcessed {
                refund: RefundEntity {
                    id: format!("reconciled-{}", payment.id),
                    payment_id: payment.id.clone(),
                    amount_paise: payment.amount_paise,
                    status: "processed".to_owned(),
                    receipt: None,
                },
                payment: Some(entity(payment, "refunded")),
            };
            let received = handle(
                state,
                &format!("reconcile:refunded:{}", payment.id),
                "refund.processed",
                Ok(event),
                payload,
            )
            .await?;
            Ok(changed(received))
        }
        _ => Ok(false),
    }
}

// ---- refunds ------------------------------------------------------------------------------------

#[derive(Debug, sqlx::FromRow)]
struct ClaimedRefund {
    id: Uuid,
    provider_refund_id: Option<String>,
    payment_id: Uuid,
    reconcile_failures: i32,
}

async fn claim_refunds(db: &PgPool) -> Result<Vec<ClaimedRefund>, sqlx::Error> {
    sqlx::query_as(
        "update refunds set reconcile_after = now() + make_interval(mins => $2) \
         where id in ( \
            select id from refunds \
            where status = 'processing' \
              and updated_at <= now() - make_interval( \
                    mins => case when provider_refund_id is null then 2 else 10 end) \
              and (reconcile_after is null or reconcile_after <= now()) \
            order by coalesce(reconcile_after, updated_at) \
            limit $1 \
            for update skip locked) \
         returning id, provider_refund_id, payment_id, reconcile_failures",
    )
    .bind(BATCH)
    .bind(CLAIM_MINUTES)
    .fetch_all(db)
    .await
}

async fn refund_look_again_in(db: &PgPool, id: Uuid, minutes: i32) -> Result<(), sqlx::Error> {
    sqlx::query(
        "update refunds set reconcile_after = now() + make_interval(mins => $2), \
            reconcile_failures = 0 where id = $1",
    )
    .bind(id)
    .bind(minutes)
    .execute(db)
    .await?;
    Ok(())
}

async fn refund_failed(db: &PgPool, refund: &ClaimedRefund) -> Result<(), sqlx::Error> {
    sqlx::query(
        "update refunds set reconcile_after = now() + make_interval(secs => $2), \
            reconcile_failures = reconcile_failures + 1 where id = $1",
    )
    .bind(refund.id)
    .bind(backoff(refund.reconcile_failures) as f64)
    .execute(db)
    .await?;
    Ok(())
}

async fn reconcile_refunds(state: &AppState, report: &mut Report) -> Result<(), ApiError> {
    for refund in claim_refunds(&state.db).await? {
        report.refunds_checked += 1;
        if let Err(error) = reconcile_refund(state, &refund, report).await {
            report.refund_errors += 1;
            tracing::warn!(refund = %refund.id, ?error, "could not reconcile a refund; will retry");
            refund_failed(&state.db, &refund).await?;
        }
    }
    Ok(())
}

async fn reconcile_refund(
    state: &AppState,
    refund: &ClaimedRefund,
    report: &mut Report,
) -> Result<(), ApiError> {
    let Some(provider_refund_id) = refund.provider_refund_id.as_deref() else {
        // We asked and never heard back. Ask again: the same idempotency key means the provider
        // answers with the refund it already made, if it made one.
        report.refunds_asked_again += 1;
        refund_service::drive_to_provider(state, refund.id).await?;
        return Ok(());
    };

    let remote = state
        .payments
        .fetch_refund(provider_refund_id)
        .await
        .map_err(provider_error)?;
    match remote.status {
        ProviderRefundStatus::Processed => {
            let payment_ref: String =
                sqlx::query_scalar("select provider_ref from payments where id = $1")
                    .bind(refund.payment_id)
                    .fetch_one(&state.db)
                    .await?;
            refund_service::settle_processed(
                state,
                provider_refund_id,
                &payment_ref,
                remote.amount_paise,
            )
            .await?;
            report.refunds_settled += 1;
        }
        ProviderRefundStatus::Failed => {
            if refunds::fail(
                &state.db,
                refund.id,
                "The payment provider could not complete the refund.",
            )
            .await?
            .is_some_and(|row| row.status == RefundStatus::Failed)
            {
                report.refunds_failed += 1;
            }
        }
        ProviderRefundStatus::Pending => {
            refund_look_again_in(&state.db, refund.id, PENDING_REFUND_RECHECK_MINUTES).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_to_an_hour_with_jitter() {
        for (failures, base) in [(0, 60.0), (1, 120.0), (3, 480.0), (6, 3600.0), (40, 3600.0)] {
            for _ in 0..50 {
                let wait = backoff(failures) as f64;
                assert!(
                    (base * 0.74..=base * 1.26).contains(&wait),
                    "{failures} failures gave {wait}s, expected about {base}s"
                );
            }
        }
        assert!(backoff(-5) >= 44, "a negative count is treated as none");
    }
}
