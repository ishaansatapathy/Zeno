use axum::http::StatusCode;
use chrono::Duration;
use serde_json::json;
use sqlx::PgConnection;
use uuid::Uuid;
use zeno_core::model::{OrderStatus, RefundStatus, Role};
use zeno_core::payments::{ProviderError, ProviderRefundStatus, RefundRequest};

use crate::auth::extractor::AuthUser;
use crate::error::ApiError;
use crate::mail::OutgoingEmail;
use crate::orders::dto::{PriceSnapshot, DEFAULT_REFUND_WINDOW_DAYS};
use crate::orders::service::provider_failure;
use crate::outbox;
use crate::refunds::repo::{self, ConfirmedPayment, NewRefund, OrderContext, RefundRow};
use crate::state::AppState;
use crate::webhooks::event::{Event, RefundEntity};
use crate::webhooks::service::handle;

const STAFF: &[Role] = &[Role::Creator, Role::Admin];
const NOTICE_TTL_DAYS: i64 = 7;

fn conflict(code: &'static str, message: &'static str) -> ApiError {
    ApiError::Http {
        status: StatusCode::CONFLICT,
        code,
        message: message.into(),
    }
}

fn not_found(subject: &'static str) -> ApiError {
    ApiError::Http {
        status: StatusCode::NOT_FOUND,
        code: "not_found",
        message: format!("{subject} not found."),
    }
}

fn is_admin(user: &AuthUser) -> bool {
    user.role == Role::Admin
}

/// Queues an email to a user inside the caller's transaction. A person with no email address
/// (phone-only accounts) simply gets none.
pub async fn queue_mail(
    conn: &mut PgConnection,
    user_id: Uuid,
    subject: String,
    text: String,
) -> Result<bool, sqlx::Error> {
    let Some(to) = repo::email_of(&mut *conn, user_id).await? else {
        return Ok(false);
    };
    outbox::enqueue_in(
        conn,
        OutgoingEmail { to, subject, text },
        Duration::days(NOTICE_TTL_DAYS),
    )
    .await?;
    Ok(true)
}

/// The order for someone who runs the course: its creator, or an admin. Everyone else, other
/// creators included, is told it does not exist; students are turned away by role first.
async fn staff_order(
    state: &AppState,
    actor: &AuthUser,
    order_id: Uuid,
) -> Result<OrderContext, ApiError> {
    actor.require_role(STAFF)?;
    let context = repo::order_context(&state.db, order_id)
        .await?
        .ok_or_else(|| not_found("Order"))?;
    if !is_admin(actor) && context.creator_id != actor.id {
        return Err(not_found("Order"));
    }
    Ok(context)
}

fn snapshot_of(context: &OrderContext) -> Result<PriceSnapshot, ApiError> {
    Ok(serde_json::from_value(context.price_snapshot.clone()).map_err(anyhow::Error::from)?)
}

/// What can still be returned on the order's payment, or why nothing can.
async fn refundable(
    conn: &mut PgConnection,
    context: &OrderContext,
) -> Result<(ConfirmedPayment, i64), ApiError> {
    match context.status {
        OrderStatus::Refunded => {
            return Err(conflict(
                "already_refunded",
                "This order has already been refunded.",
            ))
        }
        OrderStatus::Paid => {}
        _ => {
            return Err(conflict(
                "not_refundable",
                "Only a paid order can be refunded.",
            ))
        }
    }
    let payment = repo::confirmed_payment(conn, context.id)
        .await?
        .ok_or_else(|| conflict("not_refundable", "Only a paid order can be refunded."))?;
    let remaining = payment.amount_paise - payment.refunded_paise;
    if remaining <= 0 {
        return Err(conflict(
            "already_refunded",
            "This order has already been refunded.",
        ));
    }
    Ok((payment, remaining))
}

// ---- a student asks ---------------------------------------------------------------------------

