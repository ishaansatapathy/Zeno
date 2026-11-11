use axum::http::StatusCode;
use uuid::Uuid;
use zeno_core::model::{PayoutStatus, Role};

use crate::auth::extractor::AuthUser;
use crate::error::{ApiError, FieldError};
use crate::ledger::{post, repo as ledger};
use crate::outbox;
use crate::payouts::dto::Payment;
use crate::payouts::repo::{self, AdminPayoutRow, PayoutRow};
use crate::refunds::service::queue_mail;
use crate::state::AppState;

const EARNERS: &[Role] = &[Role::Creator, Role::Admin];

fn conflict(code: &'static str, message: impl Into<String>) -> ApiError {
    ApiError::Http {
        status: StatusCode::CONFLICT,
        code,
        message: message.into(),
    }
}

fn not_found() -> ApiError {
    ApiError::Http {
        status: StatusCode::NOT_FOUND,
        code: "not_found",
        message: "Payout not found.".into(),
    }
}

fn rupees(paise: i64) -> String {
    format!("Rs {}.{:02}", paise / 100, paise % 100)
}

// ---- the creator ---------------------------------------------------------------------------------

/// Asks for `amount_paise` to be paid out. The money leaves the creator's balance in the ledger in
/// the same transaction that records the request, so however many requests race, the total
/// reserved never exceeds what was available.
///
/// Two things guarantee that: the unique index allowing one open request per creator (the losers
/// of a race get `payout_in_progress`), and the row lock taken below, which makes the balance
/// check and the reservation one step. While the index exists the lock changes nothing anyone can
/// observe; it is what keeps the check sound if that rule is ever relaxed.
pub async fn request(
    state: &AppState,
    user: &AuthUser,
    amount_paise: i64,
) -> Result<PayoutRow, ApiError> {
    user.require_role(EARNERS)?;
    let minimum = state.config.ledger.payout_minimum_paise;
    if amount_paise < minimum {
        return Err(ApiError::validation(vec![FieldError::new(
            "amount_paise",
            format!("The smallest payout is {}.", rupees(minimum)),
        )]));
    }
    let mut tx = state.db.begin().await?;
    ledger::lock_creator(&mut tx, user.id).await?;
    if repo::open_for_creator(&mut *tx, user.id).await?.is_some() {
        return Err(conflict(
            "payout_in_progress",
            "You already have a payout waiting to be paid. Wait for it, or cancel it first.",
        ));
    }
    let available = ledger::position(&mut *tx, user.id).await?.available_paise();
    if amount_paise > available {
        return Err(conflict(
            "insufficient_funds",
            format!(
                "You can request up to {} now. Earnings inside a refund window are not payable yet.",
                rupees(available)
            ),
        ));
    }
    let payout = repo::insert(&mut tx, user.id, amount_paise)
        .await?
        .ok_or_else(|| conflict("payout_in_progress", "You already have a payout waiting."))?;
    post::payout(&mut tx, payout.id, user.id, amount_paise).await?;
    tx.commit().await?;
    Ok(payout)
}

/// The creator withdraws a request that has not been paid yet; the money returns to their balance.
pub async fn cancel_own(
    state: &AppState,
    user: &AuthUser,
    payout_id: Uuid,
) -> Result<PayoutRow, ApiError> {
    user.require_role(EARNERS)?;
    let mut tx = state.db.begin().await?;
    // Someone else's payout looks exactly like one that does not exist.
    let payout = repo::lock(&mut tx, payout_id)
        .await?
        .filter(|payout| payout.creator_id == user.id)
        .ok_or_else(not_found)?;
    let cancelled = repo::mark_cancelled(&mut tx, payout.id, None, user.id)
        .await?
        .ok_or_else(|| {
            conflict(
                "payout_not_open",
                "This payout has already been dealt with.",
            )
        })?;
    post::payout_reversal(&mut tx, payout.id, payout.creator_id, payout.amount_paise).await?;
    tx.commit().await?;
    Ok(cancelled)
}

pub async fn list_own(
    state: &AppState,
    user: &AuthUser,
    limit: Option<u32>,
) -> Result<Vec<PayoutRow>, ApiError> {
    user.require_role(EARNERS)?;
    let limit = i64::from(limit.unwrap_or(25).clamp(1, 100));
    Ok(repo::list_for_creator(&state.db, user.id, limit).await?)
}

// ---- an admin ------------------------------------------------------------------------------------

pub async fn list_all(
    state: &AppState,
    admin: &AuthUser,
    status: Option<PayoutStatus>,
    limit: Option<u32>,
) -> Result<Vec<AdminPayoutRow>, ApiError> {
    admin.require_role(&[Role::Admin])?;
    let limit = i64::from(limit.unwrap_or(50).clamp(1, 100));
    Ok(repo::list_all(&state.db, status, limit).await?)
}

/// The money has been sent. Records how, and the bank's reference, and tells the creator.
pub async fn pay(
    state: &AppState,
    admin: &AuthUser,
    payout_id: Uuid,
    payment: Payment,
) -> Result<PayoutRow, ApiError> {
    admin.require_role(&[Role::Admin])?;
    let mut tx = state.db.begin().await?;
    repo::lock(&mut tx, payout_id)
        .await?
        .ok_or_else(not_found)?;
    let paid = repo::mark_paid(
        &mut tx,
        payout_id,
        payment.method,
        &payment.reference,
        admin.id,
    )
    .await?
    .ok_or_else(|| {
        conflict(
            "payout_not_open",
            "This payout has already been dealt with.",
        )
    })?;
    let queued = queue_mail(
        &mut tx,
        paid.creator_id,
        "Your payout has been sent".to_owned(),
        format!(
            "We have paid out {} to you ({}).\nReference: {}",
            rupees(paid.amount_paise),
            payment.method,
            payment.reference
        ),
    )
    .await?;
    tx.commit().await?;
    if queued {
        outbox::kick(state);
    }
    Ok(paid)
}

/// An admin declines a request. The money returns to the creator's balance and they are told why.
pub async fn cancel(
    state: &AppState,
    admin: &AuthUser,
    payout_id: Uuid,
    reason: String,
) -> Result<PayoutRow, ApiError> {
    admin.require_role(&[Role::Admin])?;
    let mut tx = state.db.begin().await?;
    let payout = repo::lock(&mut tx, payout_id)
        .await?
        .ok_or_else(not_found)?;
    let cancelled = repo::mark_cancelled(&mut tx, payout_id, Some(&reason), admin.id)
        .await?
        .ok_or_else(|| {
            conflict(
                "payout_not_open",
                "This payout has already been dealt with.",
            )
        })?;
    post::payout_reversal(&mut tx, payout.id, payout.creator_id, payout.amount_paise).await?;
    let queued = queue_mail(
        &mut tx,
        payout.creator_id,
        "Your payout request was cancelled".to_owned(),
        format!(
            "Your request for {} was cancelled and the money is back in your balance.\nReason: {reason}",
            rupees(payout.amount_paise)
        ),
    )
    .await?;
    tx.commit().await?;
    if queued {
        outbox::kick(state);
    }
    Ok(cancelled)
}
