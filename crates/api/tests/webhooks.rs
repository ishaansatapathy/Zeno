//! Payment webhooks: nothing is trusted until it is signed and matches the order, every event is
//! applied exactly once, and money received is never lost sight of.

mod common;

use axum::http::StatusCode;
use common::*;
use futures_util::future::join_all;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;
use zeno_core::payments::FakeProvider;

macro_rules! db_test {
    ($(#[$meta:meta])* async fn $name:ident($pool:ident: PgPool) $body:block) => {
        #[sqlx::test(migrator = "zeno_core::db::MIGRATOR")]
        $(#[$meta])*
        async fn $name($pool: PgPool) $body
    };
}

const PRICE: i64 = 49_900;
const HOOK: &str = "/v1/webhooks/razorpay";

// ---- a shop with one paid course and one buyer who has started checkout ----------------

struct Shop {
    app: App,
    provider: FakeProvider,
    pool: PgPool,
    buyer: Jar,
    buyer_id: Uuid,
    creator_id: Uuid,
    course: Uuid,
    video: Uuid,
    order: Uuid,
    /// The provider's id for the order.
    remote: String,
}

async fn user_id(pool: &PgPool, email: &str) -> Uuid {
    sqlx::query_scalar("select id from users where email=$1")
        .bind(email)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn make_course(app: &App, pool: &PgPool, creator: &Jar, title: &str) -> (Uuid, Uuid) {
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

async fn shop(pool: PgPool) -> Shop {
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
        buyer,
        course,
        video,
    }
}

// ---- the provider's payloads ---------------------------------------------------------------

fn payment(id: &str, order: &str, amount: i64, status: &str) -> Value {
    json!({
        "id": id, "entity": "payment", "amount": amount, "currency": "INR", "status": status,
        "order_id": order, "method": "upi", "captured": status == "captured",
        "email": "buyer@hooks.in", "contact": "+919999999999"
    })
}

fn event(name: &str, payload: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"entity": "event", "event": name, "payload": payload})).unwrap()
}

fn captured(pay: &str, order: &str, amount: i64) -> Vec<u8> {
    event(
        "payment.captured",
        json!({"payment": {"entity": payment(pay, order, amount, "captured")}}),
    )
}

fn order_paid(pay: &str, order: &str, amount: i64) -> Vec<u8> {
    event(
        "order.paid",
        json!({
            "payment": {"entity": payment(pay, order, amount, "captured")},
            "order": {"entity": {"id": order, "amount": amount, "amount_paid": amount, "status": "paid"}}
        }),
    )
}

fn failed(pay: &str, order: &str, amount: i64) -> Vec<u8> {
    let mut entity = payment(pay, order, amount, "failed");
    entity["error_code"] = json!("BAD_REQUEST_ERROR");
    entity["error_description"] = json!("Payment failed");
    event("payment.failed", json!({"payment": {"entity": entity}}))
}

fn refund(id: &str, pay: &str, amount: i64, status: &str) -> Value {
    json!({"id": id, "entity": "refund", "payment_id": pay, "amount": amount, "status": status})
}

fn refund_processed(id: &str, pay: &str, amount: i64, with_payment: Option<Value>) -> Vec<u8> {
    let mut payload = json!({"refund": {"entity": refund(id, pay, amount, "processed")}});
    if let Some(payment) = with_payment {
        payload["payment"] = json!({"entity": payment});
    }
    event("refund.processed", payload)
}

fn refund_failed(id: &str, pay: &str, amount: i64) -> Vec<u8> {
    event(
        "refund.failed",
        json!({"refund": {"entity": refund(id, pay, amount, "failed")}}),
    )
}

// ---- delivery -------------------------------------------------------------------------------

