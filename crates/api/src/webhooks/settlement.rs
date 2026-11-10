//! What a payment event does to orders, payments and enrollments.
//!
//! These are plain functions over a connection, to be called inside a transaction the caller
//! owns. They hold the whole state machine in one place so that the webhook, and later the
//! reconciliation job, cannot disagree about it:
//!
//! * a payment is **trusted only after it matches its order**: same provider order, same amount,
//!   same currency. A mismatch enrolls nobody and is flagged for a person;
//! * **every step is idempotent**. Delivering the same payment twice, or `order.paid` and
//!   `payment.captured` for it, changes nothing the second time;
//! * **state never goes backwards**: a paid order is not failed by a late `payment.failed`, and a
//!   refunded payment is not re-enrolled by a late `payment.captured`;
//! * **money received is never lost sight of**: a second payment, a payment for access a creator
//!   blocked, or a payment that matches nothing is recorded and flagged, not dropped.

use serde_json::{json, Value};
use sqlx::PgConnection;
use uuid::Uuid;
use zeno_core::model::{EnrollmentSource, OrderStatus};

use crate::enrollments::repo as enrollments;
use crate::refunds::repo as refunds;
use crate::webhooks::event::{DisputeEntity, PaymentEntity, RefundEntity};
use crate::webhooks::repo::{self, LockedOrder, NewPayment, PaymentRow};

/// How an event ended. Stored with the event; the ones that need a person say so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Paid: the buyer now has the course.
    Enrolled,
    /// This payment was already applied (a redelivery, or `order.paid` after `payment.captured`).
    AlreadyApplied,
    /// Paid, but the buyer already had access by another route.
    PaidAlreadyEnrolled,
    /// Paid, but a creator or admin had blocked this person's access. Money without access.
    PaidAccessBlocked,
    /// A second successful payment for an order that was already paid.
    DuplicatePayment,
    /// The payment's amount is not the order's.
    AmountMismatch,
    CurrencyMismatch,
    /// A captured payment that belongs to no order of ours.
    UnknownOrder,
    /// The payment arrived after it had already been refunded.
    AlreadyRefunded,
    FailureRecorded,
    RefundApplied,
    /// A refund for part of a payment; Stage 1 does not take partial refunds.
    RefundPartial,
    /// The refund arrived before the payment; the payment is recorded as refunded, nothing granted.
    RefundBeforePayment,
    RefundFailed,
    /// A refund for a payment we cannot place.
    RefundUnknownPayment,
    /// A dispute with a customer's bank, noted against the order for a person to handle.
    DisputeNoted,
    /// An event we do not act on, or one that says nothing to act on.
    Ignored,
    /// An event we could not read.
    Malformed,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Enrolled => "enrolled",
            Self::AlreadyApplied => "already_applied",
            Self::PaidAlreadyEnrolled => "paid_already_enrolled",
            Self::PaidAccessBlocked => "paid_access_blocked",
            Self::DuplicatePayment => "duplicate_payment",
            Self::AmountMismatch => "amount_mismatch",
            Self::CurrencyMismatch => "currency_mismatch",
            Self::UnknownOrder => "unknown_order",
            Self::AlreadyRefunded => "already_refunded",
            Self::FailureRecorded => "failure_recorded",
            Self::RefundApplied => "refund_applied",
            Self::RefundPartial => "refund_partial",
            Self::RefundBeforePayment => "refund_before_payment",
            Self::RefundFailed => "refund_failed",
            Self::RefundUnknownPayment => "refund_unknown_payment",
            Self::DisputeNoted => "dispute_noted",
            Self::Ignored => "ignored",
            Self::Malformed => "malformed",
        }
    }

    /// Money moved in a way a person must look at.
    pub fn needs_review(self) -> bool {
        matches!(
            self,
            Self::PaidAlreadyEnrolled
                | Self::PaidAccessBlocked
                | Self::DuplicatePayment
                | Self::AmountMismatch
                | Self::CurrencyMismatch
                | Self::UnknownOrder
                | Self::RefundPartial
                | Self::RefundFailed
                | Self::RefundUnknownPayment
                | Self::DisputeNoted
        )
    }
}

/// What to tell the buyer once the transaction has committed.
#[derive(Debug, Clone)]
pub struct Receipt {
    pub user_id: Uuid,
    pub order_id: Uuid,
    pub course_title: String,
    pub amount_paise: i64,
}

