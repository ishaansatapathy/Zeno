use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Everything printed on an invoice, as issued. Stored whole in `invoices.data`, so the invoice
/// shows what it showed on the day, whatever happens to the course or the buyer's name afterwards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoiceData {
    pub v: u8,
    pub number: String,
    pub issued_at: DateTime<Utc>,
    pub order_id: Uuid,
    /// The provider's id for the payment this invoice is for.
    pub payment_ref: Option<String>,
    pub seller: Seller,
    pub buyer: Buyer,
    pub lines: Vec<Line>,
    pub total_paise: i64,
    pub currency: String,
    /// Reserved for tax. Always empty until the tax side has been advised on.
    pub tax: Option<Tax>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seller {
    pub name: String,
    pub address: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Buyer {
    pub name: String,
    pub email: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Line {
    pub description: String,
    pub list_price_paise: i64,
    pub discount_paise: i64,
    pub coupon_code: Option<String>,
    /// What was charged for this line: list price less discount.
    pub amount_paise: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tax {
    pub label: String,
    pub amount_paise: i64,
}

#[derive(Debug, Serialize)]
pub struct InvoiceResponse {
    pub number: String,
    pub issued_at: DateTime<Utc>,
    pub order_id: Uuid,
    pub amount_paise: i64,
    pub currency: String,
    pub invoice: InvoiceData,
}

#[derive(Debug, Serialize)]
pub struct InvoiceEnvelope {
    pub invoice: InvoiceResponse,
}
