//! The boundary to the payment provider.
//!
//! Nothing outside this module knows what Razorpay's JSON looks like. The rest of the system
//! talks to a [`PaymentProvider`], which has two implementations:
//!
//! * [`RazorpayProvider`], the real thing: basic-auth HTTPS, short timeouts, retries only for
//!   reads (a repeated `GET` cannot do harm; a repeated "create order" can);
//! * [`FakeProvider`], an in-memory provider that can be told to time out, fail, answer slowly or
//!   create an order and *then* fail, so every unhappy path is testable without a network.
//!
//! Amounts are integers in paise everywhere. A provider's answer is never trusted blindly: the
//! caller compares it with what was asked for (see `Expected` in the API's checkout service).

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;

pub mod fake;
pub mod razorpay;
pub mod webhook;

pub use fake::{FakeProvider, Fault, Op};
pub use razorpay::{RazorpayConfig, RazorpayProvider};

/// What goes wrong when talking to a provider, grouped by what the caller should *do*.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderError {
    /// No answer in time. The request may or may not have been carried out.
    #[error("the payment provider did not answer in time")]
    Timeout,
    /// A 5xx or a connection that broke mid-request. Possibly carried out.
    #[error("the payment provider failed: {0}")]
    Unavailable(String),
    /// Could not connect at all, so the request certainly was not carried out.
    #[error("the payment provider is unreachable: {0}")]
    Unreachable(String),
    #[error("the payment provider is rate limiting us")]
    RateLimited,
    /// A 4xx that retrying will not fix: a bad amount, a bad id.
    #[error("the payment provider refused the request: {code}: {description}")]
    Rejected { code: String, description: String },
    /// Our API credentials were refused. Retrying cannot help; someone has to fix the config.
    #[error("the payment provider refused our credentials")]
    Unauthorized,
    #[error("the payment provider has no such object")]
    NotFound,
    /// A 2xx whose body is not what the provider documents.
    #[error("unexpected response from the payment provider: {0}")]
    Malformed(String),
}

impl ProviderError {
    /// True when the request might have been carried out even though we got no clear answer.
    /// After such an error on a write, look the object up before trying again.
    pub fn outcome_unknown(&self) -> bool {
        matches!(self, Self::Timeout | Self::Unavailable(_))
    }

    /// Worth retrying (for an operation that is safe to repeat).
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::Timeout | Self::Unavailable(_) | Self::Unreachable(_) | Self::RateLimited
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateOrder {
    /// Our order id. The provider keeps it and returns it, which is how an order whose creation
    /// timed out is found again instead of being created twice.
    pub receipt: String,
    pub amount_paise: i64,
    pub currency: String,
    /// Free-form labels for the provider's dashboard (never anything secret).
    pub notes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderOrderStatus {
    Created,
    /// A payment was tried (and may have failed); the order is still open.
    Attempted,
    Paid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderOrder {
    pub id: String,
    pub amount_paise: i64,
    pub amount_paid_paise: i64,
    pub currency: String,
    pub receipt: Option<String>,
    pub status: ProviderOrderStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderPaymentStatus {
    Created,
    Authorized,
    Captured,
    Refunded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderPayment {
    pub id: String,
    pub order_id: Option<String>,
    pub amount_paise: i64,
    pub amount_refunded_paise: i64,
    pub currency: String,
    pub status: ProviderPaymentStatus,
    /// What the provider kept as its fee (tax on the fee included), when it says. Only captured
    /// payments have one.
    pub fee_paise: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefundRequest {
    pub payment_id: String,
    pub amount_paise: i64,
    /// Our own id for this refund. Sent to the provider so a repeated request is a no-op.
    pub idempotency_key: String,
    pub notes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderRefundStatus {
    Pending,
    Processed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRefund {
    pub id: String,
    pub payment_id: String,
    pub amount_paise: i64,
    pub status: ProviderRefundStatus,
}

#[async_trait]
pub trait PaymentProvider: Send + Sync {
    /// Stored on every order, so a row always says which provider's ids it holds.
    fn name(&self) -> &'static str;

    /// The public half of the credentials: what the browser's checkout widget needs to open.
    fn checkout_key(&self) -> String;

    /// Creates an order. **Not retried by the implementation**: see [`Self::find_order_by_receipt`].
    async fn create_order(&self, order: &CreateOrder) -> Result<ProviderOrder, ProviderError>;

    /// The oldest order created with this receipt, if any. Used after an uncertain
    /// `create_order` before creating again.
    async fn find_order_by_receipt(
        &self,
        receipt: &str,
    ) -> Result<Option<ProviderOrder>, ProviderError>;

    async fn fetch_order(&self, provider_order_id: &str) -> Result<ProviderOrder, ProviderError>;

    async fn fetch_payment(
        &self,
        provider_payment_id: &str,
    ) -> Result<ProviderPayment, ProviderError>;

    /// Every payment attempt made against an order, failed ones included. Reconciliation uses it
    /// to find the payment behind an order the provider reports as paid.
    async fn fetch_order_payments(
        &self,
        provider_order_id: &str,
    ) -> Result<Vec<ProviderPayment>, ProviderError>;

    async fn fetch_refund(&self, provider_refund_id: &str)
        -> Result<ProviderRefund, ProviderError>;

    async fn refund(&self, request: &RefundRequest) -> Result<ProviderRefund, ProviderError>;

    /// Whether `signature` is the provider's signature of exactly these bytes.
    fn verify_webhook(&self, body: &[u8], signature: &str) -> bool;
}

/// How long a single provider call may take before it counts as a [`ProviderError::Timeout`].
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
