//! The Razorpay client against a local stand-in for Razorpay's API: what it sends, how it reads
//! answers, and what it does when the provider is slow, down or odd. No network is involved.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::to_bytes;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Router;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use zeno_core::payments::{
    CreateOrder, PaymentProvider, ProviderError, ProviderOrderStatus, ProviderPaymentStatus,
    ProviderRefundStatus, RazorpayConfig, RazorpayProvider, RefundRequest,
};
use zeno_core::secret::Secret;

const KEY_ID: &str = "rzp_test_AbC123";
const KEY_SECRET: &str = "super-secret-api-key-value";

#[derive(Clone)]
struct Reply {
    status: u16,
    body: String,
    delay: Duration,
    location: Option<String>,
}

fn reply(status: u16, body: Value) -> Reply {
    Reply {
        status,
        body: body.to_string(),
        delay: Duration::ZERO,
        location: None,
    }
}

#[derive(Debug, Clone)]
struct Seen {
    method: String,
    target: String,
    authorization: Option<String>,
    content_type: Option<String>,
    refund_key: Option<String>,
    body: Vec<u8>,
}

#[derive(Default)]
struct Mock {
    script: Mutex<VecDeque<Reply>>,
    seen: Mutex<Vec<Seen>>,
}

impl Mock {
    fn push(&self, reply: Reply) {
        self.script.lock().unwrap().push_back(reply);
    }
    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

async fn handle(State(mock): State<Arc<Mock>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let header = |name: &str| {
        parts
            .headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    let body = to_bytes(body, 1 << 20).await.unwrap().to_vec();
    mock.seen.lock().unwrap().push(Seen {
        method: parts.method.to_string(),
        target: parts
            .uri
            .path_and_query()
            .map(|p| p.as_str().to_owned())
            .unwrap_or_default(),
        authorization: header("authorization"),
        content_type: header("content-type"),
        refund_key: header("x-refund-idempotency"),
        body,
    });
    let next = mock.script.lock().unwrap().pop_front();
    let reply = next.unwrap_or_else(|| reply(500, json!({})));
    tokio::time::sleep(reply.delay).await;
    let mut response = (
        StatusCode::from_u16(reply.status).unwrap(),
        [("content-type", "application/json")],
        reply.body,
    )
        .into_response();
    if let Some(location) = reply.location {
        response
            .headers_mut()
            .insert("location", location.parse().unwrap());
    }
    response
}

async fn serve() -> (String, Arc<Mock>) {
    let mock = Arc::new(Mock::default());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new().fallback(handle).with_state(mock.clone());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (base, mock)
}

fn provider(base: &str) -> RazorpayProvider {
    let mut config = RazorpayConfig::new(
        KEY_ID,
        Secret::new(KEY_SECRET),
        vec![Secret::new("webhook-secret-of-sufficient-length")],
    );
    config.api_base = base.to_owned();
    config.request_timeout = Duration::from_millis(400);
    config.retry_backoff = Duration::from_millis(1);
    RazorpayProvider::new(config).unwrap()
}

fn order_json(id: &str, receipt: &str, status: &str, created_at: i64) -> Value {
    json!({
        "id": id, "entity": "order", "amount": 49900, "amount_paid": 0, "amount_due": 49900,
        "currency": "INR", "receipt": receipt, "status": status, "attempts": 0,
        "notes": [], "created_at": created_at
    })
}

fn new_order(receipt: &str) -> CreateOrder {
    CreateOrder {
        receipt: receipt.into(),
        amount_paise: 49_900,
        currency: "INR".into(),
        notes: BTreeMap::from([("zeno_order_id".to_owned(), receipt.to_owned())]),
    }
}

#[tokio::test]
async fn creating_an_order_sends_exactly_what_the_api_documents() {
    let (base, mock) = serve().await;
    mock.push(reply(200, order_json("order_1", "r-1", "created", 100)));

    let created = provider(&base)
        .create_order(&new_order("r-1"))
        .await
        .unwrap();

    assert_eq!(created.id, "order_1");
    assert_eq!(created.amount_paise, 49_900);
    assert_eq!(created.status, ProviderOrderStatus::Created);
    assert_eq!(created.receipt.as_deref(), Some("r-1"));

    let seen = mock.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        (seen[0].method.as_str(), seen[0].target.as_str()),
        ("POST", "/v1/orders")
    );
    let expected = format!(
        "Basic {}",
        STANDARD.encode(format!("{KEY_ID}:{KEY_SECRET}"))
    );
    assert_eq!(seen[0].authorization.as_deref(), Some(expected.as_str()));
    assert_eq!(seen[0].content_type.as_deref(), Some("application/json"));
    let body: Value = serde_json::from_slice(&seen[0].body).unwrap();
    // The amount is an integer number of paise, never a string or a float.
    assert_eq!(body["amount"], json!(49_900));
    assert!(body["amount"].is_i64());
    assert_eq!(body["currency"], "INR");
    assert_eq!(body["receipt"], "r-1");
    assert_eq!(body["partial_payment"], false);
    assert_eq!(body["notes"]["zeno_order_id"], "r-1");
}

#[tokio::test]
async fn creating_an_order_is_never_retried() {
    let (base, mock) = serve().await;
    mock.push(reply(503, json!({})));
    mock.push(reply(200, order_json("order_1", "r-1", "created", 100)));

    let result = provider(&base).create_order(&new_order("r-1")).await;

    assert!(
        matches!(result, Err(ProviderError::Unavailable(_))),
        "{result:?}"
    );
    assert_eq!(
        mock.seen().len(),
        1,
        "a retried create could make two orders"
    );
}

#[tokio::test]
async fn reads_are_retried_through_transient_failures() {
    let (base, mock) = serve().await;
    mock.push(reply(503, json!({})));
    mock.push(reply(429, json!({})));
    mock.push(reply(200, order_json("order_1", "r-1", "attempted", 100)));

    let order = provider(&base).fetch_order("order_1").await.unwrap();

    assert_eq!(order.status, ProviderOrderStatus::Attempted);
    assert_eq!(mock.seen().len(), 3);
    assert!(mock
        .seen()
        .iter()
        .all(|s| s.method == "GET" && s.target == "/v1/orders/order_1"));
}

#[tokio::test]
async fn reads_give_up_after_three_attempts() {
    let (base, mock) = serve().await;
    for _ in 0..5 {
        mock.push(reply(502, json!({})));
    }
    let result = provider(&base).fetch_order("order_1").await;
    assert!(
        matches!(result, Err(ProviderError::Unavailable(_))),
        "{result:?}"
    );
    assert_eq!(mock.seen().len(), 3);
}

#[tokio::test]
async fn definite_answers_are_not_retried() {
    let (base, mock) = serve().await;
    let provider = provider(&base);

    mock.push(reply(
        401,
        json!({"error": {"code": "BAD_REQUEST_ERROR", "description": "Authentication failed"}}),
    ));
    assert_eq!(
        provider.fetch_order("order_1").await,
        Err(ProviderError::Unauthorized)
    );

    mock.push(reply(
        400,
        json!({"error": {"code": "BAD_REQUEST_ERROR", "description": "The id provided does not exist"}}),
    ));
    assert_eq!(
        provider.fetch_order("order_missing").await,
        Err(ProviderError::NotFound)
    );

    mock.push(reply(
        400,
        json!({"error": {"code": "BAD_REQUEST_ERROR", "description": "The amount must be at least INR 1.00"}}),
    ));
    assert_eq!(
        provider.create_order(&new_order("r-1")).await,
        Err(ProviderError::Rejected {
            code: "BAD_REQUEST_ERROR".into(),
            description: "The amount must be at least INR 1.00".into()
        })
    );
    assert_eq!(mock.seen().len(), 3, "one call each");
}

#[tokio::test]
async fn a_slow_provider_times_out_instead_of_hanging() {
    let (base, mock) = serve().await;
    let mut slow = reply(200, order_json("order_1", "r-1", "created", 100));
    slow.delay = Duration::from_secs(3);
    mock.push(slow);

    let started = std::time::Instant::now();
    let result = provider(&base).create_order(&new_order("r-1")).await;

    assert_eq!(result, Err(ProviderError::Timeout));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    assert!(ProviderError::Timeout.outcome_unknown());
}

#[tokio::test]
async fn an_unreachable_provider_is_reported_as_not_having_been_reached() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);

    // Windows takes about two seconds to give up on a closed port; allow for that.
    let mut config = RazorpayConfig::new(KEY_ID, Secret::new(KEY_SECRET), vec![]);
    config.api_base = base;
    config.request_timeout = Duration::from_secs(8);
    let result = RazorpayProvider::new(config)
        .unwrap()
        .create_order(&new_order("r-1"))
        .await;

    let Err(error) = result else {
        panic!("nothing is listening");
    };
    assert!(matches!(error, ProviderError::Unreachable(_)), "{error:?}");
    assert!(
        !error.outcome_unknown(),
        "a refused connection means nothing was processed"
    );
}

