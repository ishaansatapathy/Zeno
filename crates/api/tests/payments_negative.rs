//! The negative suite for the payment path: everything an attacker, a broken client or a confused
//! provider could send, and the one thing that must be true of each: no money, no access, no
//! crash. The happy paths and the individual rules have their own suites (`webhooks`, `checkout`,
//! `coupons`, `refunds`); this one is the script that tries to get around them from the outside.

mod common;

use axum::http::{Method, StatusCode};
use common::shop::*;
use common::*;
use serde_json::{json, Value};
use sqlx::PgPool;

macro_rules! db_test {
    ($(#[$meta:meta])* async fn $name:ident($pool:ident: PgPool) $body:block) => {
        #[sqlx::test(migrator = "zeno_core::db::MIGRATOR")]
        $(#[$meta])*
        async fn $name($pool: PgPool) $body
    };
}

/// Nothing was paid for: the order is still open, nobody is enrolled, nobody was credited.
async fn nothing_was_bought(shop: &Shop) {
    assert_eq!(order_status(shop).await, "pending");
    for table in ["enrollments", "ledger_journals", "invoices"] {
        assert_eq!(
            n(shop, &format!("select count(*) from {table}")).await,
            0,
            "{table}"
        );
    }
    assert_eq!(
        n(
            shop,
            "select count(*) from payments where status = 'confirmed'"
        )
        .await,
        0
    );
    assert_eq!(access(shop).await, "access_revoked");
}

fn code_of(res: &Res) -> &str {
    res.body["error"]["code"].as_str().unwrap_or("")
}

// ---- the webhook as a door ---------------------------------------------------------------------

db_test! {
    async fn the_signature_is_the_credential_whatever_the_content_type_says(pool: PgPool) {
        let shop = shop(pool).await;
        let body = captured("pay_1", &shop.remote, PRICE);
        let signature = shop.provider.sign_webhook(&body);

        // The provider sends application/json, but nothing is decided by that header: the bytes
        // are signed, and what is signed is what is read.
        let plain = shop.app.post(HOOK).no_origin()
            .raw_body(body.clone(), Some("text/plain"))
            .header("x-razorpay-signature", &signature)
            .header("x-razorpay-event-id", "evt_plain")
            .send().await;
        assert_eq!((plain.status, outcome(&plain)), (StatusCode::OK, "enrolled"), "{:?}", plain.body);

        let none = shop.app.post(HOOK).no_origin()
            .raw_body(body, None)
            .header("x-razorpay-signature", &signature)
            .header("x-razorpay-event-id", "evt_none")
            .send().await;
        assert_eq!(none.status, StatusCode::OK);
        assert_eq!(n(&shop, "select count(*) from enrollments").await, 1);
        assert_eq!(n(&shop, "select count(*) from payments").await, 1);
    }
}

db_test! {
    async fn an_unsigned_or_wrongly_signed_body_of_any_shape_changes_nothing(pool: PgPool) {
        let shop = shop(pool).await;
        let good = captured("pay_1", &shop.remote, PRICE);
        let signature = shop.provider.sign_webhook(&good);
        let other_body_signature = shop.provider.sign_webhook(b"{}");
        let deep = format!("{}{}", "[".repeat(5_000), "]".repeat(5_000)).into_bytes();

        let attempts: Vec<(&str, Vec<u8>, Option<String>)> = vec![
            ("no signature", good.clone(), None),
            ("an empty signature", good.clone(), Some(String::new())),
            ("a blank signature", good.clone(), Some("   ".into())),
            ("sixty-four zeros", good.clone(), Some("0".repeat(64))),
            ("the signature of another body", good.clone(), Some(other_body_signature)),
            ("a signature with a prefix", good.clone(), Some(format!("sha256={signature}"))),
            ("a signature with a suffix", good.clone(), Some(format!("{signature}00"))),
            ("an empty body", Vec::new(), Some(signature.clone())),
            ("a body that is not JSON", b"not json".to_vec(), Some(signature.clone())),
            ("a pathologically nested body", deep, Some(signature.clone())),
        ];
        for (what, body, sig) in attempts {
            let mut request = shop.app.post(HOOK).no_origin()
                .raw_body(body, Some("application/json"))
                .header("x-razorpay-event-id", "evt_forged");
            if let Some(sig) = &sig {
                request = request.header("x-razorpay-signature", sig);
            }
            let res = request.send().await;
            assert_eq!(res.status, StatusCode::UNAUTHORIZED, "{what}: {:?}", res.body);
            assert_eq!(code_of(&res), "invalid_signature", "{what}");
        }
        nothing_was_bought(&shop).await;
        assert_eq!(n(&shop, "select count(*) from webhook_events").await, 0, "not even recorded");
    }
}

db_test! {
    async fn the_webhook_path_answers_only_to_post(pool: PgPool) {
        let shop = shop(pool).await;
        for method in [Method::GET, Method::PUT, Method::PATCH, Method::DELETE] {
            let res = shop.app.req(method.clone(), HOOK).no_origin().send().await;
            assert_eq!(res.status, StatusCode::METHOD_NOT_ALLOWED, "{method}");
        }
        nothing_was_bought(&shop).await;
    }
}

db_test! {
    async fn a_signed_body_that_makes_no_sense_is_acknowledged_and_never_a_server_error(pool: PgPool) {
        let shop = shop(pool).await;
        let entity = |tweak: &dyn Fn(&mut Value)| {
            let mut entity = payment("pay_1", &shop.remote, PRICE, "captured");
            tweak(&mut entity);
            event("payment.captured", json!({"payment": {"entity": entity}}))
        };
        let long = "x".repeat(201);
        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("an empty body", Vec::new()),
            ("an empty object", b"{}".to_vec()),
            ("a bare number", b"42".to_vec()),
            ("nesting past the parser's limit", format!("{}{}", "[".repeat(5_000), "]".repeat(5_000)).into_bytes()),
            ("an event name that is not text", br#"{"event": 7}"#.to_vec()),
            ("an event name that is far too long", event(&"e".repeat(101), json!({}))),
            ("no payment at all", event("payment.captured", json!({}))),
            ("a payment that is not an object", event("payment.captured", json!({"payment": {"entity": "pay_1"}}))),
            ("a payment id that is not text", entity(&|e| e["id"] = json!(7))),
            ("an empty payment id", entity(&|e| e["id"] = json!(""))),
            ("a payment id past the limit", entity(&|e| e["id"] = json!(long))),
            ("an amount in rupees as text", entity(&|e| e["amount"] = json!("499.00"))),
            ("an amount in paise as text", entity(&|e| e["amount"] = json!("49900"))),
            ("an amount with a fraction", entity(&|e| e["amount"] = json!(49_900.5))),
            ("an amount written as a float", entity(&|e| e["amount"] = json!(49_900.0))),
            ("a negative amount", entity(&|e| e["amount"] = json!(-49_900))),
            ("an amount past what fits", entity(&|e| e["amount"] = json!(1e30))),
            ("no amount", entity(&|e| { e.as_object_mut().unwrap().remove("amount"); })),
            ("no currency", entity(&|e| { e.as_object_mut().unwrap().remove("currency"); })),
            ("a null status", entity(&|e| e["status"] = Value::Null)),
            ("an order id that is not text", entity(&|e| e["order_id"] = json!({"a": 1}))),
        ];
        for (i, (what, body)) in cases.into_iter().enumerate() {
            let res = deliver(&shop, &format!("evt_odd_{i}"), body).await;
            assert_eq!(res.status, StatusCode::OK, "{what}: {:?}", res.body);
        }
        nothing_was_bought(&shop).await;
    }
}

db_test! {
    async fn an_event_id_is_a_label_and_never_part_of_a_query(pool: PgPool) {
        let shop = shop(pool).await;
        let body = captured("pay_1", &shop.remote, PRICE);
        // Printable, no spaces: kept as given, so it must survive being stored as data.
        let hostile = "x');droptable-orders;--\"\\%_";

        let first = deliver(&shop, hostile, body.clone()).await;
        assert_eq!((first.status, outcome(&first)), (StatusCode::OK, "enrolled"), "{:?}", first.body);
        let again = deliver(&shop, hostile, body.clone()).await;
        assert_eq!(again.body["duplicate"], true);
        assert_eq!(
            text(&shop, "select provider_event_id from webhook_events").await,
            hostile
        );

        // An id that is too long, or has spaces or tabs in it, is not trusted as a key: the
        // body's hash is. (Whatever the id, the same payment is never applied twice.)
        let signature = shop.provider.sign_webhook(&body);
        for (i, id) in ["a".repeat(500), "tab\there".to_owned(), "two words".to_owned()].iter().enumerate() {
            let res = post_hook(&shop.app, Some(id), body.clone(), &signature).send().await;
            assert_eq!(res.status, StatusCode::OK, "case {i}: {:?}", res.body);
            assert_eq!(res.body["received"], true, "case {i}");
        }
        assert_eq!(n(&shop, "select count(*) from enrollments").await, 1);
        assert_eq!(n(&shop, "select count(*) from payments").await, 1);
    }
}

// ---- checkout as a door -------------------------------------------------------------------------

db_test! {
    async fn a_price_the_client_names_is_never_used(pool: PgPool) {
        let shop = shop(pool).await;
        shop.app.settle().await;
        let buyer = signup_verified_as(&shop.app, "second@hooks.in", "student").await;

        for body in [
            json!({"price_paise": 1}),
            json!({"amount": 1, "currency": "USD"}),
            json!({"amount_paise": 0, "creator_id": shop.buyer_id, "status": "paid"}),
            json!({"coupon": null, "price": {"final_paise": 1}}),
        ] {
            let res = shop.app.post(&format!("/v1/checkout/{}", shop.course))
                .jar(&buyer).json(body.clone()).send().await;
            assert!(res.status == StatusCode::CREATED || res.status == StatusCode::OK, "{body}: {:?}", res.body);
            assert_eq!(res.body["order"]["amount_paise"], PRICE, "{body}");
            assert_eq!(res.body["order"]["currency"], "INR", "{body}");
        }
        assert_eq!(
            n(&shop, "select count(*) from orders where amount_paise <> 49900").await,
            0
        );
    }
}

db_test! {
    async fn a_cross_site_form_cannot_start_a_checkout_in_the_buyers_name(pool: PgPool) {
        let shop = shop(pool).await;
        let before = n(&shop, "select count(*) from orders").await;
        // What an attacker's page can send without a preflight: a text/plain body, the buyer's
        // cookies, and the attacker's origin.
        let res = shop.app.post(&format!("/v1/checkout/{}", shop.course))
            .jar(&shop.buyer)
            .origin("https://evil.example")
            .raw_body(br#"{"coupon":"FREE"}"#.to_vec(), Some("text/plain"))
            .send().await;
        assert_eq!(res.status, StatusCode::FORBIDDEN, "{:?}", res.body);
        assert_eq!(n(&shop, "select count(*) from orders").await, before);
    }
}

db_test! {
    async fn an_idempotency_key_belongs_to_the_buyer_who_used_it(pool: PgPool) {
        let shop = shop(pool).await;
        shop.app.settle().await;
        let first = signup_verified_as(&shop.app, "first@hooks.in", "student").await;
        shop.app.settle().await;
        let second = signup_verified_as(&shop.app, "second@hooks.in", "student").await;
        let path = format!("/v1/checkout/{}", shop.course);

        let a = shop.app.post(&path).jar(&first).header("idempotency-key", "shared-key").send().await;
        let b = shop.app.post(&path).jar(&second).header("idempotency-key", "shared-key").send().await;

        assert_eq!(a.status, StatusCode::CREATED, "{:?}", a.body);
        assert_eq!(b.status, StatusCode::CREATED, "{:?}", b.body);
        assert_ne!(a.body["order"]["id"], b.body["order"]["id"], "a stranger's key returned their order");
        let owner = |id: &Value| {
            format!("select user_id::text from orders where id = '{}'", id.as_str().unwrap())
        };
        assert_eq!(text(&shop, &owner(&a.body["order"]["id"])).await, user_id(&shop.pool, "first@hooks.in").await.to_string());
        assert_eq!(text(&shop, &owner(&b.body["order"]["id"])).await, user_id(&shop.pool, "second@hooks.in").await.to_string());
    }
}

db_test! {
    async fn ids_that_are_not_ids_are_simply_not_found(pool: PgPool) {
        let shop = shop(pool).await;
        let nowhere = [
            "not-a-uuid",
            "00000000-0000-0000-0000-000000000000",
            "%27%3B%20drop%20table%20orders%3B%20--",
            "%00",
            "%2e%2e",
            "1e3",
        ];
        for id in nowhere {
            for (method, path) in [
                (Method::POST, format!("/v1/checkout/{id}")),
                (Method::POST, format!("/v1/checkout/{id}/preview")),
                (Method::GET, format!("/v1/orders/{id}")),
            ] {
                let res = shop.app.req(method.clone(), &path).jar(&shop.buyer).send().await;
                assert!(
                    res.status == StatusCode::NOT_FOUND || res.status == StatusCode::BAD_REQUEST,
                    "{method} {path}: {} {:?}", res.status, res.body
                );
            }
        }
        assert_eq!(n(&shop, "select count(*) from orders").await, 1, "no order appeared");
    }
}

db_test! {
    async fn hostile_coupon_codes_match_nothing_and_break_nothing(pool: PgPool) {
        let shop = shop(pool).await;
        let made = shop.app.post("/v1/coupons").jar(&shop.creator)
            .json(json!({"code": "SAVE10", "kind": "percent", "value": 10})).send().await;
        assert_eq!(made.status, StatusCode::CREATED, "{:?}", made.body);

        let codes = [
            "%".to_owned(),
            "_____".to_owned(),
            "SAVE%".to_owned(),
            "*".to_owned(),
            "' OR '1'='1".to_owned(),
            "SAVE10'; update orders set amount_paise=0; --".to_owned(),
            "SAVE10\u{0}".to_owned(),
            "SAVE１０".to_owned(), // full-width digits
            "ＳＡＶＥ10".to_owned(),
            "S\u{200b}AVE10".to_owned(), // a zero-width space
            "SAVE10 SAVE10".to_owned(),
            "A".repeat(10_000),
        ];
        for code in &codes {
            let res = shop.app.post(&format!("/v1/checkout/{}/preview", shop.course))
                .jar(&shop.buyer).json(json!({"coupon": code})).send().await;
            assert!(
                res.status.is_client_error(),
                "{code:?} was accepted: {} {:?}", res.status, res.body
            );
        }
        assert_eq!(n(&shop, "select count(*) from coupon_redemptions").await, 0);
        assert_eq!(n(&shop, "select count(*) from orders where amount_paise <> 49900").await, 0);
        // And the real code still works afterwards: nothing above damaged it.
        let ok = shop.app.post(&format!("/v1/checkout/{}/preview", shop.course))
            .jar(&shop.buyer).json(json!({"coupon": "save10"})).send().await;
        assert_eq!(ok.status, StatusCode::OK, "{:?}", ok.body);
    }
}

// ---- money events that do not add up -------------------------------------------------------------

async fn creator_balance(shop: &Shop) -> i64 {
    n(shop, "select coalesce(sum(amount_paise), 0)::bigint from ledger_entries where creator_id is not null").await
}

async fn succeeded_refunds(shop: &Shop) -> i64 {
    n(
        shop,
        "select coalesce(sum(amount_paise), 0)::bigint from refunds where status = 'succeeded'",
    )
    .await
}

async fn integrity_is_clean(shop: &Shop) {
    let report = shop
        .app
        .get("/v1/admin/ledger/integrity")
        .jar(&admin_jar(shop).await)
        .send()
        .await;
    assert_eq!(report.status, StatusCode::OK, "{:?}", report.body);
    assert_eq!(report.body["ok"], true, "{:?}", report.body);
}

async fn payment_id(shop: &Shop, provider_ref: &str) -> uuid::Uuid {
    sqlx::query_scalar("select id from payments where provider_ref = $1")
        .bind(provider_ref)
        .fetch_one(&shop.pool)
        .await
        .unwrap()
}

async fn insert_refund(
    shop: &Shop,
    payment: uuid::Uuid,
    amount: i64,
    status: &str,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error> {
    sqlx::query(
        "insert into refunds (order_id, payment_id, amount_paise, status, origin, completed_at) \
         values ($1, $2, $3, $4::refund_status, 'provider', case when $4 = 'succeeded' then now() end)",
    )
    .bind(shop.order)
    .bind(payment)
    .bind(amount)
    .bind(status)
    .execute(&shop.pool)
    .await
}

db_test! {
    async fn a_refund_for_a_payment_we_never_saw_moves_nothing(pool: PgPool) {
        let shop = shop(pool).await;
        let stranger = deliver(&shop, "evt_r1", refund_processed("rfnd_1", "pay_unknown", PRICE, None)).await;
        assert_eq!((stranger.status, outcome(&stranger)), (StatusCode::OK, "refund_unknown_payment"));
        assert_eq!(n(&shop, "select count(*) from refunds").await, 0);
        assert_eq!(n(&shop, "select count(*) from ledger_journals").await, 0);
        assert_eq!(n(&shop, "select count(*) from webhook_events where needs_review").await, 1);
    }
}

db_test! {
    async fn a_refund_larger_than_the_payment_is_flagged_and_moves_nothing(pool: PgPool) {
        let shop = shop(pool).await;
        let paid = pay(&shop, "evt_p", "pay_1").await;
        assert_eq!(outcome(&paid), "enrolled");
        let before = creator_balance(&shop).await;

        for (i, amount) in [PRICE + 1, PRICE * 100, i64::MAX].into_iter().enumerate() {
            let res = deliver(&shop, &format!("evt_big_{i}"), refund_processed(&format!("rfnd_big_{i}"), "pay_1", amount, None)).await;
            assert_eq!((res.status, outcome(&res)), (StatusCode::OK, "refund_exceeds_payment"), "{amount}: {:?}", res.body);
        }

        assert_eq!(creator_balance(&shop).await, before, "the creator was not debited");
        assert_eq!(n(&shop, "select count(*) from refunds").await, 0, "no refund was recorded");
        assert_eq!(n(&shop, "select count(*) from ledger_journals where kind = 'refund'").await, 0);
        assert_eq!(order_status(&shop).await, "paid");
        assert_eq!(access(&shop).await, "allowed", "the buyer keeps what they paid for");
        assert_eq!(n(&shop, "select count(*) from webhook_events where outcome = 'refund_exceeds_payment' and needs_review").await, 3);
        assert_eq!(n(&shop, "select count(*) from order_events where kind = 'refund_over_limit'").await, 3);
        integrity_is_clean(&shop).await;
    }
}

db_test! {
    async fn refunds_may_add_up_to_the_payment_and_no_further(pool: PgPool) {
        let shop = shop(pool).await;
        pay(&shop, "evt_p", "pay_1").await;

        let first = deliver(&shop, "evt_1", refund_processed("rfnd_1", "pay_1", 30_000, None)).await;
        assert_eq!(outcome(&first), "refund_partial");
        // 30,000 more would return 60,000 of a 49,900 payment.
        let over = deliver(&shop, "evt_2", refund_processed("rfnd_2", "pay_1", 30_000, None)).await;
        assert_eq!(outcome(&over), "refund_exceeds_payment");
        assert_eq!(succeeded_refunds(&shop).await, 30_000);
        assert_eq!(access(&shop).await, "allowed", "a partial refund does not end access");
        // The rest, to the paisa, is fine, and completes the refund.
        let rest = deliver(&shop, "evt_3", refund_processed("rfnd_3", "pay_1", PRICE - 30_000, None)).await;
        assert_eq!(outcome(&rest), "refund_applied");
        assert_eq!(order_status(&shop).await, "refunded");
        assert_eq!(access(&shop).await, "access_revoked");
        assert_eq!(succeeded_refunds(&shop).await, PRICE);
        // The refused one, sent again, is not applied by the second try.
        let again = deliver(&shop, "evt_4", refund_processed("rfnd_2", "pay_1", 30_000, None)).await;
        assert_ne!(outcome(&again), "refund_applied");
        assert_eq!(succeeded_refunds(&shop).await, PRICE);
        integrity_is_clean(&shop).await;
    }
}

db_test! {
    async fn the_database_itself_refuses_refunds_past_the_payment(pool: PgPool) {
        let shop = shop(pool).await;
        pay(&shop, "evt_p", "pay_1").await;
        let payment = payment_id(&shop, "pay_1").await;

        // Asking for more is only a request; returning more is what the book cannot allow.
        assert!(insert_refund(&shop, payment, PRICE * 10, "requested").await.is_ok());
        let past = insert_refund(&shop, payment, PRICE + 1, "succeeded").await.unwrap_err().to_string();
        assert!(past.contains("more than the"), "{past}");
        assert!(insert_refund(&shop, payment, 40_000, "succeeded").await.is_ok());
        assert!(insert_refund(&shop, payment, 9_901, "succeeded").await.is_err(), "40,000 + 9,901 is past 49,900");
        assert!(insert_refund(&shop, payment, 9_900, "succeeded").await.is_ok(), "40,000 + 9,900 is exactly 49,900");
        // Changing a recorded refund is held to the same rule.
        let grown = sqlx::query("update refunds set amount_paise = amount_paise + 1 where amount_paise = 9900")
            .execute(&shop.pool).await;
        assert!(grown.is_err());
        // Marking a request as succeeded counts it then.
        let promoted = sqlx::query("update refunds set status = 'succeeded', completed_at = now() where status = 'requested'")
            .execute(&shop.pool).await;
        assert!(promoted.is_err());
        assert_eq!(succeeded_refunds(&shop).await, PRICE);
    }
}

db_test! {
    async fn two_refunds_racing_for_one_payment_cannot_both_be_recorded(pool: PgPool) {
        let shop = shop(pool).await;
        pay(&shop, "evt_p", "pay_1").await;
        let payment = payment_id(&shop, "pay_1").await;

        let tries = futures_util::future::join_all((0..2).map(|_| insert_refund(&shop, payment, 30_000, "succeeded"))).await;

        // 30,000 + 30,000 is past 49,900, so whichever got the payment first is the only one.
        assert_eq!(tries.iter().filter(|t| t.is_ok()).count(), 1, "{tries:?}");
        assert_eq!(succeeded_refunds(&shop).await, 30_000);
    }
}
