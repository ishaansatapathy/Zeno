//! Payouts: a creator asks, the money is reserved in the ledger at once, an admin sends it by hand
//! and records the bank's reference. Money inside a refund window is never payable, and however many
//! requests race, no more is reserved than there was.

mod common;

use axum::http::StatusCode;
use common::shop::*;
use common::*;
use futures_util::future::join_all;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;
use zeno_api::auth::extractor::AuthUser;
use zeno_api::error::ApiError;
use zeno_api::payouts::service;
use zeno_core::model::Role;

macro_rules! db_test {
    ($(#[$meta:meta])* async fn $name:ident($pool:ident: PgPool) $body:block) => {
        #[sqlx::test(migrator = "zeno_core::db::MIGRATOR")]
        $(#[$meta])*
        async fn $name($pool: PgPool) $body
    };
}

/// The sale nets Rs 484 (a fee of Rs 15 was taken) and an admin adds Rs 1,000.
const NET: i64 = PRICE - 1_500;
const CREDIT: i64 = 100_000;
const AVAILABLE: i64 = NET + CREDIT;

/// A creator with a sale that is past its refund window and a credit from an admin: `AVAILABLE`
/// paise payable.
async fn funded(pool: PgPool) -> (Shop, Jar) {
    let shop = shop(pool).await;
    sell(&shop).await;
    sqlx::query("update orders set paid_at = now() - interval '30 days'")
        .execute(&shop.pool)
        .await
        .unwrap();
    let admin = admin_jar(&shop).await;
    let credit = shop.app.post("/v1/admin/ledger/adjustments").jar(&admin)
        .json(json!({"creator_id": shop.creator_id, "amount_paise": CREDIT, "reason": "Seed balance for a test"}))
        .send().await;
    assert_eq!(credit.status, StatusCode::CREATED, "{:?}", credit.body);
    (shop, admin)
}

async fn sell(shop: &Shop) -> String {
    let payment = shop.provider.capture(&shop.remote).unwrap();
    let res = deliver(
        shop,
        &format!("evt_{}", Uuid::new_v4()),
        captured_with_fee(&payment.id, &shop.remote, PRICE, Some(1_500)),
    )
    .await;
    assert_eq!(outcome(&res), "enrolled", "{:?}", res.body);
    payment.id
}

async fn request(shop: &Shop, amount: i64) -> Res {
    shop.app
        .post("/v1/me/payouts")
        .jar(&shop.creator)
        .json(json!({"amount_paise": amount}))
        .send()
        .await
}

fn payout_id(res: &Res) -> Uuid {
    Uuid::parse_str(res.body["payout"]["id"].as_str().unwrap()).unwrap()
}

async fn earnings(shop: &Shop) -> Value {
    shop.app
        .get("/v1/me/earnings")
        .jar(&shop.creator)
        .send()
        .await
        .body["earnings"]
        .clone()
}

async fn admin_pay(shop: &Shop, admin: &Jar, id: Uuid, method: &str, reference: &str) -> Res {
    shop.app
        .post(&format!("/v1/admin/payouts/{id}/pay"))
        .jar(admin)
        .json(json!({"method": method, "reference": reference}))
        .send()
        .await
}

async fn admin_cancel(shop: &Shop, admin: &Jar, id: Uuid, reason: Option<&str>) -> Res {
    let body = match reason {
        Some(reason) => json!({"reason": reason}),
        None => json!({}),
    };
    shop.app
        .post(&format!("/v1/admin/payouts/{id}/cancel"))
        .jar(admin)
        .json(body)
        .send()
        .await
}

async fn creator_total(shop: &Shop) -> i64 {
    n(shop, &format!("select coalesce(sum(amount_paise), 0)::bigint from ledger_entries where creator_id = '{}'", shop.creator_id)).await
}

// ---- asking ----------------------------------------------------------------------------------------------

db_test! {
    async fn a_request_takes_the_money_out_of_the_balance_at_once(pool: PgPool) {
        let (shop, _) = funded(pool).await;
        assert_eq!(earnings(&shop).await["available_paise"], AVAILABLE);

        let res = request(&shop, 60_000).await;

        assert_eq!(res.status, StatusCode::CREATED, "{:?}", res.body);
        assert_eq!(res.body["payout"]["status"], "requested");
        assert_eq!(res.body["payout"]["amount_paise"], 60_000);
        assert_eq!(creator_total(&shop).await, AVAILABLE - 60_000);
        assert_eq!(n(&shop, "select coalesce(sum(amount_paise), 0)::bigint from ledger_entries where account = 'payouts'").await, 60_000);
        let seen = earnings(&shop).await;
        assert_eq!(seen["balance_paise"], AVAILABLE - 60_000);
        assert_eq!(seen["available_paise"], AVAILABLE - 60_000);
        assert_eq!(seen["lifetime"]["paid_out_paise"], 60_000);
        assert_eq!(seen["open_payout"]["id"], res.body["payout"]["id"]);
        assert_eq!(n(&shop, "select count(*) from ledger_journals where kind = 'payout'").await, 1);
    }
}

db_test! {
    async fn only_what_is_payable_and_at_least_the_minimum_can_be_requested(pool: PgPool) {
        let (shop, _) = funded(pool).await;
        for amount in [0, -5_000, 49_999] {
            let res = request(&shop, amount).await;
            assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{amount}: {:?}", res.body);
        }
        let too_much = request(&shop, AVAILABLE + 1).await;
        assert_eq!((too_much.status, too_much.code()), (StatusCode::CONFLICT, "insufficient_funds"));
        assert!(too_much.body["error"]["message"].as_str().unwrap().contains("Rs 1484.00"), "{:?}", too_much.body);
        assert_eq!(n(&shop, "select count(*) from payouts").await, 0);
        assert_eq!(n(&shop, "select count(*) from ledger_journals where kind = 'payout'").await, 0);
        // All of it, to the paisa, is fine.
        assert_eq!(request(&shop, AVAILABLE).await.status, StatusCode::CREATED);
        assert_eq!(creator_total(&shop).await, 0);
    }
}

db_test! {
    async fn money_inside_a_refund_window_cannot_be_requested(pool: PgPool) {
        let shop = shop(pool).await;
        sell(&shop).await; // just paid: the whole net is pending
        let admin = admin_jar(&shop).await;
        shop.app.post("/v1/admin/ledger/adjustments").jar(&admin)
            .json(json!({"creator_id": shop.creator_id, "amount_paise": 20_000, "reason": "Seed balance for a test"})).send().await;
        // Balance is NET + 20,000 but only the credit is past any window.
        let seen = earnings(&shop).await;
        assert_eq!((seen["balance_paise"].as_i64(), seen["available_paise"].as_i64()), (Some(NET + 20_000), Some(20_000)));
        let res = request(&shop, 50_000).await;
        assert_eq!((res.status, res.code()), (StatusCode::CONFLICT, "insufficient_funds"));
    }
}

db_test! {
    async fn one_request_at_a_time(pool: PgPool) {
        let (shop, admin) = funded(pool).await;
        let first = request(&shop, 60_000).await;
        let second = request(&shop, 60_000).await;
        assert_eq!((second.status, second.code()), (StatusCode::CONFLICT, "payout_in_progress"));
        assert_eq!(creator_total(&shop).await, AVAILABLE - 60_000, "the refused request reserved nothing");
        // Once the first is dealt with, another may be made.
        admin_pay(&shop, &admin, payout_id(&first), "bank_transfer", "UTR 1234 5678").await;
        assert_eq!(request(&shop, 60_000).await.status, StatusCode::CREATED);
    }
}

db_test! {
    async fn twenty_requests_at_once_reserve_the_money_once(pool: PgPool) {
        let (shop, _) = funded(pool).await;
        let user = AuthUser { id: shop.creator_id, role: Role::Creator, session_id: Uuid::new_v4() };
        let results = join_all((0..20).map(|_| service::request(&shop.app.state, &user, AVAILABLE))).await;
        let won = results.iter().filter(|r| r.is_ok()).count();
        let in_progress = results.iter().filter(|r| matches!(r, Err(ApiError::Http { code: "payout_in_progress", .. }))).count();
        let insufficient = results.iter().filter(|r| matches!(r, Err(ApiError::Http { code: "insufficient_funds", .. }))).count();
        assert_eq!((won, in_progress + insufficient), (1, 19), "{:?}", results.iter().map(|r| r.as_ref().err().map(|e| format!("{e:?}"))).collect::<Vec<_>>());
        assert_eq!(creator_total(&shop).await, 0, "everything was reserved, once");
        assert_eq!(n(&shop, "select count(*) from payouts").await, 1);
    }
}

db_test! {
    async fn the_minimum_is_a_setting(pool: PgPool) {
        let shop = shop_with_config(pool, |config| config.ledger.payout_minimum_paise = 100).await;
        sell(&shop).await;
        sqlx::query("update orders set paid_at = now() - interval '30 days'").execute(&shop.pool).await.unwrap();
        assert_eq!(request(&shop, 99).await.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(request(&shop, 100).await.status, StatusCode::CREATED);
        assert_eq!(earnings(&shop).await["minimum_payout_paise"], 100);
    }
}

db_test! {
    async fn asking_for_payouts_over_and_over_is_slowed(pool: PgPool) {
        let (shop, _) = funded(pool).await;
        let mut statuses = Vec::new();
        for _ in 0..12 {
            statuses.push(request(&shop, 0).await.status); // each is refused, and each counts
        }
        assert_eq!(statuses.iter().filter(|s| **s == StatusCode::TOO_MANY_REQUESTS).count(), 2, "{statuses:?}");
    }
}

// ---- cancelling ----------------------------------------------------------------------------------------------

db_test! {
    async fn a_creator_withdraws_a_request_and_the_money_returns(pool: PgPool) {
        let (shop, _) = funded(pool).await;
        let res = request(&shop, 60_000).await;
        let id = payout_id(&res);

        let cancelled = shop.app.post(&format!("/v1/me/payouts/{id}/cancel")).jar(&shop.creator).json(json!({})).send().await;

        assert_eq!(cancelled.status, StatusCode::OK, "{:?}", cancelled.body);
        assert_eq!(cancelled.body["payout"]["status"], "cancelled");
        assert_eq!(creator_total(&shop).await, AVAILABLE);
        assert_eq!(n(&shop, "select count(*) from ledger_journals where kind = 'payout_reversal'").await, 1);
        // The history is kept: the request and its reversal are both in the book.
        assert_eq!(n(&shop, "select count(*) from ledger_journals where kind = 'payout'").await, 1);
        assert_eq!(earnings(&shop).await["lifetime"]["paid_out_paise"], 0);
        assert!(earnings(&shop).await["open_payout"].is_null());
        // Cancelling twice is refused; so is paying a cancelled payout.
        let again = shop.app.post(&format!("/v1/me/payouts/{id}/cancel")).jar(&shop.creator).json(json!({})).send().await;
        assert_eq!((again.status, again.code()), (StatusCode::CONFLICT, "payout_not_open"));
        assert_eq!(n(&shop, "select count(*) from ledger_journals where kind = 'payout_reversal'").await, 1);
    }
}

db_test! {
    async fn nobody_else_can_cancel_someone_elses_payout(pool: PgPool) {
        let (shop, admin) = funded(pool).await;
        let id = payout_id(&request(&shop, 60_000).await);
        shop.app.settle().await;
        let rival = signup_creator(&shop.app, "rival@payouts.in").await;
        let url = format!("/v1/me/payouts/{id}/cancel");
        let post = |jar: &Jar| shop.app.post(&url).jar(jar).json(json!({})).send();
        assert_eq!(post(&rival).await.status, StatusCode::NOT_FOUND);
        assert_eq!(post(&admin).await.status, StatusCode::NOT_FOUND, "an admin cancels through the admin route");
        assert_eq!(post(&shop.buyer).await.status, StatusCode::FORBIDDEN);
        assert_eq!(shop.app.post(&url).json(json!({})).send().await.status, StatusCode::UNAUTHORIZED);
        assert_eq!(shop.app.post("/v1/me/payouts/not-an-id/cancel").jar(&shop.creator).json(json!({})).send().await.status, StatusCode::NOT_FOUND);
        assert_eq!(creator_total(&shop).await, AVAILABLE - 60_000, "nothing changed");
    }
}

// ---- an admin pays ---------------------------------------------------------------------------------------------

db_test! {
    async fn an_admin_records_the_transfer_and_the_creator_is_told(pool: PgPool) {
        let (shop, admin) = funded(pool).await;
        let id = payout_id(&request(&shop, 60_000).await);
        shop.app.settle().await;
        let mails = shop.app.mailer.sent().len();

        let res = admin_pay(&shop, &admin, id, "bank_transfer", "UTR 4021 9988 1").await;

        assert_eq!(res.status, StatusCode::OK, "{:?}", res.body);
        assert_eq!(res.body["payout"]["status"], "paid");
        assert_eq!(res.body["payout"]["method"], "bank_transfer");
        assert_eq!(res.body["payout"]["reference"], "UTR 4021 9988 1");
        assert!(res.body["payout"]["paid_at"].is_string());
        // Paying changes no money in the book: it was reserved when it was asked for.
        assert_eq!(creator_total(&shop).await, AVAILABLE - 60_000);
        assert_eq!(n(&shop, "select count(*) from ledger_journals where kind in ('payout', 'payout_reversal')").await, 1);
        let mail = shop.app.mailer.wait_for(mails + 1).await.pop().unwrap();
        assert_eq!(mail.to, "teacher@hooks.in");
        assert!(mail.text.contains("Rs 600.00") && mail.text.contains("UTR 4021 9988 1"), "{}", mail.text);
        assert_eq!(n(&shop, "select count(*) from payouts where decided_by is not null").await, 1);

        // A paid payout is final.
        let again = admin_pay(&shop, &admin, id, "upi", "another-ref").await;
        assert_eq!((again.status, again.code()), (StatusCode::CONFLICT, "payout_not_open"));
        let cancel = admin_cancel(&shop, &admin, id, Some("too late")).await;
        assert_eq!((cancel.status, cancel.code()), (StatusCode::CONFLICT, "payout_not_open"));
        let own = shop.app.post(&format!("/v1/me/payouts/{id}/cancel")).jar(&shop.creator).json(json!({})).send().await;
        assert_eq!((own.status, own.code()), (StatusCode::CONFLICT, "payout_not_open"));
    }
}

db_test! {
    async fn a_payment_needs_a_method_and_the_banks_reference(pool: PgPool) {
        let (shop, admin) = funded(pool).await;
        let id = payout_id(&request(&shop, 60_000).await);
        for (method, reference) in [("cash", "UTR123456"), ("upi", "ab"), ("upi", ""), ("upi", "line\nbreak"), ("", "UTR123456")] {
            let res = admin_pay(&shop, &admin, id, method, reference).await;
            assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{method} {reference:?}");
        }
        let missing = admin_pay(&shop, &admin, Uuid::new_v4(), "upi", "UTR123456").await;
        assert_eq!(missing.status, StatusCode::NOT_FOUND);
        assert_eq!(text(&shop, "select status::text from payouts").await, "requested");
    }
}

db_test! {
    async fn an_admin_can_decline_with_a_reason_and_the_money_returns(pool: PgPool) {
        let (shop, admin) = funded(pool).await;
        let id = payout_id(&request(&shop, 60_000).await);
        shop.app.settle().await;
        let mails = shop.app.mailer.sent().len();

        assert_eq!(admin_cancel(&shop, &admin, id, None).await.status, StatusCode::UNPROCESSABLE_ENTITY, "a reason is required");
        let res = admin_cancel(&shop, &admin, id, Some("Bank details on file are out of date")).await;

        assert_eq!(res.status, StatusCode::OK, "{:?}", res.body);
        assert_eq!(res.body["payout"]["cancel_reason"], "Bank details on file are out of date");
        assert_eq!(creator_total(&shop).await, AVAILABLE);
        let mail = shop.app.mailer.wait_for(mails + 1).await.pop().unwrap();
        assert!(mail.text.contains("Bank details on file are out of date"), "{}", mail.text);
    }
}

db_test! {
    async fn paying_and_cancelling_at_the_same_moment_has_one_winner(pool: PgPool) {
        let (shop, admin) = funded(pool).await;
        let id = payout_id(&request(&shop, 60_000).await);
        let (paid, cancelled) = futures_util::future::join(
            admin_pay(&shop, &admin, id, "bank_transfer", "UTR 777 888 999"),
            shop.app.post(&format!("/v1/me/payouts/{id}/cancel")).jar(&shop.creator).json(json!({})).send(),
        ).await;
        let outcomes = (paid.status, cancelled.status);
        assert!(
            outcomes == (StatusCode::OK, StatusCode::CONFLICT) || outcomes == (StatusCode::CONFLICT, StatusCode::OK),
            "{outcomes:?}"
        );
        let status = text(&shop, "select status::text from payouts").await;
        let reversals = n(&shop, "select count(*) from ledger_journals where kind = 'payout_reversal'").await;
        match status.as_str() {
            "paid" => assert_eq!((reversals, creator_total(&shop).await), (0, AVAILABLE - 60_000)),
            "cancelled" => assert_eq!((reversals, creator_total(&shop).await), (1, AVAILABLE)),
            other => panic!("{other}"),
        }
    }
}

db_test! {
    async fn only_admins_touch_the_payout_queue(pool: PgPool) {
        let (shop, _admin) = funded(pool).await;
        let id = payout_id(&request(&shop, 60_000).await);
        let attempts = [
            ("get", "/v1/admin/payouts".to_owned(), json!({})),
            ("post", format!("/v1/admin/payouts/{id}/pay"), json!({"method": "upi", "reference": "UTR123456"})),
            ("post", format!("/v1/admin/payouts/{id}/cancel"), json!({"reason": "because"})),
        ];
        for (verb, url, body) in attempts {
            for (who, jar, expected) in [("creator", Some(&shop.creator), StatusCode::FORBIDDEN), ("student", Some(&shop.buyer), StatusCode::FORBIDDEN), ("anonymous", None, StatusCode::UNAUTHORIZED)] {
                let mut request = if verb == "get" { shop.app.get(&url) } else { shop.app.post(&url) };
                if let Some(jar) = jar { request = request.jar(jar); }
                let res = request.json(body.clone()).send().await;
                assert_eq!(res.status, expected, "{who} {verb} {url}");
            }
        }
        assert_eq!(text(&shop, "select status::text from payouts").await, "requested");
    }
}

db_test! {
    async fn the_queue_shows_who_is_waiting_oldest_first(pool: PgPool) {
        let (shop, admin) = funded(pool).await;
        let first = payout_id(&request(&shop, 60_000).await);
        // A second creator, with their own earnings, also asks.
        shop.app.settle().await;
        let rival = signup_creator(&shop.app, "rival@payouts.in").await;
        let rival_id: Uuid = sqlx::query_scalar("select id from users where email = 'rival@payouts.in'").fetch_one(&shop.pool).await.unwrap();
        shop.app.post("/v1/admin/ledger/adjustments").jar(&admin)
            .json(json!({"creator_id": rival_id, "amount_paise": 70_000, "reason": "Seed balance for a test"})).send().await;
        let theirs = shop.app.post("/v1/me/payouts").jar(&rival).json(json!({"amount_paise": 70_000})).send().await;
        assert_eq!(theirs.status, StatusCode::CREATED, "{:?}", theirs.body);

        let queue = shop.app.get("/v1/admin/payouts?status=requested").jar(&admin).send().await;
        let payouts = queue.body["payouts"].as_array().unwrap();
        assert_eq!(payouts.len(), 2);
        assert_eq!(payouts[0]["id"], first.to_string(), "the one waiting longest is first");
        assert_eq!(payouts[0]["creator_name"], "Asha Rao");
        assert_eq!(payouts[0]["creator_email"], "teacher@hooks.in");
        assert_eq!(payouts[1]["creator_email"], "rival@payouts.in");

        admin_pay(&shop, &admin, first, "upi", "UPI-REF-1").await;
        let waiting = shop.app.get("/v1/admin/payouts?status=requested").jar(&admin).send().await;
        assert_eq!(waiting.body["payouts"].as_array().unwrap().len(), 1);
        let paid = shop.app.get("/v1/admin/payouts?status=paid").jar(&admin).send().await;
        assert_eq!(paid.body["payouts"].as_array().unwrap().len(), 1);
        assert_eq!(shop.app.get("/v1/admin/payouts?status=bogus").jar(&admin).send().await.status, StatusCode::UNPROCESSABLE_ENTITY);
    }
}

db_test! {
    async fn a_creator_sees_only_their_own_payouts_newest_first(pool: PgPool) {
        let (shop, admin) = funded(pool).await;
        let first = payout_id(&request(&shop, 50_000).await);
        admin_pay(&shop, &admin, first, "upi", "UPI-REF-1").await;
        let second = payout_id(&request(&shop, 50_000).await);
        shop.app.settle().await;
        let rival = signup_creator(&shop.app, "rival@payouts.in").await;

        let mine = shop.app.get("/v1/me/payouts").jar(&shop.creator).send().await;
        let ids: Vec<&str> = mine.body["payouts"].as_array().unwrap().iter().map(|p| p["id"].as_str().unwrap()).collect();
        assert_eq!(ids, [second.to_string(), first.to_string()]);
        let theirs = shop.app.get("/v1/me/payouts").jar(&rival).send().await;
        assert_eq!(theirs.body["payouts"].as_array().unwrap().len(), 0);
        assert_eq!(shop.app.get("/v1/me/payouts").jar(&shop.buyer).send().await.status, StatusCode::FORBIDDEN);
    }
}

// ---- when a refund comes after the money has gone -----------------------------------------------------------

db_test! {
    async fn a_refund_after_a_payout_leaves_a_debt_that_later_sales_pay_off(pool: PgPool) {
        let (shop, admin) = funded(pool).await;
        // Everything is paid out...
        let id = payout_id(&request(&shop, AVAILABLE).await);
        admin_pay(&shop, &admin, id, "bank_transfer", "UTR 111 222 333").await;
        assert_eq!(creator_total(&shop).await, 0);

        // ...and then the buyer is refunded in full.
        let refund = shop.app.post(&format!("/v1/orders/{}/refund", shop.order)).jar(&shop.creator).json(json!({})).send().await;
        assert_eq!(refund.status, StatusCode::ACCEPTED, "{:?}", refund.body);
        let seen = earnings(&shop).await;
        assert_eq!(seen["balance_paise"], -PRICE, "the creator owes Zeno what was refunded");
        assert_eq!(seen["available_paise"], 0);
        let refused = request(&shop, 50_000).await;
        assert_eq!((refused.status, refused.code()), (StatusCode::CONFLICT, "insufficient_funds"));

        // A new sale (past its window) pays off part of the debt first.
        shop.app.settle().await;
        let buyer = signup_verified_as(&shop.app, "second@payouts.in", "student").await;
        let started = shop.app.post(&format!("/v1/checkout/{}", shop.course)).jar(&buyer).send().await;
        let remote = started.body["checkout"]["provider_order_id"].as_str().unwrap().to_owned();
        let payment = shop.provider.capture(&remote).unwrap();
        deliver(&shop, "evt_next", captured_with_fee(&payment.id, &remote, PRICE, Some(1_500))).await;
        sqlx::query("update orders set paid_at = now() - interval '30 days'").execute(&shop.pool).await.unwrap();
        let after = earnings(&shop).await;
        assert_eq!(after["balance_paise"], -PRICE + NET);
        assert_eq!(after["available_paise"], 0, "still in debt: {after}");
    }
}
