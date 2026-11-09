use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use axum::http::StatusCode;
use uuid::Uuid;
use zeno_core::model::{CourseStatus, OrderStatus};
use zeno_core::payments::{CreateOrder, ProviderError, ProviderOrder, ProviderOrderStatus};

use crate::auth::extractor::AuthUser;
use crate::error::ApiError;
use crate::orders::dto::{
    CheckoutParams, OrderCourse, OrderEnvelope, OrderResponse, PriceSnapshot,
};
use crate::orders::repo::{self, CourseForCheckout, NewOrder, OrderRow};
use crate::state::AppState;

/// How long a request waits for another request that is already creating the same order at the
/// provider, before telling the client to try again.
const PEER_WAIT: Duration = Duration::from_secs(4);
const PEER_POLL: Duration = Duration::from_millis(75);

pub struct Checkout {
    pub order: OrderRow,
    /// False when this call returned an order that already existed.
    pub created: bool,
}

fn conflict(code: &'static str, message: impl Into<String>) -> ApiError {
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

/// How a provider failure reaches the client. Anything that might clear up by itself is a `503`
/// ("try again"); a refusal that will not is our problem, reported as a `502` without the
/// provider's wording.
pub fn provider_failure(error: ProviderError) -> ApiError {
    match error {
        ProviderError::Timeout
        | ProviderError::Unavailable(_)
        | ProviderError::Unreachable(_)
        | ProviderError::RateLimited => {
            tracing::warn!(%error, "payment provider unavailable");
            ApiError::Unavailable("Payments are temporarily unavailable. Please try again.")
        }
        ProviderError::Unauthorized => {
            tracing::error!("the payment provider refused our credentials");
            ApiError::Unavailable("Payments are temporarily unavailable. Please try again.")
        }
        ProviderError::Rejected { .. } | ProviderError::NotFound | ProviderError::Malformed(_) => {
            tracing::error!(%error, "the payment provider gave an unusable answer");
            ApiError::Http {
                status: StatusCode::BAD_GATEWAY,
                code: "payment_provider_error",
                message: "The payment provider could not create this order.".into(),
            }
        }
    }
}

pub async fn checkout(
    state: &AppState,
    user: &AuthUser,
    course_id: Uuid,
    idempotency_key: Option<&str>,
) -> Result<Checkout, ApiError> {
    // A repeated request with the same key gets the order it made the first time, whatever has
    // happened to that order since (paid, expired). This comes before every other check.
    if let Some(key) = idempotency_key {
        if let Some(existing) = repo::by_key(&state.db, user.id, key).await? {
            if existing.course_id != course_id {
                return Err(key_reused());
            }
            return Ok(Checkout {
                order: settle(state, existing).await?,
                created: false,
            });
        }
    }

    let course = repo::course(&state.db, course_id)
        .await?
        .filter(|course| !course.deleted)
        .ok_or_else(|| not_found("Course"))?;
    ensure_purchasable(&course, user)?;
    ensure_not_enrolled(state, user, course_id).await?;
    retire_lapsed_order(state, user.id, course_id).await?;

    let snapshot = PriceSnapshot {
        v: 1,
        course_id,
        course_title: course.title.clone(),
        course_slug: course.slug.clone(),
        creator_id: course.creator_id,
        list_price_paise: course.price_paise,
        discount_paise: 0,
        amount_paise: course.price_paise,
        currency: course.currency.clone(),
    };
    let snapshot = serde_json::to_value(&snapshot).map_err(anyhow::Error::from)?;
    let new = NewOrder {
        id: Uuid::new_v4(),
        user_id: user.id,
        course_id,
        amount_paise: course.price_paise,
        currency: &course.currency,
        price_snapshot: &snapshot,
        provider: state.payments.name(),
        idempotency_key,
        ttl_minutes: i32::try_from(state.config.order_ttl_minutes).unwrap_or(30),
    };
    let mut conn = state.db.acquire().await?;
    let inserted = repo::insert(&mut conn, &new).await?;
    drop(conn);

    let (order, created) = match inserted {
        Some(order) => (order, true),
        // The user already has an open order for this course (a double click, a second tab) or
        // another request with this key got there first: they share one order.
        None => (
            existing_after_conflict(state, user.id, course_id, idempotency_key).await?,
            false,
        ),
    };
    Ok(Checkout {
        order: settle(state, order).await?,
        created,
    })
}

fn key_reused() -> ApiError {
    conflict(
        "idempotency_key_reused",
        "This Idempotency-Key was already used for a different course.",
    )
}

fn ensure_purchasable(course: &CourseForCheckout, user: &AuthUser) -> Result<(), ApiError> {
    if course.status != CourseStatus::Published {
        return Err(conflict(
            "course_unavailable",
            "This course is not accepting enrollments.",
        ));
    }
    if course.creator_id == user.id {
        return Err(conflict("own_course", "You cannot buy your own course."));
    }
    if course.price_paise == 0 {
        return Err(conflict(
            "not_payable",
            format!(
                "This course is free. Enroll at /v1/courses/{}/enroll.",
                course.id
            ),
        ));
    }
    if course.currency != "INR" {
        return Err(conflict(
            "not_payable",
            "Only payments in INR are supported.",
        ));
    }
    Ok(())
}

async fn ensure_not_enrolled(
    state: &AppState,
    user: &AuthUser,
    course_id: Uuid,
) -> Result<(), ApiError> {
    let Some(gate) = repo::enrollment_gate(&state.db, user.id, course_id).await? else {
        return Ok(());
    };
    if gate.live {
        return Err(conflict(
            "already_enrolled",
            "You already have access to this course.",
        ));
    }
    // Access a creator took away is not something a payment can buy back; one that ended by a
    // refund or by expiring can be bought again.
    if gate.revoked && !matches!(gate.revocation_kind.as_deref(), Some("payment" | "system")) {
        return Err(conflict(
            "access_revoked_by_creator",
            "Access was revoked. Contact the course creator to restore it.",
        ));
    }
    Ok(())
}

/// Clears away this user's open order for the course if it has run out of time, so a new one can
/// be made. Before doing so, asks the provider whether the customer paid in the last moments:
/// a second order beside a payment in flight is how people get charged twice.
async fn retire_lapsed_order(
    state: &AppState,
    user_id: Uuid,
    course_id: Uuid,
) -> Result<(), ApiError> {
    let Some(open) = repo::open_order(&state.db, user_id, course_id).await? else {
        return Ok(());
    };
    if !open.lapsed {
        return Ok(());
    }
    if let Some(provider_order_id) = &open.provider_order_id {
        match state.payments.fetch_order(provider_order_id).await {
            Ok(order)
                if order.status == ProviderOrderStatus::Paid || order.amount_paid_paise > 0 =>
            {
                return Err(conflict(
                    "payment_processing",
                    "A payment for this course is being confirmed. Your access will appear shortly.",
                ));
            }
            Ok(_) | Err(ProviderError::NotFound) => {}
            // Cannot tell whether money moved: refuse rather than risk charging twice.
            Err(error) => return Err(provider_failure(error)),
        }
    }
    repo::expire(&state.db, open.id).await?;
    Ok(())
}

async fn existing_after_conflict(
    state: &AppState,
    user_id: Uuid,
    course_id: Uuid,
    idempotency_key: Option<&str>,
) -> Result<OrderRow, ApiError> {
    if let Some(order) = repo::open_order(&state.db, user_id, course_id).await? {
        return Ok(order);
    }
    if let Some(key) = idempotency_key {
        if let Some(order) = repo::by_key(&state.db, user_id, key).await? {
            if order.course_id != course_id {
                return Err(key_reused());
            }
            return Ok(order);
        }
    }
    // The order that blocked the insert moved on between the two statements. Asking again works.
    Err(conflict(
        "checkout_in_progress",
        "Checkout is already in progress. Please try again in a moment.",
    ))
}

/// Makes sure an open order has its provider order, creating it if nobody has.
async fn settle(state: &AppState, order: OrderRow) -> Result<OrderRow, ApiError> {
    if order.status != OrderStatus::Created || order.lapsed {
        return Ok(order);
    }
    let started = Instant::now();
    loop {
        if let Some(earlier_attempts) = repo::claim_attempt(&state.db, order.id).await? {
            return match tokio::time::timeout(
                repo::ATTEMPT_WORK_TIMEOUT,
                create_at_provider(state, order, earlier_attempts),
            )
            .await
            {
                Ok(result) => result,
                // The provider may have carried out a cancelled request. Leave the claim until
                // it lapses; the next caller must look up the receipt before creating again.
                Err(_) => Err(provider_failure(ProviderError::Timeout)),
            };
        }
        // Someone else is on it, or it has moved on. Look at where it stands now.
        let current = repo::by_id(&state.db, order.id)
            .await?
            .ok_or_else(|| ApiError::Internal(anyhow::anyhow!("order vanished")))?;
        if current.provider_order_id.is_some() || current.status != OrderStatus::Created {
            return Ok(current);
        }
        if started.elapsed() >= PEER_WAIT {
            return Err(conflict(
                "checkout_in_progress",
                "Checkout is already in progress. Please try again in a few seconds.",
            ));
        }
        tokio::time::sleep(PEER_POLL).await;
    }
}

async fn create_at_provider(
    state: &AppState,
    order: OrderRow,
    earlier_attempts: i32,
) -> Result<OrderRow, ApiError> {
    let receipt = order.id.to_string();
    let provider = &state.payments;

    // An earlier attempt that ended without an answer may have created the order. Creating
    // again would charge nobody twice but would leave a second order behind, so look first.
    let found = if earlier_attempts > 0 {
        match provider.find_order_by_receipt(&receipt).await {
            Ok(found) => found,
            Err(error) => {
                let _ = repo::release_attempt(&state.db, order.id).await;
                return Err(provider_failure(error));
            }
        }
    } else {
        None
    };
    let provider_order = match found {
        Some(existing) => existing,
        None => {
            let request = CreateOrder {
                receipt: receipt.clone(),
                amount_paise: order.amount_paise,
                currency: order.currency.clone(),
                notes: BTreeMap::from([
                    ("zeno_order_id".to_owned(), receipt.clone()),
                    ("zeno_course_id".to_owned(), order.course_id.to_string()),
                ]),
            };
            match provider.create_order(&request).await {
                Ok(created) => created,
                Err(error) => {
                    // After a timeout the order may exist at the provider. Keep the claim until
                    // it lapses so nobody else creates one beside it; the next attempt looks
                    // it up first. A clear refusal releases the claim at once.
                    if !error.outcome_unknown() {
                        let _ = repo::release_attempt(&state.db, order.id).await;
                    }
                    return Err(provider_failure(error));
                }
            }
        }
    };

    if let Err(problem) = matches_request(&order, &provider_order) {
        tracing::error!(order_id = %order.id, %problem, "provider order does not match ours");
        let _ = repo::release_attempt(&state.db, order.id).await;
        return Err(ApiError::Http {
            status: StatusCode::BAD_GATEWAY,
            code: "payment_provider_error",
            message: "The payment provider could not create this order.".into(),
        });
    }

    match repo::attach_provider_order(&state.db, order.id, &provider_order.id).await? {
        Some(attached) => Ok(attached),
        None => {
            // Expired (or attached by a request that raced us) while we were at the provider.
            let current = repo::by_id(&state.db, order.id)
                .await?
                .ok_or_else(|| ApiError::Internal(anyhow::anyhow!("order vanished")))?;
            if current.provider_order_id.is_some() {
                Ok(current)
            } else {
                Err(conflict(
                    "order_expired",
                    "This checkout expired. Please start again.",
                ))
            }
        }
    }
}

/// The provider's order must be exactly the one we asked for.
fn matches_request(order: &OrderRow, provider_order: &ProviderOrder) -> Result<(), &'static str> {
    if provider_order.id.is_empty() {
        return Err("empty provider order id");
    }
    if provider_order.amount_paise != order.amount_paise {
        return Err("amount differs");
    }
    if provider_order.currency != order.currency {
        return Err("currency differs");
    }
    if provider_order.receipt.as_deref() != Some(order.id.to_string().as_str()) {
        return Err("receipt differs");
    }
    Ok(())
}

