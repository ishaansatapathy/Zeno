use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeno_core::model::{CouponKind, OrderStatus};

use crate::enrollments::dto::EnrollmentResponse;
use crate::refunds::dto::RefundResponse;

/// What an order was priced from, frozen when it was made (`orders.price_snapshot`). The
/// receipt and any later dispute read this, not the course as it is today.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceSnapshot {
    pub v: u8,
    pub course_id: Uuid,
    pub course_title: String,
    pub course_slug: String,
    pub creator_id: Uuid,
    pub list_price_paise: i64,
    pub discount_paise: i64,
    pub amount_paise: i64,
    pub currency: String,
    /// The refund window the buyer was promised, copied from the course when the order was made.
    /// Orders made before this field existed get the platform default.
    #[serde(default = "default_refund_window_days")]
    pub refund_window_days: i32,
    /// The coupon that produced the discount, if one did.
    #[serde(default)]
    pub coupon: Option<CouponSnapshot>,
}

/// The coupon as it was when the order was made.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CouponSnapshot {
    pub id: Uuid,
    pub code: String,
    pub kind: CouponKind,
    pub value: i64,
}

pub const DEFAULT_REFUND_WINDOW_DAYS: i32 = 7;

fn default_refund_window_days() -> i32 {
    DEFAULT_REFUND_WINDOW_DAYS
}

#[derive(Debug, Serialize)]
pub struct OrderCourse {
    pub id: Uuid,
    pub title: String,
    pub slug: String,
}

#[derive(Debug, Serialize)]
pub struct OrderResponse {
    pub id: Uuid,
    pub status: OrderStatus,
    pub course: OrderCourse,
    /// What the buyer pays, after any coupon.
    pub amount_paise: i64,
    pub list_price_paise: i64,
    pub discount_paise: i64,
    pub coupon_code: Option<String>,
    pub currency: String,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub paid_at: Option<DateTime<Utc>>,
    /// Until when the buyer may ask for a refund; absent when the order is not paid or the course
    /// does not offer refunds.
    pub refundable_until: Option<DateTime<Utc>>,
}

/// What the browser hands to the provider's checkout widget. Only present while the order can
/// still be paid.
#[derive(Debug, Serialize)]
pub struct CheckoutParams {
    pub provider: String,
    /// The provider's public key (never a secret).
    pub key: String,
    pub provider_order_id: String,
    pub amount_paise: i64,
    pub currency: String,
}

#[derive(Debug, Serialize)]
pub struct OrderEnvelope {
    pub order: OrderResponse,
    pub checkout: Option<CheckoutParams>,
    /// The latest refund for the order, if there has been one.
    pub refund: Option<RefundResponse>,
}

/// The optional body of a checkout request. Everything else a client might send is ignored: the
/// price is never the client's to name.
#[derive(Debug, Default, Deserialize)]
pub struct CheckoutBody {
    #[serde(default)]
    pub coupon: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CouponBrief {
    pub code: String,
    pub kind: CouponKind,
    pub value: i64,
}

/// What a course costs this person with this coupon, worked out by the server.
#[derive(Debug, Serialize)]
pub struct PriceView {
    pub list_price_paise: i64,
    pub discount_paise: i64,
    pub amount_paise: i64,
    /// The coupon covers everything: checking out enrolls without a payment.
    pub free: bool,
    pub coupon: Option<CouponBrief>,
}

#[derive(Debug, Serialize)]
pub struct PreviewEnvelope {
    pub price: PriceView,
}

/// The answer to a checkout whose coupon made the course free.
#[derive(Debug, Serialize)]
pub struct EnrolledEnvelope {
    pub status: &'static str,
    pub enrollment: EnrollmentResponse,
    pub price: PriceView,
}