/// A student asks for their money back, inside the window their course promised when they paid.
/// Returns the request, and whether it was just made (asking again returns the same one).
pub async fn request(
    state: &AppState,
    user: &AuthUser,
    order_id: Uuid,
    reason: Option<String>,
) -> Result<(RefundRow, bool), ApiError> {
    // Someone else's order looks exactly like one that does not exist.
    let mine = repo::order_context(&state.db, order_id)
        .await?
        .filter(|context| context.user_id == user.id)
        .ok_or_else(|| not_found("Order"))?;

    let mut tx = state.db.begin().await?;
    let context = repo::lock_order_context(&mut tx, mine.id)
        .await?
        .ok_or_else(|| not_found("Order"))?;
    let (payment, remaining) = refundable(&mut tx, &context).await?;

    let window_days = snapshot_of(&context)
        .map(|snapshot| snapshot.refund_window_days)
        .unwrap_or(DEFAULT_REFUND_WINDOW_DAYS);
    if window_days <= 0 {
        return Err(conflict(
            "refund_not_available",
            "This course does not offer refunds.",
        ));
    }
    let paid_at = context.paid_at.unwrap_or(context.db_now);
    if context.db_now > paid_at + Duration::days(i64::from(window_days)) {
        return Err(conflict(
            "refund_window_closed",
            "The refund window for this course has closed.",
        ));
    }

    if let Some(active) = repo::active_for_order(&mut tx, context.id).await? {
        return if active.status == RefundStatus::Requested {
            Ok((active, false))
        } else {
            Err(conflict(
                "refund_in_progress",
                "A refund for this order is already being made.",
            ))
        };
    }
    let refund = repo::insert(
        &mut tx,
        &NewRefund {
            order_id: context.id,
            payment_id: payment.id,
            amount_paise: remaining,
            status: RefundStatus::Requested,
            origin: "student",
            requested_by: Some(user.id),
            decided_by: None,
            reason: reason.as_deref(),
        },
    )
    .await?
    .ok_or_else(|| {
        conflict(
            "refund_in_progress",
            "A refund for this order is already in progress.",
        )
    })?;
    let queued = queue_mail(
        &mut tx,
        context.creator_id,
        format!("Refund requested for {}", context.course_title),
        format!(
            "A student asked for a refund of \"{}\".\nOrder: {}\nReason: {}\n\nOpen {}/dashboard to approve or decline.",
            context.course_title,
            context.id,
            reason.as_deref().unwrap_or("not given"),
            state.config.client_origin
        ),
    )
    .await?;
    tx.commit().await?;
    if queued {
        outbox::kick(state);
    }
    Ok((refund, true))
}

// ---- a creator or admin decides ---------------------------------------------------------------

/// Approves the student's request, or refunds the order outright, and hands it to the provider.
/// Asking again while it is being made returns it (and, if the provider never answered, asks
/// again with the same idempotency key, which cannot refund twice).
pub async fn approve(
    state: &AppState,
    actor: &AuthUser,
    order_id: Uuid,
    reason: Option<String>,
) -> Result<RefundRow, ApiError> {
    staff_order(state, actor, order_id).await?;

    let mut tx = state.db.begin().await?;
    let context = repo::lock_order_context(&mut tx, order_id)
        .await?
        .ok_or_else(|| not_found("Order"))?;
    // The refund in flight is judged before the order: once the provider confirms, the order
    // reads as refunded, and asking again should still find the refund.
    let active = repo::active_for_order(&mut tx, context.id).await?;
    let (refund, moved) = match active {
        Some(active) if active.status == RefundStatus::Processing => (active, false),
        Some(request) => {
            refundable(&mut tx, &context).await?;
            let approved =
                repo::approve_request(&mut tx, request.id, actor.id, reason.as_deref()).await?;
            (approved, true)
        }
        None => {
            let (payment, remaining) = refundable(&mut tx, &context).await?;
            let created = repo::insert(
                &mut tx,
                &NewRefund {
                    order_id: context.id,
                    payment_id: payment.id,
                    amount_paise: remaining,
                    status: RefundStatus::Processing,
                    origin: if is_admin(actor) { "admin" } else { "creator" },
                    requested_by: None,
                    decided_by: Some(actor.id),
                    reason: reason.as_deref(),
                },
            )
            .await?
            .ok_or_else(|| {
                conflict(
                    "refund_in_progress",
                    "A refund for this order is already in progress.",
                )
            })?;
            (created, true)
        }
    };
    let queued = if moved {
        queue_mail(
            &mut tx,
            context.user_id,
            format!("Your refund for {} is on its way", context.course_title),
            format!(
                "Your refund of Rs {}.{:02} for \"{}\" was approved. It reaches your account when your bank completes it.\nOrder: {}",
                refund.amount_paise / 100,
                refund.amount_paise % 100,
                context.course_title,
                context.id
            ),
        )
        .await?
    } else {
        false
    };
    tx.commit().await?;
    if queued {
        outbox::kick(state);
    }

    match drive_to_provider(state, refund.id).await? {
        Driven::Failed(failed) => Err(ApiError::Http {
            status: StatusCode::BAD_GATEWAY,
            code: "refund_failed",
            message: format!(
                "The payment provider could not make this refund. {}",
                failed.decision_note.unwrap_or_default()
            ),
        }),
        Driven::Processing(row) | Driven::Done(row) => Ok(row),
    }
}

pub async fn decline(
    state: &AppState,
    actor: &AuthUser,
    order_id: Uuid,
    note: String,
) -> Result<RefundRow, ApiError> {
    staff_order(state, actor, order_id).await?;
    let mut tx = state.db.begin().await?;
    let context = repo::lock_order_context(&mut tx, order_id)
        .await?
        .ok_or_else(|| not_found("Order"))?;
    let request = repo::active_for_order(&mut tx, context.id)
        .await?
        .filter(|active| active.status == RefundStatus::Requested)
        .ok_or_else(|| {
            conflict(
                "no_refund_request",
                "There is no refund request waiting for an answer on this order.",
            )
        })?;
    let declined = repo::decline(&mut tx, request.id, actor.id, &note).await?;
    let queued = queue_mail(
        &mut tx,
        context.user_id,
        format!("Your refund request for {}", context.course_title),
        format!(
            "Your refund request for \"{}\" was declined.\nReason: {}\nOrder: {}",
            context.course_title, note, context.id
        ),
    )
    .await?;
    tx.commit().await?;
    if queued {
        outbox::kick(state);
    }
    Ok(declined)
}