/// What to tell the buyer when their refund has gone through.
#[derive(Debug, Clone)]
pub struct RefundNotice {
    pub user_id: Uuid,
    pub order_id: Uuid,
    pub course_title: String,
    pub amount_paise: i64,
}

#[derive(Debug, Clone)]
pub struct Settled {
    pub outcome: Outcome,
    pub order_id: Option<Uuid>,
    pub receipt: Option<Receipt>,
    pub refund_notice: Option<RefundNotice>,
}

pub(crate) fn settled(outcome: Outcome, order: Option<&LockedOrder>) -> Settled {
    Settled {
        outcome,
        order_id: order.map(|o| o.id),
        receipt: None,
        refund_notice: None,
    }
}

fn payment_facts(p: &PaymentEntity) -> Value {
    json!({
        "payment": p.id,
        "amount_paise": p.amount_paise,
        "currency": p.currency,
        "error_code": p.error_code,
    })
}

/// A payment the provider reports as captured.
pub async fn apply_captured(
    conn: &mut PgConnection,
    provider: &str,
    payment: &PaymentEntity,
    raw: &Value,
) -> Result<Settled, sqlx::Error> {
    if payment.status != "captured" {
        return Ok(settled(Outcome::Ignored, None));
    }
    let Some(provider_order_id) = payment.order_id.as_deref() else {
        tracing::error!(alert = "unknown_order", payment = %payment.id, "captured payment with no order");
        return Ok(settled(Outcome::UnknownOrder, None));
    };
    let Some(order) = repo::lock_order_by_provider_id(conn, provider, provider_order_id).await?
    else {
        tracing::error!(alert = "unknown_order", payment = %payment.id, "captured payment for an order we never made");
        return Ok(settled(Outcome::UnknownOrder, None));
    };

    // The payload is the provider's word about the payment; our order is our word about what was
    // asked. They must agree before anything is granted.
    if payment.amount_paise != order.amount_paise {
        tracing::error!(
            alert = "amount_mismatch", order = %order.id, payment = %payment.id,
            expected_paise = order.amount_paise, got_paise = payment.amount_paise,
            "payment amount differs from the order"
        );
        repo::add_order_event(
            conn,
            order.id,
            "payment_mismatch",
            None,
            json!({"expected_paise": order.amount_paise, "got": payment_facts(payment)}),
        )
        .await?;
        return Ok(settled(Outcome::AmountMismatch, Some(&order)));
    }
    if payment.currency != order.currency {
        tracing::error!(
            alert = "currency_mismatch", order = %order.id, payment = %payment.id,
            "payment currency differs from the order"
        );
        repo::add_order_event(
            conn,
            order.id,
            "payment_mismatch",
            None,
            json!({"expected_currency": order.currency, "got": payment_facts(payment)}),
        )
        .await?;
        return Ok(settled(Outcome::CurrencyMismatch, Some(&order)));
    }

    let existing = repo::payment_by_ref(conn, &payment.id, true).await?;
    match existing.as_ref().map(|row| row.status.as_str()) {
        Some("confirmed" | "duplicate") => {
            return Ok(settled(Outcome::AlreadyApplied, Some(&order)));
        }
        Some("refunded") => return Ok(settled(Outcome::AlreadyRefunded, Some(&order))),
        _ => {}
    }

    // Another payment already settled this order: this one is money we should not have taken.
    let already_settled = matches!(order.status, OrderStatus::Paid | OrderStatus::Refunded);
    let status = if already_settled {
        "duplicate"
    } else {
        "confirmed"
    };
    let payment_row = match existing {
        Some(row) => {
            // A failed attempt that turned out to have gone through.
            repo::set_payment_status(conn, row.id, status, Some(raw)).await?;
            row.id
        }
        None => {
            repo::insert_payment(
                conn,
                &NewPayment {
                    provider,
                    provider_ref: &payment.id,
                    order_id: order.id,
                    amount_paise: payment.amount_paise,
                    currency: &payment.currency,
                    status,
                    raw_event: raw,
                },
            )
            .await?
        }
    };

    if already_settled {
        tracing::error!(alert = "duplicate_payment", order = %order.id, payment = %payment.id, "order paid twice");
        repo::add_order_event(
            conn,
            order.id,
            "duplicate_payment",
            Some(payment_row),
            payment_facts(payment),
        )
        .await?;
        return Ok(settled(Outcome::DuplicatePayment, Some(&order)));
    }

    repo::set_order_status(conn, order.id, OrderStatus::Paid).await?;
    repo::add_order_event(
        conn,
        order.id,
        "paid",
        Some(payment_row),
        payment_facts(payment),
    )
    .await?;
    if order.status == OrderStatus::Expired && repo::reclaim_coupon(conn, order.id).await? {
        repo::add_order_event(
            conn,
            order.id,
            "coupon_reclaimed",
            Some(payment_row),
            json!({"reason": "paid after the order expired"}),
        )
        .await?;
    }

    let outcome = match grant(conn, &order, payment_row, &payment.id).await? {
        Grant::Enrolled => Outcome::Enrolled,
        Grant::AlreadyLive => Outcome::PaidAlreadyEnrolled,
        Grant::Blocked => {
            tracing::error!(alert = "paid_access_blocked", order = %order.id, payment = %payment.id, "paid for access that was blocked");
            repo::add_order_event(
                conn,
                order.id,
                "access_blocked",
                Some(payment_row),
                json!({"reason": "a creator or admin revoked this person's access"}),
            )
            .await?;
            Outcome::PaidAccessBlocked
        }
    };
    let receipt = (outcome != Outcome::PaidAccessBlocked).then(|| Receipt {
        user_id: order.user_id,
        order_id: order.id,
        course_title: order.price_snapshot["course_title"]
            .as_str()
            .unwrap_or("your course")
            .to_owned(),
        amount_paise: order.amount_paise,
    });
    Ok(Settled {
        outcome,
        order_id: Some(order.id),
        receipt,
        refund_notice: None,
    })
}

