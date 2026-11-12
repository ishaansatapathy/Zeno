//! Razorpay's REST API (<https://razorpay.com/docs/api/>) behind [`PaymentProvider`].
//!
//! Choices that matter:
//!
//! * **Short, bounded calls.** A connect timeout and a total timeout; a hung provider must not
//!   hold a request open for minutes.
//! * **Reads retry, writes do not.** `GET`s are retried a couple of times with a short back-off
//!   on timeouts, 5xx and 429. `POST /orders` is *not*: a retry after a timeout could create a
//!   second order, so the caller looks the order up by receipt instead.
//! * **No redirects.** The request carries our API secret; following a redirect to another host
//!   would hand it over.
//! * **Nothing sensitive in errors.** Errors carry the provider's error code and a truncated
//!   description, never the request URL, headers or the response body.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::{Method, StatusCode};
use serde_json::{json, Value};

use super::{
    webhook, CreateOrder, PaymentProvider, ProviderError, ProviderOrder, ProviderOrderStatus,
    ProviderPayment, ProviderPaymentStatus, ProviderRefund, ProviderRefundStatus, RefundRequest,
    REQUEST_TIMEOUT,
};
use crate::secret::Secret;

const DEFAULT_API_BASE: &str = "https://api.razorpay.com";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const READ_RETRIES: u32 = 2;
const DEFAULT_BACKOFF: Duration = Duration::from_millis(250);
/// Razorpay documents a 40-character limit on receipts.
const MAX_RECEIPT_CHARS: usize = 40;
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_DESCRIPTION_CHARS: usize = 200;

#[derive(Debug, Clone)]
pub struct RazorpayConfig {
    pub key_id: String,
    pub key_secret: Secret,
    /// Secrets that may have signed an incoming webhook: the current one first, then the
    /// previous one while a rotation is in progress.
    pub webhook_secrets: Vec<Secret>,
    /// Without a trailing slash. Only tests and sandboxes change this.
    pub api_base: String,
    pub request_timeout: Duration,
    pub retry_backoff: Duration,
}

impl RazorpayConfig {
    pub fn new(
        key_id: impl Into<String>,
        key_secret: Secret,
        webhook_secrets: Vec<Secret>,
    ) -> Self {
        Self {
            key_id: key_id.into(),
            key_secret,
            webhook_secrets,
            api_base: DEFAULT_API_BASE.to_owned(),
            request_timeout: REQUEST_TIMEOUT,
            retry_backoff: DEFAULT_BACKOFF,
        }
    }
}

#[derive(Clone)]
pub struct RazorpayProvider {
    http: reqwest::Client,
    base: String,
    key_id: String,
    key_secret: Secret,
    webhook_secrets: Vec<Secret>,
    backoff: Duration,
}

impl std::fmt::Debug for RazorpayProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RazorpayProvider")
            .field("base", &self.base)
            .field("key_id", &self.key_id)
            .finish_non_exhaustive()
    }
}

impl RazorpayProvider {
    pub fn new(config: RazorpayConfig) -> Result<Self, ProviderError> {
        // reqwest is built without a bundled TLS provider; installing ring is idempotent.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(config.request_timeout)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("zeno-api")
            .build()
            .map_err(|e| ProviderError::Unavailable(e.without_url().to_string()))?;
        Ok(Self {
            http,
            base: config.api_base.trim_end_matches('/').to_owned(),
            key_id: config.key_id,
            key_secret: config.key_secret,
            webhook_secrets: config.webhook_secrets,
            backoff: config.retry_backoff,
        })
    }

    async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<Vec<u8>>,
        headers: &[(&str, &str)],
    ) -> Result<Value, ProviderError> {
        let mut request = self
            .http
            .request(method, format!("{}{path}", self.base))
            .basic_auth(&self.key_id, Some(self.key_secret.expose()))
            .header("accept", "application/json");
        if let Some(body) = body {
            request = request
                .header("content-type", "application/json")
                .body(body);
        }
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = request.send().await.map_err(transport_error)?;
        let status = response.status();
        let bytes = response.bytes().await.map_err(transport_error)?;
        if bytes.len() > MAX_RESPONSE_BYTES {
            return Err(ProviderError::Malformed("response too large".into()));
        }
        classify(status, &bytes)
    }

    /// A read: safe to repeat, so transient failures are retried.
    async fn get(&self, path: &str) -> Result<Value, ProviderError> {
        let mut attempt = 0;
        loop {
            match self.send(Method::GET, path, None, &[]).await {
                Err(error) if error.is_transient() && attempt < READ_RETRIES => {
                    attempt += 1;
                    tokio::time::sleep(self.backoff * attempt).await;
                }
                other => return other,
            }
        }
    }
}