pub async fn show(state: &AppState, user: &AuthUser, order_id: Uuid) -> Result<OrderRow, ApiError> {
    // Someone else's order looks exactly like one that does not exist.
    repo::by_id_for_user(&state.db, order_id, user.id)
        .await?
        .ok_or_else(|| not_found("Order"))
}

/// The response for an order: its frozen terms, and the widget parameters while it can be paid.
pub fn envelope(state: &AppState, order: &OrderRow) -> Result<OrderEnvelope, ApiError> {
    let snapshot: PriceSnapshot =
        serde_json::from_value(order.price_snapshot.clone()).map_err(anyhow::Error::from)?;
    let status = order.effective_status();
    let checkout = match (&order.provider_order_id, status) {
        (Some(provider_order_id), OrderStatus::Pending) => Some(CheckoutParams {
            provider: order.provider.clone(),
            key: state.payments.checkout_key(),
            provider_order_id: provider_order_id.clone(),
            amount_paise: order.amount_paise,
            currency: order.currency.clone(),
        }),
        _ => None,
    };
    Ok(OrderEnvelope {
        order: OrderResponse {
            id: order.id,
            status,
            course: OrderCourse {
                id: snapshot.course_id,
                title: snapshot.course_title,
                slug: snapshot.course_slug,
            },
            amount_paise: order.amount_paise,
            currency: order.currency.clone(),
            expires_at: order.expires_at,
            created_at: order.created_at,
        },
        checkout,
    })
}
