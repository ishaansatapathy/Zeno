use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeno_core::model::OrderStatus;

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
    pub amount_paise: i64,
    pub currency: String,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
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
}