fn transport_error(error: reqwest::Error) -> ProviderError {
    // Checked first: a connect attempt that times out is reported as both, and nothing was sent.
    if error.is_connect() {
        ProviderError::Unreachable(error.without_url().to_string())
    } else if error.is_timeout() {
        ProviderError::Timeout
    } else {
        ProviderError::Unavailable(error.without_url().to_string())
    }
}

fn classify(status: StatusCode, body: &[u8]) -> Result<Value, ProviderError> {
    if status.is_success() {
        return serde_json::from_slice(body)
            .map_err(|_| ProviderError::Malformed("body is not JSON".into()));
    }
    match status.as_u16() {
        401 | 403 => Err(ProviderError::Unauthorized),
        404 => Err(ProviderError::NotFound),
        429 => Err(ProviderError::RateLimited),
        500..=599 => Err(ProviderError::Unavailable(format!(
            "HTTP {}",
            status.as_u16()
        ))),
        _ => {
            let parsed: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
            let code = parsed["error"]["code"]
                .as_str()
                .unwrap_or("UNKNOWN")
                .to_owned();
            let description: String = parsed["error"]["description"]
                .as_str()
                .unwrap_or("")
                .chars()
                .take(MAX_DESCRIPTION_CHARS)
                .collect();
            // Razorpay answers 400, not 404, for an id it has never issued.
            if description.to_ascii_lowercase().contains("does not exist") {
                return Err(ProviderError::NotFound);
            }
            Err(ProviderError::Rejected { code, description })
        }
    }
}

/// Provider ids are interpolated into URL paths, so only the characters Razorpay issues pass.
fn safe_id(id: &str) -> Result<&str, ProviderError> {
    if !id.is_empty()
        && id.len() <= 64
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        Ok(id)
    } else {
        Err(ProviderError::Rejected {
            code: "INVALID_ID".into(),
            description: "not a provider id".into(),
        })
    }
}

fn malformed(what: &str) -> ProviderError {
    ProviderError::Malformed(format!("missing or invalid {what}"))
}

fn text(value: &Value, field: &str) -> Result<String, ProviderError> {
    value[field]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| malformed(field))
}

fn integer(value: &Value, field: &str) -> Result<i64, ProviderError> {
    value[field].as_i64().ok_or_else(|| malformed(field))
}

fn parse_order(value: &Value) -> Result<ProviderOrder, ProviderError> {
    let status = match value["status"].as_str() {
        Some("created") => ProviderOrderStatus::Created,
        Some("attempted") => ProviderOrderStatus::Attempted,
        Some("paid") => ProviderOrderStatus::Paid,
        _ => return Err(malformed("order status")),
    };
    Ok(ProviderOrder {
        id: text(value, "id")?,
        amount_paise: integer(value, "amount")?,
        amount_paid_paise: value["amount_paid"].as_i64().unwrap_or(0),
        currency: text(value, "currency")?,
        receipt: value["receipt"].as_str().map(str::to_owned),
        status,
    })
}

fn parse_payment(value: &Value) -> Result<ProviderPayment, ProviderError> {
    let status = match value["status"].as_str() {
        Some("created") => ProviderPaymentStatus::Created,
        Some("authorized") => ProviderPaymentStatus::Authorized,
        Some("captured") => ProviderPaymentStatus::Captured,
        Some("refunded") => ProviderPaymentStatus::Refunded,
        Some("failed") => ProviderPaymentStatus::Failed,
        _ => return Err(malformed("payment status")),
    };
    Ok(ProviderPayment {
        id: text(value, "id")?,
        order_id: value["order_id"].as_str().map(str::to_owned),
        amount_paise: integer(value, "amount")?,
        amount_refunded_paise: value["amount_refunded"].as_i64().unwrap_or(0),
        currency: text(value, "currency")?,
        status,
        fee_paise: value["fee"].as_i64().filter(|fee| *fee >= 0),
    })
}

fn parse_refund(value: &Value) -> Result<ProviderRefund, ProviderError> {
    let status = match value["status"].as_str() {
        Some("pending") => ProviderRefundStatus::Pending,
        Some("processed") => ProviderRefundStatus::Processed,
        Some("failed") => ProviderRefundStatus::Failed,
        _ => return Err(malformed("refund status")),
    };
    Ok(ProviderRefund {
        id: text(value, "id")?,
        payment_id: text(value, "payment_id")?,
        amount_paise: integer(value, "amount")?,
        status,
    })
}