fn post_hook(app: &App, event_id: Option<&str>, body: Vec<u8>, signature: &str) -> common::Req {
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
async fn deliver(shop: &Shop, id: &str, body: Vec<u8>) -> Res {
    let signature = shop.provider.sign_webhook(&body);
    post_hook(&shop.app, Some(id), body, &signature)
        .send()
        .await
}

fn outcome(res: &Res) -> &str {
    res.body["outcome"].as_str().unwrap_or("")
}

async fn n(shop: &Shop, sql: &str) -> i64 {
    count(&shop.pool, sql).await
}

async fn text(shop: &Shop, sql: &str) -> String {
    sqlx::query_scalar(sql).fetch_one(&shop.pool).await.unwrap()
}

async fn order_status(shop: &Shop) -> String {
    text(
        shop,
        &format!("select status::text from orders where id='{}'", shop.order),
    )
    .await
}

async fn pay(shop: &Shop, event_id: &str, pay_id: &str) -> Res {
    deliver(shop, event_id, captured(pay_id, &shop.remote, PRICE)).await
}

async fn access(shop: &Shop) -> String {
    text(
        shop,
        &format!(
            "select video_access_state('{}', '{}')",
            shop.buyer_id, shop.video
        ),
    )
    .await
}

// ---- the happy path ----------------------------------------------------------------------------

db_test! {
    async fn a_captured_payment_enrolls_the_buyer_in_one_step(pool: PgPool) {
        let shop = shop(pool).await;
        assert_eq!(access(&shop).await, "access_revoked", "no access before paying");
        let mails = shop.app.mailer.sent().len();

        let res = pay(&shop, "evt_1", "pay_1").await;

        assert_eq!(res.status, StatusCode::OK, "{:?}", res.body);
        assert_eq!(outcome(&res), "enrolled");
        assert_eq!(order_status(&shop).await, "paid");
        assert_eq!(access(&shop).await, "allowed");
        // The payment, the enrollment that points at it, and the trail of how it came about.
        assert_eq!(
            text(&shop, "select status::text from payments where provider_ref='pay_1'").await,
            "confirmed"
        );
        let (source, linked): (String, bool) = sqlx::query_as(
            "select e.source::text, e.payment_id = p.id from enrollments e \
             join payments p on p.provider_ref = 'pay_1' where e.user_id = $1",
        )
        .bind(shop.buyer_id)
        .fetch_one(&shop.pool)
        .await
        .unwrap();
        assert_eq!((source.as_str(), linked), ("payment", true));
        assert_eq!(n(&shop, "select count(*) from enrollment_events where action='created' and source='payment'").await, 1);
        assert_eq!(n(&shop, "select count(*) from order_events where kind='paid'").await, 1);
        let (event_outcome, review): (String, bool) =
            sqlx::query_as("select outcome, needs_review from webhook_events")
                .fetch_one(&shop.pool)
                .await
                .unwrap();
        assert_eq!((event_outcome.as_str(), review), ("enrolled", false));

        // The buyer is told, with the amount and the course, and nothing that is not theirs.
        let receipt = shop.app.mailer.wait_for(mails + 1).await.pop().unwrap();
        assert_eq!(receipt.to, "buyer@hooks.in");
        assert!(receipt.subject.contains("Paid course"));
        assert!(receipt.text.contains("Rs 499.00"), "{}", receipt.text);
        assert!(receipt.text.contains(&shop.order.to_string()));
        assert!(!receipt.text.contains("pay_1"));
    }
}

db_test! {
    async fn the_buyer_sees_the_course_in_their_library_afterwards(pool: PgPool) {
        let shop = shop(pool).await;
        pay(&shop, "evt_1", "pay_1").await;
        let mine = shop.app.get("/v1/me/courses").jar(&shop.buyer).send().await;
        assert_eq!(mine.status, StatusCode::OK, "{:?}", mine.body);
        assert!(mine.body.to_string().contains("Paid course"), "{:?}", mine.body);
    }
}

db_test! {
    async fn the_event_that_arrives_before_the_browser_returns_is_enough(pool: PgPool) {
        // Nothing here touches the order through the API after checkout: the browser never came
        // back, and the enrollment still happens.
        let shop = shop(pool).await;
        assert_eq!(pay(&shop, "evt_1", "pay_1").await.status, StatusCode::OK);
        assert_eq!(n(&shop, "select count(*) from enrollments").await, 1);
        let read = shop.app.get(&format!("/v1/orders/{}", shop.order)).jar(&shop.buyer).send().await;
        assert_eq!(read.body["order"]["status"], "paid");
        assert!(read.body["checkout"].is_null());
    }
}

// ---- authenticity -------------------------------------------------------------------------------

db_test! {
    async fn anything_not_signed_by_the_provider_changes_nothing(pool: PgPool) {
        let shop = shop(pool).await;
        let body = captured("pay_1", &shop.remote, PRICE);
        let good = shop.provider.sign_webhook(&body);
        let other_secret = FakeProvider::with_webhook_secret("not-our-secret").sign_webhook(&body);
        let mut flipped = good.clone().into_bytes();
        flipped[7] = if flipped[7] == b'0' { b'1' } else { b'0' };
        let mut tampered = body.clone();
        let at = tampered.windows(5).position(|w| w == b"49900").unwrap();
        tampered[at] = b'1'; // 14990 instead of 49900, original signature kept

        let attempts: Vec<(&str, Vec<u8>, Option<String>)> = vec![
            ("no signature", body.clone(), None),
            ("empty signature", body.clone(), Some(String::new())),
            ("wrong secret", body.clone(), Some(other_secret)),
            ("one digit flipped", body.clone(), Some(String::from_utf8(flipped).unwrap())),
            ("truncated signature", body.clone(), Some(good[..60].to_owned())),
            ("signature plus junk", body.clone(), Some(format!("{good}00"))),
            ("not hex", body.clone(), Some("z".repeat(64))),
            ("body changed after signing", tampered, Some(good.clone())),
            ("truncated body", body[..body.len() - 3].to_vec(), Some(good.clone())),
            ("body with trailing space", [body.clone(), b" ".to_vec()].concat(), Some(good.clone())),
            ("signature of another body", body.clone(), Some(shop.provider.sign_webhook(b"{}"))),
        ];
        for (label, body, signature) in attempts {
            let mut request = shop
                .app
                .post(HOOK)
                .no_origin()
                .raw_body(body, Some("application/json"))
                .header("x-razorpay-event-id", "evt_forged");
            if let Some(signature) = &signature {
                request = request.header("x-razorpay-signature", signature);
            }
            let res = request.send().await;
            assert_eq!(res.status, StatusCode::UNAUTHORIZED, "{label}: {:?}", res.body);
            assert_eq!(res.code(), "invalid_signature", "{label}");
        }
        assert_eq!(order_status(&shop).await, "pending");
        assert_eq!(n(&shop, "select count(*) from enrollments").await, 0);
        assert_eq!(n(&shop, "select count(*) from payments").await, 0);
        assert_eq!(n(&shop, "select count(*) from webhook_events").await, 0, "a forgery is not even recorded");
    }
}

db_test! {
    async fn the_signature_covers_the_exact_bytes_so_formatting_matters(pool: PgPool) {
        let shop = shop(pool).await;
        let body = captured("pay_1", &shop.remote, PRICE);
        let signature = shop.provider.sign_webhook(&body);
        // The same JSON, pretty-printed, is different bytes and so a different signature.
        let value: Value = serde_json::from_slice(&body).unwrap();
        let pretty = serde_json::to_vec_pretty(&value).unwrap();
        let res = post_hook(&shop.app, Some("evt_1"), pretty, &signature).send().await;
        assert_eq!(res.status, StatusCode::UNAUTHORIZED);
        // The original bytes with that signature are accepted, whatever the content type says.
        let res = shop
            .app
            .post(HOOK)
            .no_origin()
            .raw_body(body, Some("text/plain"))
            .header("x-razorpay-signature", &signature)
            .header("x-razorpay-event-id", "evt_1")
            .send()
            .await;
        assert_eq!(res.status, StatusCode::OK, "{:?}", res.body);
    }
}

db_test! {
    async fn a_flood_of_forgeries_is_slowed_but_real_events_never_are(pool: PgPool) {
        let shop = shop(pool).await;
        let body = captured("pay_1", &shop.remote, PRICE);
        let mut statuses = Vec::new();
        for _ in 0..32 {
            let res = post_hook(&shop.app, Some("evt_x"), body.clone(), "00").send().await;
            statuses.push(res.status);
        }
        assert_eq!(statuses.iter().filter(|s| **s == StatusCode::UNAUTHORIZED).count(), 30);
        assert_eq!(statuses.iter().filter(|s| **s == StatusCode::TOO_MANY_REQUESTS).count(), 2);
        // The provider, from the same address, is still served.
        assert_eq!(pay(&shop, "evt_1", "pay_1").await.status, StatusCode::OK);
        assert_eq!(order_status(&shop).await, "paid");
    }
}

db_test! {
    async fn bodies_are_capped_but_larger_than_the_usual_api_limit(pool: PgPool) {
        let shop = shop(pool).await;
        let padded = |bytes: usize| {
            serde_json::to_vec(&json!({"event": "payment.authorized", "pad": "x".repeat(bytes)})).unwrap()
        };
        // 200 KiB is more than the API accepts anywhere else and is fine here.
        let res = deliver(&shop, "evt_big", padded(200 * 1024)).await;
        assert_eq!(res.status, StatusCode::OK, "{:?}", res.body);
        assert_eq!(outcome(&res), "ignored");
        let res = deliver(&shop, "evt_huge", padded(300 * 1024)).await;
        assert_eq!(res.status, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(n(&shop, "select count(*) from webhook_events").await, 1);
    }
}

// ---- exactly once -------------------------------------------------------------------------------

db_test! {
    async fn the_same_event_delivered_again_changes_nothing(pool: PgPool) {
        let shop = shop(pool).await;
        let first = pay(&shop, "evt_1", "pay_1").await;
        let second = pay(&shop, "evt_1", "pay_1").await;
        assert_eq!(outcome(&first), "enrolled");
        assert_eq!(second.status, StatusCode::OK);
        assert_eq!(second.body["duplicate"], true);
        assert_eq!(n(&shop, "select count(*) from enrollments").await, 1);
        assert_eq!(n(&shop, "select count(*) from payments").await, 1);
        assert_eq!(n(&shop, "select count(*) from webhook_events").await, 1);
        assert_eq!(n(&shop, "select count(*) from order_events").await, 1);
    }
}

db_test! {
    async fn replaying_a_captured_body_under_a_made_up_event_id_still_enrolls_once(pool: PgPool) {
        // The event id travels in an unsigned header, so an attacker can vary it. The payment's
        // own id is what the books key on.
        let shop = shop(pool).await;
        let body = captured("pay_1", &shop.remote, PRICE);
        assert_eq!(outcome(&deliver(&shop, "evt_1", body.clone()).await), "enrolled");
        for id in ["evt_2", "evt_3"] {
            let res = deliver(&shop, id, body.clone()).await;
            assert_eq!(res.status, StatusCode::OK);
            assert_eq!(outcome(&res), "already_applied");
        }
        assert_eq!(n(&shop, "select count(*) from enrollments").await, 1);
        assert_eq!(n(&shop, "select count(*) from payments").await, 1);
        assert_eq!(n(&shop, "select count(*) from order_events where kind='paid'").await, 1);
    }
}

db_test! {
    async fn without_an_event_id_the_body_is_the_key(pool: PgPool) {
        let shop = shop(pool).await;
        let body = captured("pay_1", &shop.remote, PRICE);
        let signature = shop.provider.sign_webhook(&body);
        let first = post_hook(&shop.app, None, body.clone(), &signature).send().await;
        let second = post_hook(&shop.app, None, body, &signature).send().await;
        assert_eq!(outcome(&first), "enrolled");
        assert_eq!(second.body["duplicate"], true);
        assert_eq!(n(&shop, "select count(*) from enrollments").await, 1);
    }
}

db_test! {
    async fn order_paid_and_payment_captured_for_one_payment_enroll_once(pool: PgPool) {
        let shop = shop(pool).await;
        let a = deliver(&shop, "evt_a", order_paid("pay_1", &shop.remote, PRICE)).await;
        let b = deliver(&shop, "evt_b", captured("pay_1", &shop.remote, PRICE)).await;
        assert_eq!((outcome(&a), outcome(&b)), ("enrolled", "already_applied"));
        assert_eq!(n(&shop, "select count(*) from enrollments").await, 1);
        assert_eq!(n(&shop, "select count(*) from payments").await, 1);
    }
}

db_test! {
    async fn the_other_order_of_arrival_works_too(pool: PgPool) {
        let shop = shop(pool).await;
        let a = deliver(&shop, "evt_b", captured("pay_1", &shop.remote, PRICE)).await;
        let b = deliver(&shop, "evt_a", order_paid("pay_1", &shop.remote, PRICE)).await;
        assert_eq!((outcome(&a), outcome(&b)), ("enrolled", "already_applied"));
        assert_eq!(n(&shop, "select count(*) from enrollments").await, 1);
    }
}

db_test! {
    async fn ten_simultaneous_deliveries_of_one_event_enroll_once(pool: PgPool) {
        let shop = shop(pool).await;
        let body = captured("pay_1", &shop.remote, PRICE);
        let results = join_all((0..10).map(|_| deliver(&shop, "evt_1", body.clone()))).await;
        assert!(results.iter().all(|r| r.status == StatusCode::OK), "{:?}", results.iter().map(|r| r.status).collect::<Vec<_>>());
        let handled = results.iter().filter(|r| outcome(r) == "enrolled").count();
        let duplicates = results.iter().filter(|r| r.body["duplicate"] == true).count();
        assert_eq!((handled, duplicates), (1, 9));
        assert_eq!(n(&shop, "select count(*) from enrollments").await, 1);
        assert_eq!(n(&shop, "select count(*) from payments").await, 1);
        assert_eq!(n(&shop, "select count(*) from webhook_events").await, 1);
    }
}

db_test! {
    async fn different_events_for_one_payment_racing_enroll_once(pool: PgPool) {
        let shop = shop(pool).await;
        let results = join_all((0..8).map(|i| {
            let body = if i % 2 == 0 {
                captured("pay_1", &shop.remote, PRICE)
            } else {
                order_paid("pay_1", &shop.remote, PRICE)
            };
            let shop = &shop;
            async move { deliver(shop, &format!("evt_{i}"), body).await }
        }))
        .await;
        assert!(results.iter().all(|r| r.status == StatusCode::OK));
        assert_eq!(results.iter().filter(|r| outcome(r) == "enrolled").count(), 1);
        assert_eq!(n(&shop, "select count(*) from enrollments").await, 1);
        assert_eq!(n(&shop, "select count(*) from payments").await, 1);
        assert_eq!(n(&shop, "select count(*) from order_events where kind='paid'").await, 1);
    }
}

db_test! {
    async fn a_crash_midway_leaves_nothing_and_the_redelivery_finishes_the_job(pool: PgPool) {
        let shop = shop(pool).await;
        // Make the last step of handling fail, after the payment, the order and the enrollment
        // have all been written inside the transaction.
        sqlx::query(
            "create function boom() returns trigger language plpgsql as \
             $$ begin raise exception 'simulated crash'; end $$",
        )
        .execute(&shop.pool)
        .await
        .unwrap();
        sqlx::query(
            "create trigger boom before insert on enrollment_events \
             for each row execute function boom()",
        )
        .execute(&shop.pool)
        .await
        .unwrap();

        let crashed = pay(&shop, "evt_1", "pay_1").await;
        assert_eq!(crashed.status, StatusCode::INTERNAL_SERVER_ERROR);
        // Nothing survived: not the payment, not the paid order, not the enrollment, and not
        // even the note that the event arrived.
        assert_eq!(order_status(&shop).await, "pending");
        assert_eq!(n(&shop, "select count(*) from payments").await, 0);
        assert_eq!(n(&shop, "select count(*) from enrollments").await, 0);
        assert_eq!(n(&shop, "select count(*) from order_events").await, 0);
        assert_eq!(n(&shop, "select count(*) from webhook_events").await, 0);

        sqlx::query("drop trigger boom on enrollment_events")
            .execute(&shop.pool)
            .await
            .unwrap();
        let retried = pay(&shop, "evt_1", "pay_1").await;
        assert_eq!(retried.status, StatusCode::OK, "{:?}", retried.body);
        assert_eq!(outcome(&retried), "enrolled");
        assert_eq!(n(&shop, "select count(*) from enrollments").await, 1);
        assert_eq!(n(&shop, "select count(*) from payments").await, 1);
        assert_eq!(n(&shop, "select count(*) from webhook_events").await, 1);
        assert_eq!(access(&shop).await, "allowed");
    }
}

db_test! {
    async fn a_receipt_outbox_failure_rolls_back_the_payment_and_redelivery_queues_it_once(pool: PgPool) {
        let shop = shop(pool).await;
        sqlx::query(
            "create function fail_receipt() returns trigger language plpgsql as \
             $$ begin raise exception 'simulated outbox failure'; end $$",
        )
        .execute(&shop.pool)
        .await
        .unwrap();
        sqlx::query(
            "create trigger fail_receipt before insert on email_outbox \
             for each row execute function fail_receipt()",
        )
        .execute(&shop.pool)
        .await
        .unwrap();

        let failed = pay(&shop, "evt_receipt", "pay_receipt").await;
        assert_eq!(failed.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(order_status(&shop).await, "pending");
        for table in ["payments", "enrollments", "order_events", "webhook_events"] {
            assert_eq!(n(&shop, &format!("select count(*) from {table}")).await, 0, "{table}");
        }
        assert_eq!(n(&shop, "select count(*) from email_outbox where subject like 'Payment received%'").await, 0);

        sqlx::query("drop trigger fail_receipt on email_outbox")
            .execute(&shop.pool)
            .await
            .unwrap();
        let retried = pay(&shop, "evt_receipt", "pay_receipt").await;
        assert_eq!(retried.status, StatusCode::OK, "{:?}", retried.body);
        assert_eq!(outcome(&retried), "enrolled");
        assert_eq!(n(&shop, "select count(*) from email_outbox where subject like 'Payment received%'").await, 1);
        assert_eq!(n(&shop, "select count(*) from webhook_events").await, 1);
        assert_eq!(n(&shop, "select count(*) from enrollments").await, 1);
        let duplicate = pay(&shop, "evt_receipt", "pay_receipt").await;
        assert_eq!(duplicate.status, StatusCode::OK);
        assert_eq!(n(&shop, "select count(*) from email_outbox where subject like 'Payment received%'").await, 1);
    }
}

// ---- amounts and identities -------------------------------------------------------------------

db_test! {
    async fn a_payment_for_a_different_amount_enrolls_nobody_and_is_flagged(pool: PgPool) {
        let shop = shop(pool).await;
        let res = deliver(&shop, "evt_1", captured("pay_1", &shop.remote, 100)).await;
        // 200, so the provider stops resending; flagged, so a person looks.
        assert_eq!(res.status, StatusCode::OK);
        assert_eq!(outcome(&res), "amount_mismatch");
        assert_eq!(order_status(&shop).await, "pending");
        assert_eq!(n(&shop, "select count(*) from enrollments").await, 0);
        assert_eq!(n(&shop, "select count(*) from payments").await, 0);
        let (review, order): (bool, Option<Uuid>) =
            sqlx::query_as("select needs_review, order_id from webhook_events")
                .fetch_one(&shop.pool)
                .await
                .unwrap();
        assert!(review);
        assert_eq!(order, Some(shop.order));
        let details: Value = sqlx::query_scalar("select details from order_events where kind='payment_mismatch'")
            .fetch_one(&shop.pool)
            .await
            .unwrap();
        assert_eq!(details["expected_paise"], PRICE);
        assert_eq!(details["got"]["amount_paise"], 100);
        // The buyer can still pay properly afterwards.
        assert_eq!(outcome(&deliver(&shop, "evt_2", captured("pay_2", &shop.remote, PRICE)).await), "enrolled");
    }
}

db_test! {
    async fn a_higher_amount_or_a_foreign_currency_is_not_accepted_either(pool: PgPool) {
        let shop = shop(pool).await;
        let more = deliver(&shop, "evt_1", captured("pay_1", &shop.remote, PRICE + 1)).await;
        assert_eq!(outcome(&more), "amount_mismatch");
        let mut entity = payment("pay_2", &shop.remote, PRICE, "captured");
        entity["currency"] = json!("USD");
        let usd = deliver(&shop, "evt_2", event("payment.captured", json!({"payment": {"entity": entity}}))).await;
        assert_eq!(outcome(&usd), "currency_mismatch");
        assert_eq!(n(&shop, "select count(*) from enrollments").await, 0);
        assert_eq!(n(&shop, "select count(*) from webhook_events where needs_review").await, 2);
    }
}

db_test! {
    async fn a_payment_for_an_order_we_never_made_is_recorded_not_acted_on(pool: PgPool) {
        let shop = shop(pool).await;
        let stranger = deliver(&shop, "evt_1", captured("pay_1", "order_never_heard_of", PRICE)).await;
        assert_eq!(outcome(&stranger), "unknown_order");
        let mut entity = payment("pay_2", "x", PRICE, "captured");
        entity.as_object_mut().unwrap().remove("order_id");
        let no_order = deliver(&shop, "evt_2", event("payment.captured", json!({"payment": {"entity": entity}}))).await;
        assert_eq!(outcome(&no_order), "unknown_order");
        assert_eq!(n(&shop, "select count(*) from webhook_events where needs_review").await, 2);
        assert_eq!(n(&shop, "select count(*) from enrollments").await, 0);
        assert_eq!(order_status(&shop).await, "pending");
    }
}

db_test! {
    async fn a_payment_that_says_it_is_not_captured_grants_nothing(pool: PgPool) {
        let shop = shop(pool).await;
        let body = event(
            "payment.captured",
            json!({"payment": {"entity": payment("pay_1", &shop.remote, PRICE, "authorized")}}),
        );
        assert_eq!(outcome(&deliver(&shop, "evt_1", body).await), "ignored");
        assert_eq!(n(&shop, "select count(*) from enrollments").await, 0);
    }
}

db_test! {
    async fn orders_pay_out_by_the_order_they_belong_to_not_by_the_buyer_named_in_a_payload(pool: PgPool) {
        // Nothing in the payload can name a different buyer: the enrollment goes to whoever
        // owns the order.
        let shop = shop(pool).await;
        let other = signup_verified_as(&shop.app, "other@hooks.in", "student").await;
        let mut entity = payment("pay_1", &shop.remote, PRICE, "captured");
        entity["email"] = json!("other@hooks.in");
        entity["notes"] = json!({"user_id": user_id(&shop.pool, "other@hooks.in").await.to_string()});
        deliver(&shop, "evt_1", event("payment.captured", json!({"payment": {"entity": entity}}))).await;
        assert_eq!(n(&shop, &format!("select count(*) from enrollments where user_id='{}'", shop.buyer_id)).await, 1);
        let other_id = user_id(&shop.pool, "other@hooks.in").await;
        assert_eq!(n(&shop, &format!("select count(*) from enrollments where user_id='{other_id}'")).await, 0);
        drop(other);
    }
}

// ---- failures, retries and order of arrival ---------------------------------------------------

db_test! {
    async fn a_failed_attempt_is_recorded_and_the_order_stays_payable(pool: PgPool) {
        let shop = shop(pool).await;
        let res = deliver(&shop, "evt_1", failed("pay_1", &shop.remote, PRICE)).await;
        assert_eq!(outcome(&res), "failure_recorded");
        assert_eq!(order_status(&shop).await, "pending");
        assert_eq!(text(&shop, "select status::text from payments where provider_ref='pay_1'").await, "failed");
        assert_eq!(n(&shop, "select count(*) from enrollments").await, 0);
        let read = shop.app.get(&format!("/v1/orders/{}", shop.order)).jar(&shop.buyer).send().await;
        assert!(read.body["checkout"].is_object(), "still payable");

        // The buyer tries again and it works; last good state wins, the failure stays on record.
        assert_eq!(outcome(&deliver(&shop, "evt_2", captured("pay_2", &shop.remote, PRICE)).await), "enrolled");
        assert_eq!(order_status(&shop).await, "paid");
        assert_eq!(text(&shop, "select status::text from payments where provider_ref='pay_1'").await, "failed");
        assert_eq!(text(&shop, "select status::text from payments where provider_ref='pay_2'").await, "confirmed");
    }
}

db_test! {
    async fn a_failure_that_arrives_after_the_payment_never_undoes_it(pool: PgPool) {
        let shop = shop(pool).await;
        pay(&shop, "evt_1", "pay_1").await;
        // The same payment id reported failed late, and another attempt's failure.
        let same = deliver(&shop, "evt_2", failed("pay_1", &shop.remote, PRICE)).await;
        let other = deliver(&shop, "evt_3", failed("pay_0", &shop.remote, PRICE)).await;
        assert_eq!(outcome(&same), "ignored");
        assert_eq!(outcome(&other), "failure_recorded");
        assert_eq!(order_status(&shop).await, "paid");
        assert_eq!(text(&shop, "select status::text from payments where provider_ref='pay_1'").await, "confirmed");
        assert_eq!(access(&shop).await, "allowed");
    }
}

db_test! {
    async fn a_payment_first_reported_failed_that_then_captures_is_upgraded(pool: PgPool) {
        let shop = shop(pool).await;
        deliver(&shop, "evt_1", failed("pay_1", &shop.remote, PRICE)).await;
        let res = deliver(&shop, "evt_2", captured("pay_1", &shop.remote, PRICE)).await;
        assert_eq!(outcome(&res), "enrolled");
        assert_eq!(n(&shop, "select count(*) from payments").await, 1);
        assert_eq!(text(&shop, "select status::text from payments").await, "confirmed");
    }
}

db_test! {
    async fn an_order_that_expired_before_the_money_arrived_still_gets_its_buyer_enrolled(pool: PgPool) {
        let shop = shop(pool).await;
        sqlx::query("update orders set status='expired', expires_at = now() - interval '2 hours'")
            .execute(&shop.pool)
            .await
            .unwrap();
        let res = pay(&shop, "evt_1", "pay_1").await;
        assert_eq!(outcome(&res), "enrolled");
        assert_eq!(order_status(&shop).await, "paid");
        assert_eq!(access(&shop).await, "allowed");
    }
}

db_test! {
    async fn a_second_payment_for_a_paid_order_is_recorded_as_a_duplicate_and_flagged(pool: PgPool) {
        let shop = shop(pool).await;
        pay(&shop, "evt_1", "pay_1").await;
        let res = deliver(&shop, "evt_2", captured("pay_2", &shop.remote, PRICE)).await;
        assert_eq!(outcome(&res), "duplicate_payment");
        assert_eq!(text(&shop, "select status::text from payments where provider_ref='pay_2'").await, "duplicate");
        assert_eq!(text(&shop, "select status::text from payments where provider_ref='pay_1'").await, "confirmed");
        assert_eq!(n(&shop, "select count(*) from enrollments").await, 1);
        assert_eq!(n(&shop, "select count(*) from webhook_events where needs_review").await, 1);
        assert_eq!(n(&shop, "select count(*) from order_events where kind='duplicate_payment'").await, 1);
        // Redelivered, it is still just one record.
        let again = deliver(&shop, "evt_3", captured("pay_2", &shop.remote, PRICE)).await;
        assert_eq!(outcome(&again), "already_applied");
        assert_eq!(n(&shop, "select count(*) from payments").await, 2);
    }
}

db_test! {
    async fn the_database_itself_refuses_two_confirmed_payments_for_one_order(pool: PgPool) {
        let shop = shop(pool).await;
        pay(&shop, "evt_1", "pay_1").await;
        let second = sqlx::query(
            "insert into payments (provider, provider_ref, order_id, amount_paise, status, raw_event) \
             values ('fake', 'pay_sneaky', $1, 49900, 'confirmed', '{}')",
        )
        .bind(shop.order)
        .execute(&shop.pool)
        .await;
        assert!(second.is_err());
    }
}

// ---- access that was taken away or already held ---------------------------------------------

db_test! {
    async fn paying_for_access_a_creator_blocked_records_the_money_and_does_not_undo_the_block(pool: PgPool) {
        let shop = shop(pool).await;
        sqlx::query(
            "insert into enrollments \
                (user_id, course_id, source, granted_by, revoked_at, revoke_reason, revoked_by, revocation_kind) \
             values ($1,$2,'grant',$3, now(), 'misuse', $3, 'creator')",
        )
        .bind(shop.buyer_id)
        .bind(shop.course)
        .bind(shop.creator_id)
        .execute(&shop.pool)
        .await
        .unwrap();
        let mails = shop.app.mailer.sent().len();

        let res = pay(&shop, "evt_1", "pay_1").await;

        assert_eq!(outcome(&res), "paid_access_blocked");
        assert_eq!(access(&shop).await, "access_revoked");
        assert_eq!(order_status(&shop).await, "paid");
        assert_eq!(text(&shop, "select status::text from payments").await, "confirmed");
        assert_eq!(n(&shop, "select count(*) from webhook_events where needs_review").await, 1);
        shop.app.settle().await;
        assert_eq!(shop.app.mailer.sent().len(), mails, "no receipt for access that was not given");
    }
}

db_test! {
    async fn paying_when_access_was_already_granted_keeps_it_and_flags_the_money(pool: PgPool) {
        let shop = shop(pool).await;
        sqlx::query("insert into enrollments (user_id, course_id, source, granted_by) values ($1,$2,'grant',$3)")
            .bind(shop.buyer_id)
            .bind(shop.course)
            .bind(shop.creator_id)
            .execute(&shop.pool)
            .await
            .unwrap();
        let res = pay(&shop, "evt_1", "pay_1").await;
        assert_eq!(outcome(&res), "paid_already_enrolled");
        let (source, linked): (String, bool) = sqlx::query_as(
            "select source::text, payment_id is not null from enrollments where user_id=$1",
        )
        .bind(shop.buyer_id)
        .fetch_one(&shop.pool)
        .await
        .unwrap();
        assert_eq!((source.as_str(), linked), ("grant", true));
        assert_eq!(n(&shop, "select count(*) from enrollments").await, 1);
        assert_eq!(n(&shop, "select count(*) from webhook_events where needs_review").await, 1);
    }
}

// ---- refunds ----------------------------------------------------------------------------------

db_test! {
    async fn a_full_refund_ends_the_access_the_payment_bought(pool: PgPool) {
        let shop = shop(pool).await;
        pay(&shop, "evt_1", "pay_1").await;
        assert_eq!(access(&shop).await, "allowed");

        let res = deliver(&shop, "evt_2", refund_processed("rfnd_1", "pay_1", PRICE, None)).await;

        assert_eq!(outcome(&res), "refund_applied");
        assert_eq!(order_status(&shop).await, "refunded");
        assert_eq!(text(&shop, "select status::text from payments").await, "refunded");
        assert_eq!(access(&shop).await, "access_revoked");
        let (kind, reason): (String, String) = sqlx::query_as(
            "select revocation_kind, revoke_reason from enrollments where user_id=$1",
        )
        .bind(shop.buyer_id)
        .fetch_one(&shop.pool)
        .await
        .unwrap();
        assert_eq!((kind.as_str(), reason.as_str()), ("payment", "Refunded."));
        assert_eq!(n(&shop, "select count(*) from enrollment_events where action='revoked'").await, 1);

        // Redelivered under another id, it is already done.
        let again = deliver(&shop, "evt_3", refund_processed("rfnd_1", "pay_1", PRICE, None)).await;
        assert_eq!(outcome(&again), "already_refunded");
        assert_eq!(n(&shop, "select count(*) from enrollment_events where action='revoked'").await, 1);
    }
}

db_test! {
    async fn a_refund_ends_playback_that_is_in_progress(pool: PgPool) {
        let shop = shop(pool).await;
        pay(&shop, "evt_1", "pay_1").await;
        let session = Uuid::new_v4();
        sqlx::query(
            "insert into playback_sessions (id, user_id, video_id, expires_at) \
             values ($1, $2, $3, now() + interval '2 hours')",
        )
        .bind(session)
        .bind(shop.buyer_id)
        .bind(shop.video)
        .execute(&shop.pool)
        .await
        .unwrap();
        deliver(&shop, "evt_2", refund_processed("rfnd_1", "pay_1", PRICE, None)).await;
        let reason: String = sqlx::query_scalar("select end_reason from playback_sessions where id=$1")
            .bind(session)
            .fetch_one(&shop.pool)
            .await
            .unwrap();
        assert_eq!(reason, "access_revoked");
    }
}

db_test! {
    async fn a_refunded_buyer_can_buy_again_and_gets_the_same_enrollment_back(pool: PgPool) {
        let shop = shop(pool).await;
        pay(&shop, "evt_1", "pay_1").await;
        let enrollment: Uuid = sqlx::query_scalar("select id from enrollments")
            .fetch_one(&shop.pool)
            .await
            .unwrap();
        deliver(&shop, "evt_2", refund_processed("rfnd_1", "pay_1", PRICE, None)).await;

        // A refund ended it, so checkout is open again.
        let again = shop.app.post(&format!("/v1/checkout/{}", shop.course)).jar(&shop.buyer).send().await;
        assert_eq!(again.status, StatusCode::CREATED, "{:?}", again.body);
        let remote = again.body["checkout"]["provider_order_id"].as_str().unwrap().to_owned();
        let res = deliver(&shop, "evt_3", captured("pay_9", &remote, PRICE)).await;

        assert_eq!(outcome(&res), "enrolled");
        assert_eq!(access(&shop).await, "allowed");
        let rows: Vec<Uuid> = sqlx::query_scalar("select id from enrollments").fetch_all(&shop.pool).await.unwrap();
        assert_eq!(rows, vec![enrollment], "reactivated, not duplicated");
        assert_eq!(n(&shop, "select count(*) from enrollment_events where action='reactivated'").await, 1);
        let pointer = text(&shop, "select p.provider_ref from enrollments e join payments p on p.id = e.payment_id").await;
        assert_eq!(pointer, "pay_9");
    }
}

db_test! {
    async fn a_refund_that_overtakes_its_payment_means_the_payment_grants_nothing(pool: PgPool) {
        let shop = shop(pool).await;
        let entity = payment("pay_1", &shop.remote, PRICE, "refunded");
        let res = deliver(&shop, "evt_1", refund_processed("rfnd_1", "pay_1", PRICE, Some(entity))).await;
        assert_eq!(outcome(&res), "refund_before_payment");
        assert_eq!(order_status(&shop).await, "refunded");
        assert_eq!(text(&shop, "select status::text from payments").await, "refunded");

        // Its own capture event turns up late.
        let late = pay(&shop, "evt_2", "pay_1").await;
        assert_eq!(outcome(&late), "already_refunded");
        assert_eq!(n(&shop, "select count(*) from enrollments").await, 0);
        assert_eq!(order_status(&shop).await, "refunded");
        assert_eq!(access(&shop).await, "access_revoked");
    }
}

db_test! {
    async fn a_refund_for_a_payment_we_cannot_place_is_flagged(pool: PgPool) {
        let shop = shop(pool).await;
        let res = deliver(&shop, "evt_1", refund_processed("rfnd_1", "pay_nowhere", PRICE, None)).await;
        assert_eq!(outcome(&res), "refund_unknown_payment");
        assert_eq!(n(&shop, "select count(*) from webhook_events where needs_review").await, 1);
    }
}

db_test! {
    async fn a_partial_refund_changes_nothing_and_asks_for_a_person(pool: PgPool) {
        let shop = shop(pool).await;
        pay(&shop, "evt_1", "pay_1").await;
        let res = deliver(&shop, "evt_2", refund_processed("rfnd_1", "pay_1", PRICE / 2, None)).await;
        assert_eq!(outcome(&res), "refund_partial");
        assert_eq!(access(&shop).await, "allowed");
        assert_eq!(order_status(&shop).await, "paid");
        assert_eq!(text(&shop, "select status::text from payments").await, "confirmed");
        assert_eq!(n(&shop, "select count(*) from webhook_events where needs_review").await, 1);
    }
}

db_test! {
    async fn a_refund_the_provider_could_not_complete_changes_nothing_and_is_flagged(pool: PgPool) {
        let shop = shop(pool).await;
        pay(&shop, "evt_1", "pay_1").await;
        let res = deliver(&shop, "evt_2", refund_failed("rfnd_1", "pay_1", PRICE)).await;
        assert_eq!(outcome(&res), "refund_failed");
        assert_eq!(access(&shop).await, "allowed");
        assert_eq!(order_status(&shop).await, "paid");
        assert_eq!(n(&shop, "select count(*) from order_events where kind='refund_failed'").await, 1);
        assert_eq!(n(&shop, "select count(*) from webhook_events where needs_review").await, 1);
    }
}

db_test! {
    async fn refunding_a_duplicate_payment_leaves_the_real_purchase_alone(pool: PgPool) {
        let shop = shop(pool).await;
        pay(&shop, "evt_1", "pay_1").await;
        deliver(&shop, "evt_2", captured("pay_2", &shop.remote, PRICE)).await;
        let res = deliver(&shop, "evt_3", refund_processed("rfnd_1", "pay_2", PRICE, None)).await;
        assert_eq!(outcome(&res), "refund_applied");
        assert_eq!(text(&shop, "select status::text from payments where provider_ref='pay_2'").await, "refunded");
        assert_eq!(order_status(&shop).await, "paid", "the purchase itself stands");
        assert_eq!(access(&shop).await, "allowed");
    }
}

// ---- events we do not act on -------------------------------------------------------------------

db_test! {
    async fn events_we_do_not_act_on_are_recorded_and_acknowledged(pool: PgPool) {
        let shop = shop(pool).await;
        let res = deliver(&shop, "evt_1", event("payment.authorized", json!({"payment": {"entity": payment("pay_1", &shop.remote, PRICE, "authorized")}}))).await;
        assert_eq!((res.status, outcome(&res)), (StatusCode::OK, "ignored"));
        let res = deliver(&shop, "evt_2", event("settlement.processed", json!({}))).await;
        assert_eq!((res.status, outcome(&res)), (StatusCode::OK, "ignored"));
        assert_eq!(n(&shop, "select count(*) from webhook_events").await, 2);
        assert_eq!(order_status(&shop).await, "pending");
    }
}

db_test! {
    async fn signed_events_that_cannot_be_read_are_acknowledged_so_they_stop_coming(pool: PgPool) {
        let shop = shop(pool).await;
        let bodies: Vec<Vec<u8>> = vec![
            b"not json at all".to_vec(),
            b"[1,2,3]".to_vec(),
            event("payment.captured", json!({})),
            event(
                "payment.captured",
                json!({"payment": {"entity": {"id": "pay_1", "order_id": shop.remote, "amount": "49900", "currency": "INR", "status": "captured"}}}),
            ),
        ];
        for (i, body) in bodies.into_iter().enumerate() {
            let res = deliver(&shop, &format!("evt_{i}"), body).await;
            assert_eq!(res.status, StatusCode::OK, "{i}: {:?}", res.body);
            assert_eq!(outcome(&res), "malformed", "{i}");
        }
        assert_eq!(n(&shop, "select count(*) from enrollments").await, 0);
        assert_eq!(order_status(&shop).await, "pending");
    }
}

db_test! {
    async fn the_stored_event_keeps_what_the_provider_sent(pool: PgPool) {
        let shop = shop(pool).await;
        pay(&shop, "evt_1", "pay_1").await;
        let payload: Value = sqlx::query_scalar("select payload from webhook_events where provider_event_id='evt_1'")
            .fetch_one(&shop.pool)
            .await
            .unwrap();
        assert_eq!(payload["event"], "payment.captured");
        assert_eq!(payload["payload"]["payment"]["entity"]["id"], "pay_1");
        assert_eq!(n(&shop, "select count(*) from webhook_events where order_id is not null").await, 1);
    }
}