enum Grant {
    Enrolled,
    AlreadyLive,
    Blocked,
}

/// Gives the buyer the course, whatever state any earlier enrollment is in.
async fn grant(
    conn: &mut PgConnection,
    order: &LockedOrder,
    payment_row: Uuid,
    payment_ref: &str,
) -> Result<Grant, sqlx::Error> {
    let reason = format!("Payment {payment_ref}");
    // Two tries: the second only if another transaction created the row between our look and
    // our insert.
    for _ in 0..2 {
        match repo::enrollment_for_update(conn, order.user_id, order.course_id).await? {
            None => {
                if let Some(id) = repo::insert_payment_enrollment(
                    conn,
                    order.user_id,
                    order.course_id,
                    payment_row,
                )
                .await?
                {
                    enrollments::record_event(
                        conn,
                        id,
                        None,
                        "created",
                        EnrollmentSource::Payment,
                        Some(&reason),
                    )
                    .await?;
                    return Ok(Grant::Enrolled);
                }
            }
            Some(current) => {
                let ended_by_money_or_time = current.revoked
                    && matches!(
                        current.revocation_kind.as_deref(),
                        Some("payment" | "system")
                    );
                if ended_by_money_or_time || (!current.revoked && current.expired) {
                    repo::reactivate_with_payment(conn, current.id, payment_row).await?;
                    enrollments::record_event(
                        conn,
                        current.id,
                        None,
                        "reactivated",
                        EnrollmentSource::Payment,
                        Some(&reason),
                    )
                    .await?;
                    return Ok(Grant::Enrolled);
                }
                if current.revoked {
                    // A creator or admin took this person's access away. Paying does not undo it.
                    return Ok(Grant::Blocked);
                }
                repo::link_payment(conn, current.id, payment_row).await?;
                return Ok(Grant::AlreadyLive);
            }
        }
    }
    Err(sqlx::Error::Protocol(
        "enrollment kept appearing and disappearing".into(),
    ))
}

