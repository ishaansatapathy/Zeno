//! An in-memory payment provider for tests and local development.
//!
//! It behaves like the real one where that matters (receipts are *not* unique, so creating twice
//! really makes two orders; a refund with a repeated idempotency key is a no-op) and can be told
//! to misbehave on demand, one call at a time:
//!
//! ```ignore
//! provider.inject(Op::CreateOrder, Fault::FailAfter(ProviderError::Timeout));
//! ```
//!
//! makes the next `create_order` create the order and then report a timeout, which is exactly the
//! situation the checkout flow has to survive.
//!
//! The "world" methods (`capture`, `fail_payment`) stand in for the customer paying in the
//! browser; later webhook tests drive them.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use async_trait::async_trait;

use super::{
    webhook, CreateOrder, PaymentProvider, ProviderError, ProviderOrder, ProviderOrderStatus,
    ProviderPayment, ProviderPaymentStatus, ProviderRefund, ProviderRefundStatus, RefundRequest,
};
use crate::secret::Secret;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Op {
    CreateOrder,
    FindOrder,
    FetchOrder,
    FetchPayment,
    FetchOrderPayments,
    FetchRefund,
    Refund,
}

#[derive(Debug, Clone)]
pub enum Fault {
    /// Fail without doing anything.
    Fail(ProviderError),
    /// Do the work, then fail: the provider has the object, the caller never hears of it.
    FailAfter(ProviderError),
    /// Answer correctly, but only after this long.
    Delay(Duration),
}

#[derive(Default)]
struct State {
    next_id: u64,
    orders: Vec<ProviderOrder>,
    payments: BTreeMap<String, ProviderPayment>,
    refunds: BTreeMap<String, ProviderRefund>,
    refund_keys: HashMap<String, String>,
    /// When set, new refunds stay pending until [`FakeProvider::settle_refund`], like a bank
    /// transfer that takes a while.
    refunds_pend: bool,
    /// The fee the fake provider charges on a captured payment, in basis points of the amount
    /// (`None`: it reports no fee, so callers must estimate).
    fee_bps: Option<u32>,
    faults: HashMap<Op, VecDeque<Fault>>,
    calls: HashMap<Op, usize>,
}

impl State {
    fn id(&mut self, prefix: &str) -> String {
        self.next_id += 1;
        format!("{prefix}_fake{:08}", self.next_id)
    }
}

#[derive(Clone)]
pub struct FakeProvider {
    state: Arc<Mutex<State>>,
    webhook_secret: Secret,
}

impl Default for FakeProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for FakeProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FakeProvider")
    }
}

impl FakeProvider {
    pub fn new() -> Self {
        Self::with_webhook_secret("fake-webhook-secret")
    }

