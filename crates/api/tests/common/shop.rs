//! A shop with one paid course and one buyer who has started checkout, and the provider's
//! payloads, for tests of everything that happens after a customer pays.
#![allow(dead_code)]

use axum::http::StatusCode;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;
use zeno_core::payments::FakeProvider;

use super::*;

pub const PRICE: i64 = 49_900;
pub const HOOK: &str = "/v1/webhooks/razorpay";

// ---- a shop with one paid course and one buyer who has started checkout ----------------

pub struct Shop {
    pub app: App,
    pub provider: FakeProvider,
    pub pool: PgPool,
    pub creator: Jar,
    pub buyer: Jar,
    pub buyer_id: Uuid,
    pub creator_id: Uuid,
    pub course: Uuid,
    pub video: Uuid,
    pub order: Uuid,
    /// The provider's id for the order.
    pub remote: String,
}

pub async fn user_id(pool: &PgPool, email: &str) -> Uuid {
    sqlx::query_scalar("select id from users where email=$1")
        .bind(email)
        .fetch_one(pool)
        .await
        .unwrap()
}

pub async fn make_course(app: &App, pool: &PgPool, creator: &Jar, title: &str) -> (Uuid, Uuid) {
    let response = app
        .post("/v1/courses")
        .jar(creator)
        .json(json!({"title": title, "description": "Course", "price_paise": PRICE}))
        .send()
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);
    let id = Uuid::parse_str(response.body["course"]["id"].as_str().unwrap()).unwrap();
    let owner: Uuid = sqlx::query_scalar("select creator_id from courses where id=$1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap();
    let video = Uuid::new_v4();
    sqlx::query(
        "insert into videos \
            (id, owner_id, title, original_filename, content_type, size_bytes, part_size, \
             source_key, status, duration_seconds) \
         values ($1,$2,'Lesson','lesson.mp4','video/mp4',1,5242880,$3,'ready',600)",
    )
    .bind(video)
    .bind(owner)
    .bind(format!("uploads/{video}"))
    .execute(pool)
    .await
    .unwrap();
    let lesson = app
        .post(&format!("/v1/courses/{id}/lessons"))
        .jar(creator)
        .json(json!({"title": "Lesson one", "video_id": video}))
        .send()
        .await;
    assert_eq!(lesson.status, StatusCode::CREATED, "{:?}", lesson.body);
    let version = lesson.body["course_version"].as_i64().unwrap();
    let published = app
        .post(&format!("/v1/courses/{id}/publish"))
        .jar(creator)
        .json(json!({"version": version}))
        .send()
        .await;
    assert_eq!(published.status, StatusCode::OK, "{:?}", published.body);
    (id, video)
}

pub async fn shop(pool: PgPool) -> Shop {
    let (app, provider) = app_with_provider(pool.clone());
    let creator = signup_creator(&app, "teacher@hooks.in").await;
    let buyer = signup_verified_as(&app, "buyer@hooks.in", "student").await;
    let (course, video) = make_course(&app, &pool, &creator, "Paid course").await;
    let started = app
        .post(&format!("/v1/checkout/{course}"))
        .jar(&buyer)
        .send()
        .await;
    assert_eq!(started.status, StatusCode::CREATED, "{:?}", started.body);
    Shop {
        buyer_id: user_id(&pool, "buyer@hooks.in").await,
        creator_id: user_id(&pool, "teacher@hooks.in").await,
        order: Uuid::parse_str(started.body["order"]["id"].as_str().unwrap()).unwrap(),
        remote: started.body["checkout"]["provider_order_id"]
            .as_str()
            .unwrap()
            .to_owned(),
        app,
        provider,
        pool,
        creator,
        buyer,
        course,
        video,
    }
}

// ---- the provider's payloads ---------------------------------------------------------------

pub fn payment(id: &str, order: &str, amount: i64, status: &str) -> Value {
    json!({
        "id": id, "entity": "payment", "amount": amount, "currency": "INR", "status": status,
        "order_id": order, "method": "upi", "captured": status == "captured",
        "email": "buyer@hooks.in", "contact": "+919999999999"
    })
}

pub fn event(name: &str, payload: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"entity": "event", "event": name, "payload": payload})).unwrap()
}

pub fn captured(pay: &str, order: &str, amount: i64) -> Vec<u8> {
    event(
        "payment.captured",
        json!({"payment": {"entity": payment(pay, order, amount, "captured")}}),
    )
}

pub fn order_paid(pay: &str, order: &str, amount: i64) -> Vec<u8> {
    event(
        "order.paid",
        json!({
            "payment": {"entity": payment(pay, order, amount, "captured")},
            "order": {"entity": {"id": order, "amount": amount, "amount_paid": amount, "status": "paid"}}
        }),
    )
}

pub fn failed(pay: &str, order: &str, amount: i64) -> Vec<u8> {
    let mut entity = payment(pay, order, amount, "failed");
    entity["error_code"] = json!("BAD_REQUEST_ERROR");
    entity["error_description"] = json!("Payment failed");
    event("payment.failed", json!({"payment": {"entity": entity}}))
}

pub fn refund(id: &str, pay: &str, amount: i64, status: &str) -> Value {
    json!({"id": id, "entity": "refund", "payment_id": pay, "amount": amount, "status": status})
}

pub fn refund_processed(id: &str, pay: &str, amount: i64, with_payment: Option<Value>) -> Vec<u8> {
    let mut payload = json!({"refund": {"entity": refund(id, pay, amount, "processed")}});
    if let Some(payment) = with_payment {
        payload["payment"] = json!({"entity": payment});
    }
    event("refund.processed", payload)
}

pub fn refund_failed(id: &str, pay: &str, amount: i64) -> Vec<u8> {
    event(
        "refund.failed",
        json!({"refund": {"entity": refund(id, pay, amount, "failed")}}),
    )
}

// ---- delivery -------------------------------------------------------------------------------

pub fn post_hook(app: &App, event_id: Option<&str>, body: Vec<u8>, signature: &str) -> Req {
    let mut request = app
        .post(HOOK)
        .no_origin()
        .raw_body(body, Some("application/json"))
        .header("x-razorpay-signature", signature);
    if let Some(id) = event_id {
        request = request.header("x-razorpay-event-id", id);
    }
    request
}

/// What the provider does: sign the body and send it.
pub async fn deliver(shop: &Shop, id: &str, body: Vec<u8>) -> Res {
    let signature = shop.provider.sign_webhook(&body);
    post_hook(&shop.app, Some(id), body, &signature)
        .send()
        .await
}

pub fn outcome(res: &Res) -> &str {
    res.body["outcome"].as_str().unwrap_or("")
}

pub async fn n(shop: &Shop, sql: &str) -> i64 {
    count(&shop.pool, sql).await
}

pub async fn text(shop: &Shop, sql: &str) -> String {
    sqlx::query_scalar(sql).fetch_one(&shop.pool).await.unwrap()
}

pub async fn order_status(shop: &Shop) -> String {
    text(
        shop,
        &format!("select status::text from orders where id='{}'", shop.order),
    )
    .await
}

pub async fn pay(shop: &Shop, event_id: &str, pay_id: &str) -> Res {
    deliver(shop, event_id, captured(pay_id, &shop.remote, PRICE)).await
}

pub async fn access(shop: &Shop) -> String {
    text(
        shop,
        &format!(
            "select video_access_state('{}', '{}')",
            shop.buyer_id, shop.video
        ),
    )
    .await
}