fn check_receipt(receipt: &str) -> Result<(), ProviderError> {
    if receipt.is_empty() || receipt.chars().count() > MAX_RECEIPT_CHARS {
        return Err(ProviderError::Rejected {
            code: "INVALID_RECEIPT".into(),
            description: format!("a receipt is 1 to {MAX_RECEIPT_CHARS} characters"),
        });
    }
    Ok(())
}

#[async_trait]
impl PaymentProvider for RazorpayProvider {
    fn name(&self) -> &'static str {
        "razorpay"
    }

    fn checkout_key(&self) -> String {
        self.key_id.clone()
    }

    async fn create_order(&self, order: &CreateOrder) -> Result<ProviderOrder, ProviderError> {
        check_receipt(&order.receipt)?;
        let body = json!({
            "amount": order.amount_paise,
            "currency": order.currency,
            "receipt": order.receipt,
            "partial_payment": false,
            "notes": order.notes,
        });
        let body = serde_json::to_vec(&body).map_err(|_| malformed("request"))?;
        // No retry: see the module docs.
        let response = self
            .send(Method::POST, "/v1/orders", Some(body), &[])
            .await?;
        parse_order(&response)
    }

    async fn find_order_by_receipt(
        &self,
        receipt: &str,
    ) -> Result<Option<ProviderOrder>, ProviderError> {
        check_receipt(receipt)?;
        let query: String = url::form_urlencoded::byte_serialize(receipt.as_bytes()).collect();
        let response = self
            .get(&format!("/v1/orders?receipt={query}&count=100"))
            .await?;
        let items = response["items"]
            .as_array()
            .ok_or_else(|| malformed("order list"))?;
        let mut found: Vec<(i64, ProviderOrder)> = Vec::new();
        for item in items {
            let order = parse_order(item)?;
            // The filter is the provider's; do not take its word for it.
            if order.receipt.as_deref() == Some(receipt) {
                found.push((item["created_at"].as_i64().unwrap_or(i64::MAX), order));
            }
        }
        Ok(found
            .into_iter()
            .min_by_key(|(created_at, _)| *created_at)
            .map(|(_, order)| order))
    }

    async fn fetch_order(&self, provider_order_id: &str) -> Result<ProviderOrder, ProviderError> {
        let id = safe_id(provider_order_id)?;
        parse_order(&self.get(&format!("/v1/orders/{id}")).await?)
    }

    async fn fetch_payment(
        &self,
        provider_payment_id: &str,
    ) -> Result<ProviderPayment, ProviderError> {
        let id = safe_id(provider_payment_id)?;
        parse_payment(&self.get(&format!("/v1/payments/{id}")).await?)
    }

    async fn fetch_order_payments(
        &self,
        provider_order_id: &str,
    ) -> Result<Vec<ProviderPayment>, ProviderError> {
        let id = safe_id(provider_order_id)?;
        let response = self.get(&format!("/v1/orders/{id}/payments")).await?;
        response["items"]
            .as_array()
            .ok_or_else(|| malformed("payment list"))?
            .iter()
            .map(parse_payment)
            .collect()
    }

    async fn fetch_refund(
        &self,
        provider_refund_id: &str,
    ) -> Result<ProviderRefund, ProviderError> {
        let id = safe_id(provider_refund_id)?;
        parse_refund(&self.get(&format!("/v1/refunds/{id}")).await?)
    }

    async fn refund(&self, request: &RefundRequest) -> Result<ProviderRefund, ProviderError> {
        let id = safe_id(&request.payment_id)?;
        let mut body = json!({
            "amount": request.amount_paise,
            "speed": "normal",
            "notes": request.notes,
        });
        if request.idempotency_key.chars().count() <= MAX_RECEIPT_CHARS {
            body["receipt"] = json!(request.idempotency_key);
        }
        let body = serde_json::to_vec(&body).map_err(|_| malformed("request"))?;
        let response = self
            .send(
                Method::POST,
                &format!("/v1/payments/{id}/refund"),
                Some(body),
                &[("x-refund-idempotency", request.idempotency_key.as_str())],
            )
            .await?;
        parse_refund(&response)
    }

    fn verify_webhook(&self, body: &[u8], signature: &str) -> bool {
        webhook::verify(&self.webhook_secrets, body, signature)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_and_provider_never_show_the_secret() {
        let config = RazorpayConfig::new(
            "rzp_test_abc",
            Secret::new("super-secret-api-key"),
            vec![Secret::new("webhook-secret-value")],
        );
        let provider = RazorpayProvider::new(config.clone()).unwrap();
        for shown in [format!("{config:?}"), format!("{provider:?}")] {
            assert!(!shown.contains("super-secret-api-key"), "{shown}");
            assert!(!shown.contains("webhook-secret-value"), "{shown}");
        }
    }

    #[test]
    fn ids_that_could_change_the_url_are_refused() {
        for bad in [
            "",
            "../x",
            "order_1/refund",
            "order_1?x=1",
            "a b",
            "order_é",
        ] {
            assert!(safe_id(bad).is_err(), "{bad:?}");
        }
        assert_eq!(safe_id("order_Abc123").unwrap(), "order_Abc123");
    }

    #[test]
    fn error_statuses_are_grouped_by_what_to_do() {
        let body = br#"{"error":{"code":"BAD_REQUEST_ERROR","description":"The amount must be at least INR 1.00"}}"#;
        assert_eq!(
            classify(StatusCode::BAD_REQUEST, body),
            Err(ProviderError::Rejected {
                code: "BAD_REQUEST_ERROR".into(),
                description: "The amount must be at least INR 1.00".into()
            })
        );
        assert_eq!(
            classify(StatusCode::UNAUTHORIZED, b"{}"),
            Err(ProviderError::Unauthorized)
        );
        assert_eq!(
            classify(StatusCode::NOT_FOUND, b""),
            Err(ProviderError::NotFound)
        );
        assert_eq!(
            classify(StatusCode::TOO_MANY_REQUESTS, b""),
            Err(ProviderError::RateLimited)
        );
        assert!(matches!(
            classify(StatusCode::BAD_GATEWAY, b"<html>nginx</html>"),
            Err(ProviderError::Unavailable(_))
        ));
        let missing = br#"{"error":{"code":"BAD_REQUEST_ERROR","description":"The id provided does not exist"}}"#;
        assert_eq!(
            classify(StatusCode::BAD_REQUEST, missing),
            Err(ProviderError::NotFound)
        );
        assert!(matches!(
            classify(StatusCode::OK, b"<html>"),
            Err(ProviderError::Malformed(_))
        ));
    }

    #[test]
    fn an_error_description_is_truncated() {
        let long = "x".repeat(5000);
        let body = format!(r#"{{"error":{{"code":"X","description":"{long}"}}}}"#);
        let Err(ProviderError::Rejected { description, .. }) =
            classify(StatusCode::BAD_REQUEST, body.as_bytes())
        else {
            panic!("expected a rejection");
        };
        assert_eq!(description.chars().count(), MAX_DESCRIPTION_CHARS);
    }

    #[test]
    fn documented_objects_parse_and_odd_ones_do_not() {
        let order = parse_order(&json!({
            "id": "order_1", "entity": "order", "amount": 49900, "amount_paid": 0,
            "amount_due": 49900, "currency": "INR", "receipt": "r-1", "status": "created"
        }))
        .unwrap();
        assert_eq!(order.amount_paise, 49900);
        assert_eq!(order.status, ProviderOrderStatus::Created);
        assert!(parse_order(
            &json!({"id": "order_1", "amount": 1, "currency": "INR", "status": "weird"})
        )
        .is_err());
        assert!(parse_order(
            &json!({"id": "order_1", "amount": "499", "currency": "INR", "status": "created"})
        )
        .is_err());
        assert!(
            parse_order(&json!({"amount": 1, "currency": "INR", "status": "created"})).is_err()
        );
    }

    #[test]
    fn a_fee_the_provider_reports_is_kept_only_if_it_makes_sense() {
        let payment = |fee: Value| {
            let mut value = json!({
                "id": "pay_1", "order_id": "order_1", "amount": 49900, "currency": "INR",
                "status": "captured"
            });
            value["fee"] = fee;
            parse_payment(&value).unwrap().fee_paise
        };
        assert_eq!(payment(json!(1180)), Some(1180));
        assert_eq!(payment(json!(0)), Some(0));
        assert_eq!(payment(json!(-1)), None);
        assert_eq!(payment(json!("1180")), None);
        assert_eq!(payment(json!(null)), None);
    }

    #[test]
    fn long_or_empty_receipts_never_leave_the_process() {
        assert!(check_receipt("").is_err());
        assert!(check_receipt(&"a".repeat(41)).is_err());
        assert!(check_receipt(&"a".repeat(40)).is_ok());
    }
}
