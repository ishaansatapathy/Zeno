use axum::http::StatusCode;
use chrono::Utc;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;
use zeno_core::model::Role;

use crate::auth::extractor::AuthUser;
use crate::coupons::dto::{normalize_code, CouponChanges, NewCoupon, MAX_COUPONS_PER_CREATOR};
use crate::coupons::pricing::{self, Priced};
use crate::coupons::repo::{self, CouponRow};
use crate::error::{ApiError, FieldError};
use crate::state::AppState;

const STAFF: &[Role] = &[Role::Creator, Role::Admin];

/// Why a coupon cannot be used right now. Each has its own code so a checkout page can say
/// something useful. A code that belongs to another creator, or is switched off, is reported as
/// not found, exactly like one that never existed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    NotFound,
    NotApplicable,
    NotStarted,
    Expired,
    Exhausted,
    UserLimit,
    NoEffect,
}

impl Refusal {
    fn code(self) -> &'static str {
        match self {
            Self::NotFound => "coupon_not_found",
            Self::NotApplicable => "coupon_not_applicable",
            Self::NotStarted => "coupon_not_started",
            Self::Expired => "coupon_expired",
            Self::Exhausted => "coupon_exhausted",
            Self::UserLimit => "coupon_limit_reached",
            Self::NoEffect => "coupon_no_effect",
        }
    }

    fn message(self) -> &'static str {
        match self {
            Self::NotFound => "This coupon code is not valid.",
            Self::NotApplicable => "This coupon does not apply to this course.",
            Self::NotStarted => "This coupon is not valid yet.",
            Self::Expired => "This coupon has expired.",
            Self::Exhausted => "This coupon has been fully redeemed.",
            Self::UserLimit => "You have already used this coupon.",
            Self::NoEffect => "This coupon would not lower the price of this course.",
        }
    }
}

pub fn refuse(reason: Refusal) -> ApiError {
    ApiError::Http {
        status: StatusCode::UNPROCESSABLE_ENTITY,
        code: reason.code(),
        message: reason.message().into(),
    }
}

/// A coupon that applies, and what it does to the price.
#[derive(Debug, Clone)]
pub struct Evaluated {
    pub coupon: CouponRow,
    pub priced: Priced,
}

/// Whether `raw_code` can be used on this course by this person, and the price it gives. This is
/// advice; the binding check is [`redeem`], which settles races.
pub async fn evaluate(
    db: &PgPool,
    creator_id: Uuid,
    course_id: Uuid,
    list_paise: i64,
    user_id: Uuid,
    raw_code: &str,
    holding_order: Option<Uuid>,
) -> Result<Evaluated, ApiError> {
    let code = normalize_code(raw_code).map_err(|_| refuse(Refusal::NotFound))?;
    // Only the course creator's own codes exist for this course.
    let coupon = repo::by_code(db, creator_id, &code)
        .await?
        .filter(|coupon| coupon.active)
        .ok_or_else(|| refuse(Refusal::NotFound))?;
    if coupon.course_id.is_some_and(|only| only != course_id) {
        return Err(refuse(Refusal::NotApplicable));
    }
    let now = Utc::now();
    if coupon.valid_from.is_some_and(|from| from > now) {
        return Err(refuse(Refusal::NotStarted));
    }
    if coupon.valid_until.is_some_and(|until| until <= now) {
        return Err(refuse(Refusal::Expired));
    }
    // The buyer's own open order may already hold one of the redemptions; asking for the same
    // coupon again is not asking for another.
    let held = match holding_order {
        Some(order) => repo::coupon_of_order(db, order).await? == Some(coupon.id),
        None => false,
    };
    let in_use_by_others = coupon.redeemed_count - i32::from(held);
    if coupon
        .max_redemptions
        .is_some_and(|max| in_use_by_others >= max)
    {
        return Err(refuse(Refusal::Exhausted));
    }
    let except = if held { holding_order } else { None };
    if repo::uses_by(db, coupon.id, user_id, except).await? >= i64::from(coupon.per_user_limit) {
        return Err(refuse(Refusal::UserLimit));
    }
    let priced = pricing::price(list_paise, coupon.kind, coupon.value)
        .map_err(|_| refuse(Refusal::NoEffect))?;
    Ok(Evaluated { coupon, priced })
}