#[tokio::test]
async fn answers_that_are_not_what_the_api_documents_are_rejected() {
    let (base, mock) = serve().await;
    let provider = provider(&base);

    mock.push(Reply {
        body: "<html>maintenance</html>".into(),
        ..reply(200, json!({}))
    });
    assert!(matches!(
        provider.create_order(&new_order("r-1")).await,
        Err(ProviderError::Malformed(_))
    ));

    // A string where an integer amount belongs.
    mock.push(reply(
        200,
        json!({"id": "order_1", "amount": "49900", "currency": "INR", "status": "created"}),
    ));
    assert!(matches!(
        provider.create_order(&new_order("r-1")).await,
        Err(ProviderError::Malformed(_))
    ));

    // A status the client has never heard of.
    mock.push(reply(
        200,
        json!({"id": "order_1", "amount": 49900, "currency": "INR", "status": "teleported"}),
    ));
    assert!(matches!(
        provider.create_order(&new_order("r-1")).await,
        Err(ProviderError::Malformed(_))
    ));

    // Far more than any real answer.
    mock.push(Reply {
        body: "x".repeat(2 * 1024 * 1024),
        ..reply(200, json!({}))
    });
    assert!(matches!(
        provider.create_order(&new_order("r-1")).await,
        Err(ProviderError::Malformed(_))
    ));
}