/// A payment attempt that failed. The order stays open: the buyer may try again.
pub async fn apply_failed(
    conn: &mut PgConnection,
    provider: &str,
    payment: &PaymentEntity,
    raw: &Value,
) -> Result<Settled, sqlx::Error> {
    let Some(provider_order_id) = payment.order_id.as_deref() else {
        return Ok(settled(Outcome::Ignored, None));
    };
    let Some(order) = repo::lock_order_by_provider_id(conn, provider, provider_order_id).await?
    else {
        return Ok(settled(Outcome::Ignored, None));
    };
    let existing = repo::payment_by_ref(conn, &payment.id, true).await?;
    let payment_row = match existing {
        // Never move a payment that went through, or one already refunded, back to failed.
        Some(row) if row.status != "failed" && row.status != "pending" => {
            return Ok(settled(Outcome::Ignored, Some(&order)));
        }
        Some(row) => {
            repo::set_payment_status(conn, row.id, "failed", Some(raw)).await?;
            row.id
        }
        None => {
            repo::insert_payment(
                conn,
                &NewPayment {
                    provider,
                    provider_ref: &payment.id,
                    order_id: order.id,
                    amount_paise: payment.amount_paise,
                    currency: &payment.currency,
                    status: "failed",
                    raw_event: raw,
                },
            )
            .await?
        }
    };
    repo::add_order_event(
        conn,
        order.id,
        "payment_failed",
        Some(payment_row),
        payment_facts(payment),
    )
    .await?;
    Ok(settled(Outcome::FailureRecorded, Some(&order)))
}

/// A refund the provider has completed (whether we asked for it or it was made in their
/// dashboard). Once the whole payment has been returned, the access it bought ends.
pub async fn apply_refund_processed(
    conn: &mut PgConnection,
    provider: &str,
    refund: &RefundEntity,
    payment: Option<&PaymentEntity>,
    raw: &Value,
) -> Result<Settled, sqlx::Error> {
    if refund.status != "processed" {
        return Ok(settled(Outcome::Ignored, None));
    }
    // Find the order first (and lock it) so refunds take turns with payments for the same order.
    let known = repo::payment_by_ref(conn, &refund.payment_id, false).await?;
    let order = match (&known, payment.and_then(|p| p.order_id.as_deref())) {
        (
            Some(PaymentRow {
                order_id: Some(order_id),
                ..
            }),
            _,
        ) => repo::lock_order_by_id(conn, *order_id).await?,
        (_, Some(provider_order_id)) => {
            repo::lock_order_by_provider_id(conn, provider, provider_order_id).await?
        }
        _ => None,
    };
    let Some(order) = order else {
        tracing::error!(alert = "refund_unknown_payment", refund = %refund.id, "refund for a payment we cannot place");
        return Ok(settled(Outcome::RefundUnknownPayment, None));
    };

    match repo::payment_by_ref(conn, &refund.payment_id, true).await? {
        Some(row) => {
            if row.status == "refunded" {
                return Ok(settled(Outcome::AlreadyRefunded, Some(&order)));
            }
            // Record it. `None` means this very refund was recorded before.
            let Some(returned) = refunds::record_success(
                conn,
                order.id,
                row.id,
                refund.amount_paise,
                &refund.id,
                refund
                    .receipt
                    .as_deref()
                    .and_then(|id| Uuid::parse_str(id).ok()),
            )
            .await?
            else {
                return Ok(settled(Outcome::AlreadyRefunded, Some(&order)));
            };
            if returned < row.amount_paise {
                repo::add_order_event(
                    conn,
                    order.id,
                    "refund_partial",
                    Some(row.id),
                    json!({"refund": refund.id, "refund_paise": refund.amount_paise, "returned_paise": returned, "payment_paise": row.amount_paise}),
                )
                .await?;
                return Ok(settled(Outcome::RefundPartial, Some(&order)));
            }
            let settled_this_order = row.status == "confirmed";
            repo::set_payment_status(conn, row.id, "refunded", Some(raw)).await?;
            repo::add_order_event(
                conn,
                order.id,
                "refunded",
                Some(row.id),
                json!({"refund": refund.id, "refund_paise": refund.amount_paise, "returned_paise": returned}),
            )
            .await?;
            let mut result = settled(Outcome::RefundApplied, Some(&order));
            if settled_this_order {
                repo::set_order_status(conn, order.id, OrderStatus::Refunded).await?;
                let revoked = repo::revoke_for_payment(conn, row.id, "Refunded.").await?;
                for enrollment in &revoked {
                    enrollments::record_event(
                        conn,
                        enrollment.id,
                        None,
                        "revoked",
                        enrollment.source,
                        Some("Refunded."),
                    )
                    .await?;
                }
                if let Some(first) = revoked.first() {
                    enrollments::end_sessions_without_access(conn, first.user_id).await?;
                }
                result.refund_notice = Some(RefundNotice {
                    user_id: order.user_id,
                    order_id: order.id,
                    course_title: order.price_snapshot["course_title"]
                        .as_str()
                        .unwrap_or("your course")
                        .to_owned(),
                    amount_paise: returned,
                });
            }
            Ok(result)
        }
        None => {
            // The refund overtook the payment. Record the payment as refunded so that when its own
            // event arrives it grants nothing.
            let Some(entity) = payment else {
                return Ok(settled(Outcome::RefundUnknownPayment, Some(&order)));
            };
            if entity.amount_paise != order.amount_paise
                || refund.amount_paise != entity.amount_paise
                || entity.currency != order.currency
            {
                return Ok(settled(Outcome::RefundUnknownPayment, Some(&order)));
            }
            let payment_row = repo::insert_payment(
                conn,
                &NewPayment {
                    provider,
                    provider_ref: &entity.id,
                    order_id: order.id,
                    amount_paise: entity.amount_paise,
                    currency: &entity.currency,
                    status: "refunded",
                    raw_event: raw,
                },
            )
            .await?;
            refunds::record_success(
                conn,
                order.id,
                payment_row,
                refund.amount_paise,
                &refund.id,
                refund
                    .receipt
                    .as_deref()
                    .and_then(|id| Uuid::parse_str(id).ok()),
            )
            .await?;
            if !matches!(order.status, OrderStatus::Paid) {
                repo::set_order_status(conn, order.id, OrderStatus::Refunded).await?;
            }
            repo::add_order_event(
                conn,
                order.id,
                "refunded",
                Some(payment_row),
                json!({"refund": refund.id, "before_payment": true}),
            )
            .await?;
            Ok(settled(Outcome::RefundBeforePayment, Some(&order)))
        }
    }
}