    pub fn with_webhook_secret(secret: &str) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                // About what Razorpay charges (2% plus GST on the fee).
                fee_bps: Some(236),
                ..State::default()
            })),
            webhook_secret: Secret::new(secret),
        }
    }

    /// Sets the fee charged on payments captured from now on (`None`: report no fee).
    pub fn set_fee_bps(&self, fee_bps: Option<u32>) {
        self.lock().fee_bps = fee_bps;
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Makes the next call of `op` (after any already queued) misbehave.
    pub fn inject(&self, op: Op, fault: Fault) {
        self.lock().faults.entry(op).or_default().push_back(fault);
    }

    /// How many times `op` has been called, faults included.
    pub fn calls(&self, op: Op) -> usize {
        self.lock().calls.get(&op).copied().unwrap_or(0)
    }

    pub fn orders(&self) -> Vec<ProviderOrder> {
        self.lock().orders.clone()
    }

    pub fn orders_with_receipt(&self, receipt: &str) -> Vec<ProviderOrder> {
        self.lock()
            .orders
            .iter()
            .filter(|order| order.receipt.as_deref() == Some(receipt))
            .cloned()
            .collect()
    }

    pub fn refunds(&self) -> Vec<ProviderRefund> {
        self.lock().refunds.values().cloned().collect()
    }

    /// The customer pays in full: the order becomes paid and a captured payment exists.
    pub fn capture(&self, provider_order_id: &str) -> Result<ProviderPayment, ProviderError> {
        let mut state = self.lock();
        let position = state
            .orders
            .iter()
            .position(|order| order.id == provider_order_id)
            .ok_or(ProviderError::NotFound)?;
        let order = state.orders[position].clone();
        if let Some(existing) = state.payments.values().find(|payment| {
            payment.order_id.as_deref() == Some(provider_order_id)
                && payment.status == ProviderPaymentStatus::Captured
        }) {
            return Ok(existing.clone());
        }
        let fee_paise = state.fee_bps.map(|bps| {
            i64::try_from((i128::from(order.amount_paise) * i128::from(bps) + 5_000) / 10_000)
                .unwrap_or(i64::MAX)
        });
        let payment = ProviderPayment {
            id: state.id("pay"),
            order_id: Some(order.id.clone()),
            amount_paise: order.amount_paise,
            amount_refunded_paise: 0,
            currency: order.currency.clone(),
            status: ProviderPaymentStatus::Captured,
            fee_paise,
        };
        state.orders[position].amount_paid_paise = order.amount_paise;
        state.orders[position].status = ProviderOrderStatus::Paid;
        state.payments.insert(payment.id.clone(), payment.clone());
        Ok(payment)
    }

    /// A payment attempt that fails (declined card): the order stays open.
    pub fn fail_payment(&self, provider_order_id: &str) -> Result<ProviderPayment, ProviderError> {
        let mut state = self.lock();
        let position = state
            .orders
            .iter()
            .position(|order| order.id == provider_order_id)
            .ok_or(ProviderError::NotFound)?;
        let order = state.orders[position].clone();
        let payment = ProviderPayment {
            id: state.id("pay"),
            order_id: Some(order.id),
            amount_paise: order.amount_paise,
            amount_refunded_paise: 0,
            currency: order.currency,
            status: ProviderPaymentStatus::Failed,
            fee_paise: None,
        };
        if state.orders[position].status == ProviderOrderStatus::Created {
            state.orders[position].status = ProviderOrderStatus::Attempted;
        }
        state.payments.insert(payment.id.clone(), payment.clone());
        Ok(payment)
    }

    /// From now on refunds start `pending` and finish only when [`Self::settle_refund`] is called.
    pub fn hold_refunds(&self, hold: bool) {
        self.lock().refunds_pend = hold;
    }

    /// The bank finishes a pending refund.
    pub fn settle_refund(&self, provider_refund_id: &str) -> Result<ProviderRefund, ProviderError> {
        let mut state = self.lock();
        let refund = state
            .refunds
            .get_mut(provider_refund_id)
            .ok_or(ProviderError::NotFound)?;
        refund.status = ProviderRefundStatus::Processed;
        Ok(refund.clone())
    }

    /// The provider gives up on a pending refund; the money stays with the customer's payment.
    pub fn fail_refund(&self, provider_refund_id: &str) -> Result<ProviderRefund, ProviderError> {
        let mut state = self.lock();
        let (payment_id, amount) = {
            let refund = state
                .refunds
                .get_mut(provider_refund_id)
                .ok_or(ProviderError::NotFound)?;
            refund.status = ProviderRefundStatus::Failed;
            (refund.payment_id.clone(), refund.amount_paise)
        };
        if let Some(payment) = state.payments.get_mut(&payment_id) {
            payment.amount_refunded_paise -= amount;
            payment.status = ProviderPaymentStatus::Captured;
        }
        Ok(state.refunds[provider_refund_id].clone())
    }

    /// The signature the provider would put on a webhook with this body.
    pub fn sign_webhook(&self, body: &[u8]) -> String {
        webhook::sign(self.webhook_secret.expose(), body)
    }

    async fn run<T>(
        &self,
        op: Op,
        work: impl FnOnce(&mut State) -> Result<T, ProviderError>,
    ) -> Result<T, ProviderError> {
        let fault = {
            let mut state = self.lock();
            *state.calls.entry(op).or_default() += 1;
            state.faults.get_mut(&op).and_then(VecDeque::pop_front)
        };
        match &fault {
            Some(Fault::Fail(error)) => return Err(error.clone()),
            Some(Fault::Delay(pause)) => tokio::time::sleep(*pause).await,
            _ => {}
        }
        let result = work(&mut self.lock());
        match fault {
            Some(Fault::FailAfter(error)) => Err(error),
            _ => result,
        }
    }
}