pub async fn list_for_course(
    state: &AppState,
    actor: &AuthUser,
    course_id: Uuid,
    status: Option<RefundStatus>,
    limit: Option<u32>,
) -> Result<Vec<repo::StaffRefundRow>, ApiError> {
    actor.require_role(STAFF)?;
    let owner = repo::course_owner(&state.db, course_id)
        .await?
        .ok_or_else(|| not_found("Course"))?;
    if !is_admin(actor) && owner != actor.id {
        return Err(not_found("Course"));
    }
    let limit = i64::from(limit.unwrap_or(25).clamp(1, 50));
    Ok(repo::list_for_course(&state.db, course_id, status, limit).await?)
}

// ---- the provider -------------------------------------------------------------------------------

pub enum Driven {
    /// With the provider (or not yet answered); final when the provider confirms.
    Processing(RefundRow),
    /// The provider has made it and the books are settled.
    Done(RefundRow),
    /// The provider refused it. The order stays paid and the refund says why.
    Failed(RefundRow),
}

/// Asks the provider to make a refund that is in the `processing` state. Safe to call again for
/// the same refund at any time: our refund id is the provider's idempotency key, so the provider
/// returns the refund it already made instead of making another.
pub async fn drive_to_provider(state: &AppState, refund_id: Uuid) -> Result<Driven, ApiError> {
    let Some(refund) = repo::by_id(&state.db, refund_id).await? else {
        return Err(not_found("Refund"));
    };
    if refund.status != RefundStatus::Processing {
        return Ok(Driven::Done(refund));
    }
    let provider_ref: String =
        sqlx::query_scalar("select provider_ref from payments where id = $1")
            .bind(refund.payment_id)
            .fetch_one(&state.db)
            .await?;
    repo::count_attempt(&state.db, refund.id).await?;

    let request = RefundRequest {
        payment_id: provider_ref.clone(),
        amount_paise: refund.amount_paise,
        idempotency_key: refund.id.to_string(),
        notes: [("zeno_refund_id".to_owned(), refund.id.to_string())].into(),
    };
    let made = match state.payments.refund(&request).await {
        Ok(made) => made,
        Err(ProviderError::Rejected { code, .. }) => {
            let note = format!("The payment provider refused the refund ({code}).");
            return fail(state, refund.id, &note).await;
        }
        Err(ProviderError::NotFound) => {
            return fail(
                state,
                refund.id,
                "The payment was not found at the payment provider.",
            )
            .await;
        }
        // Anything that might clear up: leave it processing. Asking again is safe.
        Err(other) => return Err(provider_failure(other)),
    };

    repo::set_provider_refund(&state.db, refund.id, &made.id).await?;
    match made.status {
        ProviderRefundStatus::Pending => {
            let row = repo::by_id(&state.db, refund.id).await?.unwrap_or(refund);
            Ok(Driven::Processing(row))
        }
        ProviderRefundStatus::Failed => {
            fail(
                state,
                refund.id,
                "The payment provider could not complete the refund.",
            )
            .await
        }
        ProviderRefundStatus::Processed => {
            settle_processed(state, &made.id, &provider_ref, made.amount_paise).await?;
            let row = repo::by_id(&state.db, refund.id).await?.unwrap_or(refund);
            Ok(Driven::Done(row))
        }
    }
}

async fn fail(state: &AppState, refund_id: Uuid, note: &str) -> Result<Driven, ApiError> {
    tracing::error!(refund = %refund_id, note, "refund failed at the provider");
    let row = match repo::fail(&state.db, refund_id, note).await? {
        Some(row) => row,
        None => repo::by_id(&state.db, refund_id)
            .await?
            .ok_or_else(|| not_found("Refund"))?,
    };
    Ok(Driven::Failed(row))
}

/// The provider has completed a refund: settle it exactly as a `refund.processed` webhook would,
/// through the same code. The event id makes doing it twice (here, then reconciliation, then the
/// real webhook) harmless.
pub async fn settle_processed(
    state: &AppState,
    provider_refund_id: &str,
    provider_payment_id: &str,
    amount_paise: i64,
) -> Result<(), ApiError> {
    let entity = RefundEntity {
        id: provider_refund_id.to_owned(),
        payment_id: provider_payment_id.to_owned(),
        amount_paise,
        status: "processed".to_owned(),
        receipt: None,
    };
    let payload = json!({
        "source": "zeno",
        "refund": {"entity": {
            "id": provider_refund_id, "payment_id": provider_payment_id,
            "amount": amount_paise, "status": "processed"
        }}
    });
    handle(
        state,
        &format!("sync:refund:{provider_refund_id}"),
        "refund.processed",
        Ok(Event::RefundProcessed {
            refund: entity,
            payment: None,
        }),
        payload,
    )
    .await?;
    Ok(())
}