#[tokio::test]
async fn redirects_are_not_followed_so_the_api_secret_stays_put() {
    let (elsewhere, elsewhere_mock) = serve().await;
    let (base, mock) = serve().await;
    mock.push(Reply {
        location: Some(format!("{elsewhere}/steal")),
        ..reply(302, json!({}))
    });

    let result = provider(&base).create_order(&new_order("r-1")).await;

    assert!(result.is_err());
    assert!(
        elsewhere_mock.seen().is_empty(),
        "the client followed a redirect carrying our credentials"
    );
}

#[tokio::test]
async fn finding_an_order_by_receipt_picks_the_oldest_exact_match() {
    let (base, mock) = serve().await;
    mock.push(reply(
        200,
        json!({"entity": "collection", "count": 4, "items": [
            order_json("order_new", "r-1", "created", 300),
            order_json("order_old", "r-1", "created", 100),
            order_json("order_other", "r-10", "created", 50),
            order_json("order_mid", "r-1", "created", 200),
        ]}),
    ));

    let found = provider(&base).find_order_by_receipt("r-1").await.unwrap();

    assert_eq!(found.unwrap().id, "order_old");
    let seen = mock.seen();
    assert_eq!(seen[0].method, "GET");
    assert!(
        seen[0].target.starts_with("/v1/orders?receipt=r-1&"),
        "{}",
        seen[0].target
    );
}