#[async_trait]
impl PaymentProvider for FakeProvider {
    fn name(&self) -> &'static str {
        "fake"
    }

    fn checkout_key(&self) -> String {
        "fake_key".to_owned()
    }

    async fn create_order(&self, order: &CreateOrder) -> Result<ProviderOrder, ProviderError> {
        let order = order.clone();
        self.run(Op::CreateOrder, move |state| {
            let created = ProviderOrder {
                id: state.id("order"),
                amount_paise: order.amount_paise,
                amount_paid_paise: 0,
                currency: order.currency,
                receipt: Some(order.receipt),
                status: ProviderOrderStatus::Created,
            };
            state.orders.push(created.clone());
            Ok(created)
        })
        .await
    }

    async fn find_order_by_receipt(
        &self,
        receipt: &str,
    ) -> Result<Option<ProviderOrder>, ProviderError> {
        self.run(Op::FindOrder, |state| {
            Ok(state
                .orders
                .iter()
                .find(|order| order.receipt.as_deref() == Some(receipt))
                .cloned())
        })
        .await
    }

    async fn fetch_order(&self, provider_order_id: &str) -> Result<ProviderOrder, ProviderError> {
        self.run(Op::FetchOrder, |state| {
            state
                .orders
                .iter()
                .find(|order| order.id == provider_order_id)
                .cloned()
                .ok_or(ProviderError::NotFound)
        })
        .await
    }

    async fn fetch_payment(
        &self,
        provider_payment_id: &str,
    ) -> Result<ProviderPayment, ProviderError> {
        self.run(Op::FetchPayment, |state| {
            state
                .payments
                .get(provider_payment_id)
                .cloned()
                .ok_or(ProviderError::NotFound)
        })
        .await
    }

    async fn fetch_order_payments(
        &self,
        provider_order_id: &str,
    ) -> Result<Vec<ProviderPayment>, ProviderError> {
        self.run(Op::FetchOrderPayments, |state| {
            if !state
                .orders
                .iter()
                .any(|order| order.id == provider_order_id)
            {
                return Err(ProviderError::NotFound);
            }
            Ok(state
                .payments
                .values()
                .filter(|payment| payment.order_id.as_deref() == Some(provider_order_id))
                .cloned()
                .collect())
        })
        .await
    }

    async fn fetch_refund(
        &self,
        provider_refund_id: &str,
    ) -> Result<ProviderRefund, ProviderError> {
        self.run(Op::FetchRefund, |state| {
            state
                .refunds
                .get(provider_refund_id)
                .cloned()
                .ok_or(ProviderError::NotFound)
        })
        .await
    }

    async fn refund(&self, request: &RefundRequest) -> Result<ProviderRefund, ProviderError> {
        let request = request.clone();
        self.run(Op::Refund, move |state| {
            if let Some(existing) = state
                .refund_keys
                .get(&request.idempotency_key)
                .and_then(|id| state.refunds.get(id))
            {
                return Ok(existing.clone());
            }
            let id = state.id("rfnd");
            let pend = state.refunds_pend;
            let payment = state
                .payments
                .get_mut(&request.payment_id)
                .ok_or(ProviderError::NotFound)?;
            let refundable = payment.amount_paise - payment.amount_refunded_paise;
            if payment.status != ProviderPaymentStatus::Captured
                && payment.status != ProviderPaymentStatus::Refunded
                || request.amount_paise <= 0
                || request.amount_paise > refundable
            {
                return Err(ProviderError::Rejected {
                    code: "BAD_REQUEST_ERROR".into(),
                    description: "the refund amount is not available on this payment".into(),
                });
            }
            payment.amount_refunded_paise += request.amount_paise;
            if payment.amount_refunded_paise == payment.amount_paise {
                payment.status = ProviderPaymentStatus::Refunded;
            }
            let refund = ProviderRefund {
                id: id.clone(),
                payment_id: request.payment_id,
                amount_paise: request.amount_paise,
                status: if pend {
                    ProviderRefundStatus::Pending
                } else {
                    ProviderRefundStatus::Processed
                },
            };
            state
                .refund_keys
                .insert(request.idempotency_key, id.clone());
            state.refunds.insert(id, refund.clone());
            Ok(refund)
        })
        .await
    }

    fn verify_webhook(&self, body: &[u8], signature: &str) -> bool {
        webhook::verify(std::slice::from_ref(&self.webhook_secret), body, signature)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order(receipt: &str, amount: i64) -> CreateOrder {
        CreateOrder {
            receipt: receipt.into(),
            amount_paise: amount,
            currency: "INR".into(),
            notes: BTreeMap::new(),
        }
    }

    #[tokio::test]
    async fn receipts_are_not_unique_so_creating_twice_makes_two_orders() {
        let provider = FakeProvider::new();
        provider.create_order(&order("r1", 500)).await.unwrap();
        provider.create_order(&order("r1", 500)).await.unwrap();
        assert_eq!(provider.orders_with_receipt("r1").len(), 2);
    }

    #[tokio::test]
    async fn an_order_created_before_a_timeout_can_be_found_again() {
        let provider = FakeProvider::new();
        provider.inject(Op::CreateOrder, Fault::FailAfter(ProviderError::Timeout));
        assert_eq!(
            provider.create_order(&order("r1", 500)).await,
            Err(ProviderError::Timeout)
        );
        let found = provider.find_order_by_receipt("r1").await.unwrap();
        assert_eq!(found.unwrap().amount_paise, 500);
    }

    #[tokio::test]
    async fn faults_apply_once_and_in_order() {
        let provider = FakeProvider::new();
        provider.inject(Op::FetchOrder, Fault::Fail(ProviderError::RateLimited));
        provider.inject(Op::FetchOrder, Fault::Fail(ProviderError::NotFound));
        assert_eq!(
            provider.fetch_order("x").await,
            Err(ProviderError::RateLimited)
        );
        assert_eq!(
            provider.fetch_order("x").await,
            Err(ProviderError::NotFound)
        );
        assert_eq!(
            provider.fetch_order("x").await,
            Err(ProviderError::NotFound)
        );
        assert_eq!(provider.calls(Op::FetchOrder), 3);
    }

    #[tokio::test]
    async fn capturing_pays_the_order_once() {
        let provider = FakeProvider::new();
        let created = provider.create_order(&order("r1", 500)).await.unwrap();
        let first = provider.capture(&created.id).unwrap();
        let second = provider.capture(&created.id).unwrap();
        assert_eq!(first.id, second.id);
        let fetched = provider.fetch_order(&created.id).await.unwrap();
        assert_eq!(fetched.status, ProviderOrderStatus::Paid);
        assert_eq!(fetched.amount_paid_paise, 500);
    }

    #[tokio::test]
    async fn a_declined_payment_leaves_the_order_open() {
        let provider = FakeProvider::new();
        let created = provider.create_order(&order("r1", 500)).await.unwrap();
        let failed = provider.fail_payment(&created.id).unwrap();
        assert_eq!(failed.status, ProviderPaymentStatus::Failed);
        let fetched = provider.fetch_order(&created.id).await.unwrap();
        assert_eq!(fetched.status, ProviderOrderStatus::Attempted);
    }

    #[tokio::test]
    async fn refunds_are_idempotent_and_bounded_by_what_was_paid() {
        let provider = FakeProvider::new();
        let created = provider.create_order(&order("r1", 500)).await.unwrap();
        let payment = provider.capture(&created.id).unwrap();
        let request = |key: &str, amount| RefundRequest {
            payment_id: payment.id.clone(),
            amount_paise: amount,
            idempotency_key: key.into(),
            notes: BTreeMap::new(),
        };
        let first = provider.refund(&request("k1", 200)).await.unwrap();
        let again = provider.refund(&request("k1", 200)).await.unwrap();
        assert_eq!(first.id, again.id);
        assert_eq!(provider.refunds().len(), 1);
        assert!(provider.refund(&request("k2", 400)).await.is_err());
        provider.refund(&request("k3", 300)).await.unwrap();
        let settled = provider.fetch_payment(&payment.id).await.unwrap();
        assert_eq!(settled.status, ProviderPaymentStatus::Refunded);
        assert!(provider.refund(&request("k4", 1)).await.is_err());
    }

    #[tokio::test]
    async fn held_refunds_stay_pending_until_the_bank_finishes_or_gives_up() {
        let provider = FakeProvider::new();
        provider.hold_refunds(true);
        let created = provider.create_order(&order("r1", 500)).await.unwrap();
        let payment = provider.capture(&created.id).unwrap();
        let request = |key: &str| RefundRequest {
            payment_id: payment.id.clone(),
            amount_paise: 500,
            idempotency_key: key.into(),
            notes: BTreeMap::new(),
        };

        let pending = provider.refund(&request("k1")).await.unwrap();
        assert_eq!(pending.status, ProviderRefundStatus::Pending);
        assert_eq!(
            provider.fetch_refund(&pending.id).await.unwrap().status,
            ProviderRefundStatus::Pending
        );
        let done = provider.settle_refund(&pending.id).unwrap();
        assert_eq!(done.status, ProviderRefundStatus::Processed);
        assert_eq!(
            provider.fetch_refund(&pending.id).await.unwrap().status,
            ProviderRefundStatus::Processed
        );

        // A second payment's refund that fails gives the money back to the payment.
        let second_order = provider.create_order(&order("r2", 700)).await.unwrap();
        let second = provider.capture(&second_order.id).unwrap();
        let failing = provider
            .refund(&RefundRequest {
                payment_id: second.id.clone(),
                amount_paise: 700,
                idempotency_key: "k2".into(),
                notes: BTreeMap::new(),
            })
            .await
            .unwrap();
        provider.fail_refund(&failing.id).unwrap();
        assert_eq!(
            provider.fetch_refund(&failing.id).await.unwrap().status,
            ProviderRefundStatus::Failed
        );
        let payment_now = provider.fetch_payment(&second.id).await.unwrap();
        assert_eq!(payment_now.status, ProviderPaymentStatus::Captured);
        assert_eq!(payment_now.amount_refunded_paise, 0);
        assert_eq!(
            provider.fetch_refund("rfnd_nope").await,
            Err(ProviderError::NotFound)
        );
    }

    #[tokio::test]
    async fn an_orders_payments_include_failed_attempts_and_only_that_orders() {
        let provider = FakeProvider::new();
        let a = provider.create_order(&order("ra", 500)).await.unwrap();
        let b = provider.create_order(&order("rb", 600)).await.unwrap();
        provider.fail_payment(&a.id).unwrap();
        provider.capture(&a.id).unwrap();
        provider.capture(&b.id).unwrap();
        let payments = provider.fetch_order_payments(&a.id).await.unwrap();
        assert_eq!(payments.len(), 2);
        assert!(payments
            .iter()
            .all(|p| p.order_id.as_deref() == Some(a.id.as_str())));
        assert_eq!(
            provider.fetch_order_payments("order_missing").await,
            Err(ProviderError::NotFound)
        );
    }

    #[tokio::test]
    async fn webhooks_are_signed_with_the_providers_own_secret() {
        let provider = FakeProvider::new();
        let body = br#"{"event":"payment.captured"}"#;
        let signature = provider.sign_webhook(body);
        assert!(provider.verify_webhook(body, &signature));
        assert!(!provider.verify_webhook(b"{}", &signature));
        assert!(!FakeProvider::with_webhook_secret("other").verify_webhook(body, &signature));
    }
}