/// What a redemption is attached to.
#[derive(Debug, Clone, Copy)]
pub enum Target {
    Order(Uuid),
    Enrollment(Uuid),
}

/// Uses the coupon, for real, inside the caller's transaction. If this fails the caller drops the
/// transaction and nothing was used. The conditional update serialises redemptions of one coupon,
/// which is also what makes the per-person limit safe: two tabs of one buyer cannot both pass it.
pub async fn redeem(
    conn: &mut PgConnection,
    user_id: Uuid,
    coupon: &CouponRow,
    target: Target,
    discount_paise: i64,
) -> Result<(), ApiError> {
    let Some(per_user_limit) = repo::take_redemption(conn, coupon.id).await? else {
        // Someone got there first, or it was switched off or ran out while the buyer decided.
        let fresh = repo::by_id(&mut *conn, coupon.id)
            .await?
            .ok_or_else(|| refuse(Refusal::NotFound))?;
        let now = Utc::now();
        let reason = if !fresh.active {
            Refusal::NotFound
        } else if fresh.valid_from.is_some_and(|from| from > now) {
            Refusal::NotStarted
        } else if fresh.valid_until.is_some_and(|until| until <= now) {
            Refusal::Expired
        } else {
            Refusal::Exhausted
        };
        return Err(refuse(reason));
    };
    if repo::uses_by(&mut *conn, coupon.id, user_id, None).await? >= i64::from(per_user_limit) {
        return Err(refuse(Refusal::UserLimit));
    }
    let (order_id, enrollment_id) = match target {
        Target::Order(id) => (Some(id), None),
        Target::Enrollment(id) => (None, Some(id)),
    };
    repo::insert_redemption(
        conn,
        coupon.id,
        user_id,
        order_id,
        enrollment_id,
        discount_paise,
    )
    .await?;
    Ok(())
}

// ---- a creator's coupons ------------------------------------------------------------------------

fn not_found() -> ApiError {
    ApiError::Http {
        status: StatusCode::NOT_FOUND,
        code: "not_found",
        message: "Coupon not found.".into(),
    }
}

pub async fn create(
    state: &AppState,
    actor: &AuthUser,
    new: NewCoupon,
) -> Result<CouponRow, ApiError> {
    actor.require_role(STAFF)?;
    if let Some(course) = new.course_id {
        if !repo::owns_course(&state.db, actor.id, course).await? {
            return Err(ApiError::validation(vec![FieldError::new(
                "course_id",
                "Not one of your courses.",
            )]));
        }
    }
    if repo::count_for_creator(&state.db, actor.id).await? >= MAX_COUPONS_PER_CREATOR {
        return Err(ApiError::Http {
            status: StatusCode::CONFLICT,
            code: "coupon_limit",
            message: "You have reached the limit of coupons. Deactivate ones you no longer need."
                .into(),
        });
    }
    repo::insert(&state.db, actor.id, &new)
        .await?
        .ok_or_else(|| ApiError::Http {
            status: StatusCode::CONFLICT,
            code: "coupon_exists",
            message: "You already have a coupon with this code.".into(),
        })
}

pub async fn list(
    state: &AppState,
    actor: &AuthUser,
    limit: Option<u32>,
) -> Result<Vec<CouponRow>, ApiError> {
    actor.require_role(STAFF)?;
    let limit = i64::from(limit.unwrap_or(50).clamp(1, 100));
    Ok(repo::list_for_creator(&state.db, actor.id, limit).await?)
}

pub async fn update(
    state: &AppState,
    actor: &AuthUser,
    id: Uuid,
    changes: CouponChanges,
) -> Result<CouponRow, ApiError> {
    actor.require_role(STAFF)?;
    let coupon = repo::by_id(&state.db, id).await?.ok_or_else(not_found)?;
    if actor.role != Role::Admin && coupon.creator_id != actor.id {
        return Err(not_found());
    }
    repo::update(&state.db, id, &changes)
        .await?
        .ok_or_else(|| ApiError::Http {
            status: StatusCode::CONFLICT,
            code: "invalid_change",
            message: "The new limit is below the redemptions already used, or the end date is \
                      before the start date."
                .into(),
        })
}