/// A refund the provider could not complete. Our own refund (if it was ours) is marked failed
/// with a note, the order stays paid, and a person is told.
pub async fn note_refund_failed(
    conn: &mut PgConnection,
    refund: &RefundEntity,
) -> Result<Settled, sqlx::Error> {
    tracing::error!(alert = "refund_failed", refund = %refund.id, payment = %refund.payment_id, "provider could not complete a refund");
    let mut order_id = None;
    if let Some(row) = repo::payment_by_ref(conn, &refund.payment_id, false).await? {
        refunds::fail_reported(
            conn,
            &refund.id,
            row.id,
            "The payment provider could not complete the refund.",
            refund
                .receipt
                .as_deref()
                .and_then(|id| Uuid::parse_str(id).ok()),
        )
        .await?;
        if let Some(id) = row.order_id {
            repo::add_order_event(
                conn,
                id,
                "refund_failed",
                Some(row.id),
                json!({"refund": refund.id, "refund_paise": refund.amount_paise}),
            )
            .await?;
            order_id = Some(id);
        }
    }
    Ok(Settled {
        outcome: Outcome::RefundFailed,
        order_id,
        receipt: None,
        refund_notice: None,
    })
}

/// A customer disputed a payment with their bank. Nothing is revoked or refunded automatically:
/// a dispute is a negotiation, and which way it goes is a person's call. It is recorded against
/// the order, and the event is flagged so it is not missed.
pub async fn note_dispute(
    conn: &mut PgConnection,
    event_name: &str,
    dispute: &DisputeEntity,
) -> Result<Settled, sqlx::Error> {
    tracing::error!(alert = "dispute", dispute = %dispute.id, payment = %dispute.payment_id, event = event_name, status = %dispute.status, "payment dispute");
    let mut order_id = None;
    if let Some(row) = repo::payment_by_ref(conn, &dispute.payment_id, false).await? {
        if let Some(id) = row.order_id {
            repo::add_order_event(
                conn,
                id,
                "dispute",
                Some(row.id),
                json!({
                    "event": event_name,
                    "dispute": dispute.id,
                    "status": dispute.status,
                    "amount_paise": dispute.amount_paise,
                }),
            )
            .await?;
            order_id = Some(id);
        }
    }
    Ok(Settled {
        outcome: Outcome::DisputeNoted,
        order_id,
        receipt: None,
        refund_notice: None,
    })
}