#[tokio::test]
async fn finding_nothing_is_not_an_error_and_receipts_are_url_encoded() {
    let (base, mock) = serve().await;
    mock.push(reply(
        200,
        json!({"entity": "collection", "count": 0, "items": []}),
    ));

    let found = provider(&base)
        .find_order_by_receipt("a&b=c d")
        .await
        .unwrap();

    assert!(found.is_none());
    let target = &mock.seen()[0].target;
    assert!(
        target.starts_with("/v1/orders?receipt=a%26b%3Dc+d&"),
        "{target}"
    );
}

#[tokio::test]
async fn ids_that_could_rewrite_the_path_never_reach_the_network() {
    let (base, mock) = serve().await;
    let provider = provider(&base);
    for bad in ["../v1/payments", "order_1/../x", "a?b=c", ""] {
        assert!(matches!(
            provider.fetch_order(bad).await,
            Err(ProviderError::Rejected { .. })
        ));
        assert!(matches!(
            provider.fetch_payment(bad).await,
            Err(ProviderError::Rejected { .. })
        ));
    }
    assert!(mock.seen().is_empty());
}

#[tokio::test]
async fn payments_and_refunds_parse_and_carry_the_idempotency_key() {
    let (base, mock) = serve().await;
    let provider = provider(&base);

    mock.push(reply(
        200,
        json!({"id": "pay_1", "entity": "payment", "amount": 49900, "currency": "INR",
               "status": "captured", "order_id": "order_1", "amount_refunded": 0}),
    ));
    let payment = provider.fetch_payment("pay_1").await.unwrap();
    assert_eq!(payment.status, ProviderPaymentStatus::Captured);
    assert_eq!(payment.order_id.as_deref(), Some("order_1"));

    mock.push(reply(
        200,
        json!({"id": "rfnd_1", "entity": "refund", "amount": 10000, "payment_id": "pay_1",
               "status": "processed", "currency": "INR"}),
    ));
    let refund = provider
        .refund(&RefundRequest {
            payment_id: "pay_1".into(),
            amount_paise: 10_000,
            idempotency_key: "refund-0001".into(),
            notes: BTreeMap::new(),
        })
        .await
        .unwrap();
    assert_eq!(refund.status, ProviderRefundStatus::Processed);
    assert_eq!(refund.amount_paise, 10_000);

    let seen = mock.seen();
    let sent = &seen[1];
    assert_eq!(
        (sent.method.as_str(), sent.target.as_str()),
        ("POST", "/v1/payments/pay_1/refund")
    );
    assert_eq!(sent.refund_key.as_deref(), Some("refund-0001"));
    let body: Value = serde_json::from_slice(&sent.body).unwrap();
    assert_eq!(body["amount"], json!(10_000));
    assert_eq!(body["speed"], "normal");
}

#[tokio::test]
async fn errors_never_contain_the_credentials() {
    let (base, mock) = serve().await;
    let provider = provider(&base);
    mock.push(reply(500, json!({"detail": KEY_SECRET})));
    mock.push(reply(
        400,
        json!({"error": {"code": "X", "description": "bad"}}),
    ));
    mock.push(Reply {
        body: format!("not json {KEY_SECRET}"),
        ..reply(200, json!({}))
    });
    let errors = [
        provider.create_order(&new_order("r-1")).await.unwrap_err(),
        provider.create_order(&new_order("r-1")).await.unwrap_err(),
        provider.create_order(&new_order("r-1")).await.unwrap_err(),
    ];
    for error in errors {
        let shown = format!("{error} {error:?}");
        assert!(!shown.contains(KEY_SECRET), "{shown}");
        assert!(!shown.contains(&base), "{shown}");
    }
    assert!(!format!("{provider:?}").contains(KEY_SECRET));
}

#[tokio::test]
async fn webhook_verification_uses_the_configured_secrets() {
    let (base, _mock) = serve().await;
    let provider = provider(&base);
    let body = br#"{"event":"payment.captured"}"#;
    let good = zeno_core::payments::webhook::sign("webhook-secret-of-sufficient-length", body);
    assert!(provider.verify_webhook(body, &good));
    assert!(!provider.verify_webhook(body, &zeno_core::payments::webhook::sign("another", body)));
}
