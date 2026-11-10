//! Coupons: a creator's offers, priced by the server, redeemed at most as often as allowed however
//! many people arrive at once, and given back when an order is never paid.

mod common;

use axum::http::StatusCode;
use common::shop::*;
use common::*;
use futures_util::future::join_all;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;
use zeno_api::auth::extractor::AuthUser;
use zeno_api::orders::service;
use zeno_api::reconcile::run_once;
use zeno_core::model::Role;
use zeno_core::payments::Op;

macro_rules! db_test {
    ($(#[$meta:meta])* async fn $name:ident($pool:ident: PgPool) $body:block) => {
        #[sqlx::test(migrator = "zeno_core::db::MIGRATOR")]
        $(#[$meta])*
        async fn $name($pool: PgPool) $body
    };
}

fn percent(code: &str, value: i64) -> Value {
    json!({"code": code, "kind": "percent", "value": value})
}

fn flat(code: &str, value: i64) -> Value {
    json!({"code": code, "kind": "flat", "value": value})
}

async fn create(shop: &Shop, body: Value) -> Res {
    shop.app
        .post("/v1/coupons")
        .jar(&shop.creator)
        .json(body)
        .send()
        .await
}

async fn make(shop: &Shop, body: Value) -> Uuid {
    let res = create(shop, body).await;
    assert_eq!(res.status, StatusCode::CREATED, "{:?}", res.body);
    Uuid::parse_str(res.body["coupon"]["id"].as_str().unwrap()).unwrap()
}

async fn checkout_with(shop: &Shop, who: &Jar, coupon: Option<&str>) -> Res {
    let request = shop
        .app
        .post(&format!("/v1/checkout/{}", shop.course))
        .jar(who);
    match coupon {
        Some(code) => request.json(json!({"coupon": code})).send().await,
        None => request.send().await,
    }
}

async fn preview(shop: &Shop, who: &Jar, coupon: &str) -> Res {
    shop.app
        .post(&format!("/v1/checkout/{}/preview", shop.course))
        .jar(who)
        .json(json!({"coupon": coupon}))
        .send()
        .await
}

async fn used(shop: &Shop, code: &str) -> i64 {
    n(
        shop,
        &format!("select redeemed_count::bigint from coupons where code = '{code}'"),
    )
    .await
}

fn amount(res: &Res) -> i64 {
    res.body["order"]["amount_paise"].as_i64().unwrap_or(-1)
}

fn code_of(res: &Res) -> &str {
    res.body["error"]["code"].as_str().unwrap_or("")
}

async fn another_buyer(shop: &Shop, email: &str) -> Jar {
    shop.app.settle().await;
    signup_verified_as(&shop.app, email, "student").await
}

// ---- running a creator's coupons ------------------------------------------------------------

db_test! {
    async fn a_creator_makes_coupons_and_codes_are_canonical(pool: PgPool) {
        let shop = shop(pool).await;
        let res = create(&shop, json!({
            "code": "  launch50 ", "kind": "percent", "value": 50,
            "max_redemptions": 100, "per_user_limit": 2
        })).await;
        assert_eq!(res.status, StatusCode::CREATED, "{:?}", res.body);
        let coupon = &res.body["coupon"];
        assert_eq!(coupon["code"], "LAUNCH50");
        assert_eq!((coupon["kind"].as_str(), coupon["value"].as_i64()), (Some("percent"), Some(50)));
        assert_eq!((coupon["max_redemptions"].as_i64(), coupon["per_user_limit"].as_i64()), (Some(100), Some(2)));
        assert_eq!((coupon["redeemed_count"].as_i64(), coupon["active"].as_bool()), (Some(0), Some(true)));

        // The same code in any case is the same code.
        for again in ["LAUNCH50", "launch50", " Launch50"] {
            let res = create(&shop, percent(again, 10)).await;
            assert_eq!((res.status, res.code()), (StatusCode::CONFLICT, "coupon_exists"), "{again}");
        }
        // But another creator's namespace is their own.
        shop.app.settle().await;
        let other = signup_creator(&shop.app, "rival@coupons.in").await;
        let theirs = shop.app.post("/v1/coupons").jar(&other).json(percent("LAUNCH50", 10)).send().await;
        assert_eq!(theirs.status, StatusCode::CREATED, "{:?}", theirs.body);
        // Each sees only their own.
        let mine = shop.app.get("/v1/coupons").jar(&shop.creator).send().await;
        assert_eq!(mine.body["coupons"].as_array().unwrap().len(), 1);
        let rivals = shop.app.get("/v1/coupons").jar(&other).send().await;
        assert_eq!(rivals.body["coupons"].as_array().unwrap().len(), 1);
        assert_ne!(rivals.body["coupons"][0]["id"], mine.body["coupons"][0]["id"]);
    }
}

db_test! {
    async fn bad_coupons_are_refused_with_the_field_at_fault(pool: PgPool) {
        let shop = shop(pool).await;
        let past = (chrono::Utc::now() - chrono::Duration::days(1)).to_rfc3339();
        let cases: Vec<(Value, &str)> = vec![
            (percent("ab", 10), "code"),
            (percent("has space", 10), "code"),
            (percent("OK-CODE", 0), "value"),
            (percent("OK-CODE", 101), "value"),
            (flat("OK-CODE", 99), "value"),
            (json!({"code": "OK-CODE", "kind": "bogus", "value": 5}), "kind"),
            (json!({"code": "OK-CODE", "kind": "percent", "value": 5, "max_redemptions": 0}), "max_redemptions"),
            (json!({"code": "OK-CODE", "kind": "percent", "value": 5, "per_user_limit": 0}), "per_user_limit"),
            (json!({"code": "OK-CODE", "kind": "percent", "value": 5, "valid_until": past}), "valid_until"),
            (json!({"code": "OK-CODE", "kind": "percent", "value": 5, "course_id": Uuid::new_v4()}), "course_id"),
        ];
        for (body, field) in cases {
            let res = create(&shop, body.clone()).await;
            assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
            let fields: Vec<&str> = res.body["error"]["fields"].as_array().unwrap()
                .iter().map(|f| f["field"].as_str().unwrap()).collect();
            assert!(fields.contains(&field), "{body}: {fields:?}");
        }
        // Unknown fields are not silently dropped.
        let res = create(&shop, json!({"code": "OK-CODE", "kind": "percent", "value": 5, "free": true})).await;
        assert!(res.status.is_client_error());
        assert_eq!(n(&shop, "select count(*) from coupons").await, 0);
    }
}

db_test! {
    async fn a_coupon_can_be_limited_to_one_of_the_creators_courses(pool: PgPool) {
        let shop = shop(pool).await;
        let mut body = percent("ONLYHERE", 20);
        body["course_id"] = json!(shop.course);
        assert_eq!(create(&shop, body).await.status, StatusCode::CREATED);
        // Not someone else's course.
        shop.app.settle().await;
        let rival = signup_creator(&shop.app, "rival@coupons.in").await;
        let (theirs, _) = make_course(&shop.app, &shop.pool, &rival, "Rival course").await;
        let mut body = percent("STOLEN", 20);
        body["course_id"] = json!(theirs);
        let res = create(&shop, body).await;
        assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    }
}

db_test! {
    async fn only_creators_use_the_coupon_api_and_only_owners_change_a_coupon(pool: PgPool) {
        let shop = shop(pool).await;
        let id = make(&shop, percent("MINE", 10)).await;
        assert_eq!(shop.app.post("/v1/coupons").jar(&shop.buyer).json(percent("X-STU", 10)).send().await.status, StatusCode::FORBIDDEN);
        assert_eq!(shop.app.get("/v1/coupons").jar(&shop.buyer).send().await.status, StatusCode::FORBIDDEN);
        assert_eq!(shop.app.get("/v1/coupons").send().await.status, StatusCode::UNAUTHORIZED);
        shop.app.settle().await;
        let rival = signup_creator(&shop.app, "rival@coupons.in").await;
        let url = format!("/v1/coupons/{id}");
        let patch = |jar: &Jar, body: Value| shop.app.req(axum::http::Method::PATCH, &url).jar(jar).json(body).send();
        assert_eq!(patch(&rival, json!({"active": false})).await.status, StatusCode::NOT_FOUND);
        assert_eq!(patch(&shop.buyer, json!({"active": false})).await.status, StatusCode::FORBIDDEN);
        let off = patch(&shop.creator, json!({"active": false})).await;
        assert_eq!(off.status, StatusCode::OK, "{:?}", off.body);
        assert_eq!(off.body["coupon"]["active"], false);
        assert_eq!(patch(&shop.creator, json!({})).await.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(patch(&shop.creator, json!({"value": 90})).await.status, StatusCode::UNPROCESSABLE_ENTITY, "the discount itself is fixed");
    }
}

db_test! {
    async fn a_limit_cannot_be_lowered_below_what_is_already_used(pool: PgPool) {
        let shop = shop(pool).await;
        let id = make(&shop, json!({"code": "SOME", "kind": "percent", "value": 10, "max_redemptions": 5})).await;
        checkout_with(&shop, &shop.buyer, Some("SOME")).await;
        let url = format!("/v1/coupons/{id}");
        let set = |max: i64| shop.app.req(axum::http::Method::PATCH, &url).jar(&shop.creator).json(json!({"max_redemptions": max})).send();
        assert_eq!(set(0).await.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(set(1).await.status, StatusCode::OK);
        shop.app.settle().await;
        let other = another_buyer(&shop, "second@coupons.in").await;
        assert_eq!(code_of(&checkout_with(&shop, &other, Some("SOME")).await), "coupon_exhausted");
    }
}

db_test! {
    async fn a_creator_cannot_hoard_coupons_without_limit(pool: PgPool) {
        let shop = shop(pool).await;
        sqlx::query(
            "insert into coupons (creator_id, code, kind, value) \
             select $1, 'CODE' || g, 'percent', 10 from generate_series(1, 500) g",
        )
        .bind(shop.creator_id)
        .execute(&shop.pool)
        .await
        .unwrap();
        let res = create(&shop, percent("ONEMORE", 10)).await;
        assert_eq!((res.status, res.code()), (StatusCode::CONFLICT, "coupon_limit"));
    }
}

// ---- the price a coupon gives -----------------------------------------------------------------------

db_test! {
    async fn a_preview_shows_the_price_without_using_anything(pool: PgPool) {
        let shop = shop(pool).await;
        make(&shop, percent("HALF", 50)).await;
        make(&shop, flat("MINUS100", 10_000)).await;
        make(&shop, percent("ALL", 100)).await;
        make(&shop, flat("NEARLY", 49_850)).await;

        let half = preview(&shop, &shop.buyer, "half").await;
        assert_eq!(half.status, StatusCode::OK, "{:?}", half.body);
        let price = &half.body["price"];
        assert_eq!((price["list_price_paise"].as_i64(), price["discount_paise"].as_i64(), price["amount_paise"].as_i64()),
                   (Some(49_900), Some(24_950), Some(24_950)));
        assert_eq!(price["free"], false);
        assert_eq!(price["coupon"]["code"], "HALF");
        let flat = preview(&shop, &shop.buyer, " Minus100 ").await;
        assert_eq!(flat.body["price"]["amount_paise"], 39_900);
        assert_eq!(preview(&shop, &shop.buyer, "ALL").await.body["price"]["free"], true);
        // Held at one rupee, not below.
        assert_eq!(preview(&shop, &shop.buyer, "NEARLY").await.body["price"]["amount_paise"], 100);

        // Looking uses nothing and makes nothing.
        assert_eq!(used(&shop, "HALF").await, 0);
        assert_eq!(n(&shop, "select count(*) from coupon_redemptions").await, 0);
        assert_eq!(shop.provider.orders().len(), 1, "only the order from before");
    }
}

db_test! {
    async fn a_code_that_is_not_usable_says_why(pool: PgPool) {
        let shop = shop(pool).await;
        let future = (chrono::Utc::now() + chrono::Duration::days(2)).to_rfc3339();
        make(&shop, json!({"code": "LATER", "kind": "percent", "value": 10, "valid_from": future})).await;
        make(&shop, percent("OFF", 10)).await;
        sqlx::query("update coupons set active = false where code = 'OFF'").execute(&shop.pool).await.unwrap();
        make(&shop, percent("ENDED", 10)).await;
        sqlx::query("update coupons set valid_until = now() - interval '1 hour' where code = 'ENDED'").execute(&shop.pool).await.unwrap();
        make(&shop, json!({"code": "GONE", "kind": "percent", "value": 10, "max_redemptions": 1})).await;
        sqlx::query("update coupons set redeemed_count = 1 where code = 'GONE'").execute(&shop.pool).await.unwrap();
        make(&shop, percent("TINY", 1)).await;
        // A course with a price so low that 1% changes nothing is covered by the pricing tests;
        // here, a coupon for a different course of the same creator.
        let (other_course, _) = make_course(&shop.app, &shop.pool, &shop.creator, "Another course").await;
        let mut scoped = percent("OTHERCOURSE", 10);
        scoped["course_id"] = json!(other_course);
        make(&shop, scoped).await;

        for (code, expected) in [
            ("LATER", "coupon_not_started"),
            ("OFF", "coupon_not_found"),
            ("ENDED", "coupon_expired"),
            ("GONE", "coupon_exhausted"),
            ("OTHERCOURSE", "coupon_not_applicable"),
            ("NOSUCHCODE", "coupon_not_found"),
            ("bad code!", "coupon_not_found"),
        ] {
            let res = preview(&shop, &shop.buyer, code).await;
            assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{code}: {:?}", res.body);
            assert_eq!(code_of(&res), expected, "{code}");
            let order = checkout_with(&shop, &shop.buyer, Some(code)).await;
            assert_eq!(code_of(&order), expected, "{code} at checkout");
        }
        assert_eq!(n(&shop, "select count(*) from coupon_redemptions").await, 0);
    }
}

db_test! {
    async fn another_creators_code_does_not_exist_for_this_course(pool: PgPool) {
        let shop = shop(pool).await;
        shop.app.settle().await;
        let rival = signup_creator(&shop.app, "rival@coupons.in").await;
        let made = shop.app.post("/v1/coupons").jar(&rival).json(percent("RIVAL50", 50)).send().await;
        assert_eq!(made.status, StatusCode::CREATED);
        let res = preview(&shop, &shop.buyer, "RIVAL50").await;
        assert_eq!(code_of(&res), "coupon_not_found", "it is not revealed that the code exists");
    }
}

// ---- checking out with one -----------------------------------------------------------------------------

db_test! {
    async fn checkout_with_a_coupon_charges_the_discounted_price_at_the_provider(pool: PgPool) {
        let shop = shop(pool).await;
        make(&shop, percent("HALF", 50)).await;
        let started = shop.app.post(&format!("/v1/checkout/{}", shop.course)).jar(&shop.buyer)
            .json(json!({"coupon": "half", "amount_paise": 1, "currency": "USD"})).send().await;
        assert_eq!(started.status, StatusCode::CREATED, "{:?}", started.body);
        let order = &started.body["order"];
        assert_eq!(order["amount_paise"], 24_950);
        assert_eq!(order["list_price_paise"], PRICE);
        assert_eq!(order["discount_paise"], 24_950);
        assert_eq!(order["coupon_code"], "HALF");
        assert_eq!(started.body["checkout"]["amount_paise"], 24_950);
        // The provider was asked for the discounted amount, and the order remembers the coupon.
        let at_provider = shop.provider.orders();
        assert_eq!(at_provider.last().unwrap().amount_paise, 24_950);
        let snapshot: Value = sqlx::query_scalar("select price_snapshot from orders where status = 'pending'")
            .fetch_one(&shop.pool).await.unwrap();
        assert_eq!(snapshot["coupon"]["code"], "HALF");
        assert_eq!(snapshot["discount_paise"], 24_950);
        assert_eq!(snapshot["list_price_paise"], PRICE);
        assert_eq!(used(&shop, "HALF").await, 1);
        assert_eq!(n(&shop, "select count(*) from coupon_redemptions where order_id is not null and released_at is null").await, 1);
        let read = shop.app.get(&format!("/v1/orders/{}", order["id"].as_str().unwrap())).jar(&shop.buyer).send().await;
        assert_eq!(read.body["order"]["coupon_code"], "HALF");
    }
}

db_test! {
    async fn asking_again_with_the_same_coupon_shares_the_order_and_does_not_use_it_twice(pool: PgPool) {
        let shop = shop(pool).await;
        make(&shop, percent("HALF", 50)).await;
        let first = checkout_with(&shop, &shop.buyer, Some("HALF")).await;
        let again = checkout_with(&shop, &shop.buyer, Some("half")).await;
        assert_eq!((first.status, again.status), (StatusCode::CREATED, StatusCode::OK), "{:?}", again.body);
        assert_eq!(first.body["order"]["id"], again.body["order"]["id"]);
        assert_eq!(used(&shop, "HALF").await, 1, "one use, however often the buyer presses the button");
        // Asking with no coupon at all returns the order they have, discount and all.
        let plain = checkout_with(&shop, &shop.buyer, None).await;
        assert_eq!(plain.body["order"]["id"], first.body["order"]["id"]);
        assert_eq!(amount(&plain), 24_950);
        // The preview agrees too.
        assert_eq!(preview(&shop, &shop.buyer, "HALF").await.status, StatusCode::OK);
    }
}

db_test! {
    async fn a_different_coupon_replaces_the_open_order_and_gives_the_first_back(pool: PgPool) {
        let shop = shop(pool).await;
        make(&shop, percent("TEN", 10)).await;
        make(&shop, percent("HALF", 50)).await;
        let first = checkout_with(&shop, &shop.buyer, Some("TEN")).await;
        assert_eq!(amount(&first), 44_910);

        let second = checkout_with(&shop, &shop.buyer, Some("HALF")).await;

        assert_eq!(second.status, StatusCode::CREATED, "{:?}", second.body);
        assert_eq!(amount(&second), 24_950);
        assert_ne!(first.body["order"]["id"], second.body["order"]["id"]);
        assert_eq!((used(&shop, "TEN").await, used(&shop, "HALF").await), (0, 1));
        assert_eq!(n(&shop, "select count(*) from orders where status in ('created','pending')").await, 1);
        let old = text(&shop, &format!("select status::text from orders where id = '{}'", first.body["order"]["id"].as_str().unwrap())).await;
        assert_eq!(old, "expired");
    }
}

db_test! {
    async fn adding_a_coupon_to_the_open_order_you_already_have_re_prices_it(pool: PgPool) {
        // The page started checkout, then the buyer found a code.
        let shop = shop(pool).await;
        make(&shop, percent("HALF", 50)).await;
        let plain_id = text(&shop, "select id::text from orders").await;
        let res = checkout_with(&shop, &shop.buyer, Some("HALF")).await;
        assert_eq!(res.status, StatusCode::CREATED, "{:?}", res.body);
        assert_eq!(amount(&res), 24_950);
        assert_ne!(res.body["order"]["id"].as_str().unwrap(), plain_id);
        assert_eq!(text(&shop, &format!("select status::text from orders where id = '{plain_id}'")).await, "expired");
    }
}

db_test! {
    async fn an_order_with_a_payment_in_flight_is_not_swapped_for_a_cheaper_one(pool: PgPool) {
        let shop = shop(pool).await;
        make(&shop, percent("HALF", 50)).await;
        let paying = checkout_with(&shop, &shop.buyer, None).await;
        let remote = paying.body["checkout"]["provider_order_id"].as_str().unwrap().to_owned();
        // The customer has paid at the provider and the webhook has not arrived.
        shop.provider.capture(&remote).unwrap();

        let res = checkout_with(&shop, &shop.buyer, Some("HALF")).await;

        assert_eq!((res.status, code_of(&res)), (StatusCode::CONFLICT, "payment_processing"));
        assert_eq!(used(&shop, "HALF").await, 0, "nothing was used");
        assert_eq!(text(&shop, "select status::text from orders").await, "pending");
    }
}

db_test! {
    async fn a_body_that_is_not_json_is_refused_and_a_blank_coupon_means_none(pool: PgPool) {
        let shop = shop(pool).await;
        let bad = shop.app.post(&format!("/v1/checkout/{}", shop.course)).jar(&shop.buyer)
            .raw_body(b"not json".to_vec(), Some("application/json")).send().await;
        assert_eq!((bad.status, code_of(&bad)), (StatusCode::BAD_REQUEST, "invalid_body"));
        for body in [json!({"coupon": ""}), json!({"coupon": "   "}), json!({}), json!({"coupon": null})] {
            let res = shop.app.post(&format!("/v1/checkout/{}", shop.course)).jar(&shop.buyer).json(body.clone()).send().await;
            assert_eq!(res.status, StatusCode::OK, "{body}: {:?}", res.body);
            assert_eq!(amount(&res), PRICE);
        }
    }
}

db_test! {
    async fn guessing_codes_is_slow(pool: PgPool) {
        let shop = shop(pool).await;
        let mut refused = 0;
        let mut limited = 0;
        for i in 0..24 {
            let res = preview(&shop, &shop.buyer, &format!("GUESS{i}")).await;
            match res.status {
                StatusCode::UNPROCESSABLE_ENTITY => refused += 1,
                StatusCode::TOO_MANY_REQUESTS => limited += 1,
                other => panic!("{other}"),
            }
        }
        assert_eq!((refused, limited), (20, 4));
    }
}

// ---- limits that hold under pressure --------------------------------------------------------------

db_test! {
    async fn a_hundred_buyers_and_ten_redemptions_make_exactly_ten_orders(pool: PgPool) {
        let shop = shop(pool).await;
        make(&shop, json!({"code": "FLASH", "kind": "percent", "value": 30, "max_redemptions": 10})).await;
        sqlx::query(
            "insert into users (name, email, password_hash, role) \
             select 'Buyer ' || g, 'flash' || g || '@coupons.in', 'x', 'student' from generate_series(1, 100) g",
        )
        .execute(&shop.pool)
        .await
        .unwrap();
        let ids: Vec<Uuid> = sqlx::query_scalar("select id from users where email like 'flash%@coupons.in' order by email")
            .fetch_all(&shop.pool).await.unwrap();
        assert_eq!(ids.len(), 100);

        let results = join_all(ids.iter().map(|id| {
            let user = AuthUser { id: *id, role: Role::Student, session_id: Uuid::new_v4() };
            let state = shop.app.state.clone();
            let course = shop.course;
            async move { service::checkout_with(&state, &user, course, None, Some("FLASH")).await }
        })).await;

        let won = results.iter().filter(|r| r.is_ok()).count();
        let exhausted = results.iter().filter(|r| matches!(r, Err(zeno_api::error::ApiError::Http { code: "coupon_exhausted", .. }))).count();
        assert_eq!((won, exhausted), (10, 90), "every loser is told the coupon ran out");
        assert_eq!(used(&shop, "FLASH").await, 10);
        assert_eq!(n(&shop, "select count(*) from coupon_redemptions where released_at is null").await, 10);
        assert_eq!(n(&shop, "select count(*) from orders where price_snapshot->'coupon'->>'code' = 'FLASH'").await, 10);
        // Nobody who lost has an order, and the provider was asked for exactly the winners'.
        assert_eq!(shop.provider.orders().len(), 1 + 10);
    }
}

db_test! {
    async fn a_per_person_limit_holds_across_orders(pool: PgPool) {
        let shop = shop(pool).await;
        make(&shop, percent("ONCE", 20)).await;
        let first = checkout_with(&shop, &shop.buyer, Some("ONCE")).await;
        assert_eq!(first.status, StatusCode::CREATED);
        // The order lapses and is cleared; the redemption is given back, so they may use it again.
        sqlx::query("update orders set created_at = now() - interval '40 minutes', expires_at = now() - interval '10 minutes'")
            .execute(&shop.pool).await.unwrap();
        assert_eq!(checkout_with(&shop, &shop.buyer, Some("ONCE")).await.status, StatusCode::CREATED);
        assert_eq!(used(&shop, "ONCE").await, 1);

        // A paid order keeps its redemption: once paid, the coupon is theirs, and a refund does not
        // hand it back (or the offer could be farmed).
        let paid = checkout_with(&shop, &shop.buyer, None).await;
        let remote = paid.body["checkout"]["provider_order_id"].as_str().unwrap().to_owned();
        let payment = shop.provider.capture(&remote).unwrap();
        deliver(&shop, "evt_paid", captured(&payment.id, &remote, amount(&paid))).await;
        let refunded = shop.app.post(&format!("/v1/orders/{}/refund", paid.body["order"]["id"].as_str().unwrap()))
            .jar(&shop.creator).json(json!({})).send().await;
        assert_eq!(refunded.status, StatusCode::ACCEPTED, "{:?}", refunded.body);
        assert_eq!(used(&shop, "ONCE").await, 1);
        let again = checkout_with(&shop, &shop.buyer, Some("ONCE")).await;
        assert_eq!(code_of(&again), "coupon_limit_reached");
    }
}

// ---- giving it back --------------------------------------------------------------------------------------

db_test! {
    async fn an_order_that_expires_unpaid_gives_its_coupon_back(pool: PgPool) {
        let shop = shop(pool).await;
        make(&shop, json!({"code": "SCARCE", "kind": "percent", "value": 40, "max_redemptions": 1})).await;
        assert_eq!(checkout_with(&shop, &shop.buyer, Some("SCARCE")).await.status, StatusCode::CREATED);
        shop.app.settle().await;
        let other = another_buyer(&shop, "second@coupons.in").await;
        assert_eq!(code_of(&checkout_with(&shop, &other, Some("SCARCE")).await), "coupon_exhausted");

        // Time passes with no payment; the reconciliation job expires the order.
        sqlx::query("update orders set created_at = now() - interval '40 minutes', expires_at = now() - interval '10 minutes'")
            .execute(&shop.pool).await.unwrap();
        let report = run_once(&shop.app.state).await.unwrap();
        assert_eq!(report.orders_expired, 1, "{report:?}");

        assert_eq!(used(&shop, "SCARCE").await, 0);
        assert_eq!(n(&shop, "select count(*) from coupon_redemptions where released_at is not null").await, 1);
        assert_eq!(checkout_with(&shop, &other, Some("SCARCE")).await.status, StatusCode::CREATED, "someone else can have it now");
    }
}

db_test! {
    async fn a_payment_that_turns_up_after_the_order_expired_takes_the_coupon_back(pool: PgPool) {
        let shop = shop(pool).await;
        make(&shop, json!({"code": "SCARCE", "kind": "percent", "value": 40, "max_redemptions": 1})).await;
        let started = checkout_with(&shop, &shop.buyer, Some("SCARCE")).await;
        let remote = started.body["checkout"]["provider_order_id"].as_str().unwrap().to_owned();
        let discounted = amount(&started);
        sqlx::query("update orders set created_at = now() - interval '40 minutes', expires_at = now() - interval '10 minutes'")
            .execute(&shop.pool).await.unwrap();
        run_once(&shop.app.state).await.unwrap();
        assert_eq!(used(&shop, "SCARCE").await, 0);

        // The customer pays, at the price they were shown, after the order was cleared away.
        let payment = shop.provider.capture(&remote).unwrap();
        let res = deliver(&shop, "evt_late", captured(&payment.id, &remote, discounted)).await;

        assert_eq!(outcome(&res), "enrolled", "{:?}", res.body);
        assert_eq!(used(&shop, "SCARCE").await, 1, "the discount was used, so the coupon is spent");
        assert_eq!(n(&shop, "select count(*) from order_events where kind = 'coupon_reclaimed'").await, 1);
        assert_eq!(access(&shop).await, "allowed");
    }
}

db_test! {
    async fn the_amount_checked_against_a_payment_is_the_discounted_one(pool: PgPool) {
        let shop = shop(pool).await;
        make(&shop, percent("HALF", 50)).await;
        let started = checkout_with(&shop, &shop.buyer, Some("HALF")).await;
        let remote = started.body["checkout"]["provider_order_id"].as_str().unwrap().to_owned();
        // Paying the full list price against the half-price order is not what was agreed.
        let res = deliver(&shop, "evt_1", captured("pay_1", &remote, PRICE)).await;
        assert_eq!(outcome(&res), "amount_mismatch");
        let ok = deliver(&shop, "evt_2", captured("pay_2", &remote, 24_950)).await;
        assert_eq!(outcome(&ok), "enrolled");
    }
}

db_test! {
    async fn refunding_a_discounted_order_returns_what_was_paid(pool: PgPool) {
        let shop = shop(pool).await;
        make(&shop, percent("HALF", 50)).await;
        let started = checkout_with(&shop, &shop.buyer, Some("HALF")).await;
        let remote = started.body["checkout"]["provider_order_id"].as_str().unwrap().to_owned();
        let order = started.body["order"]["id"].as_str().unwrap().to_owned();
        let payment = shop.provider.capture(&remote).unwrap();
        deliver(&shop, "evt_1", captured(&payment.id, &remote, 24_950)).await;

        let res = shop.app.post(&format!("/v1/orders/{order}/refund")).jar(&shop.creator).json(json!({})).send().await;

        assert_eq!(res.status, StatusCode::ACCEPTED, "{:?}", res.body);
        assert_eq!(res.body["refund"]["amount_paise"], 24_950, "the paid amount, not the list price");
        assert_eq!(shop.provider.refunds()[0].amount_paise, 24_950);
    }
}

// ---- coupons that make a course free ---------------------------------------------------------------------

db_test! {
    async fn a_coupon_that_covers_everything_enrolls_without_a_payment(pool: PgPool) {
        let shop = shop(pool).await;
        make(&shop, percent("FREEPASS", 100)).await;
        let orders_before = shop.provider.orders().len();
        let create_calls = shop.provider.calls(Op::CreateOrder);

        let res = checkout_with(&shop, &shop.buyer, Some("freepass")).await;

        assert_eq!(res.status, StatusCode::CREATED, "{:?}", res.body);
        assert_eq!(res.body["status"], "enrolled");
        assert_eq!(res.body["enrollment"]["source"], "free");
        assert_eq!(res.body["price"]["free"], true);
        assert_eq!(res.body["price"]["amount_paise"], 0);
        assert_eq!(res.body["price"]["discount_paise"], PRICE);
        assert_eq!(access(&shop).await, "allowed");
        // No provider call, no order, no payment: the open order from before is simply cleared.
        assert_eq!(shop.provider.orders().len(), orders_before);
        assert_eq!(shop.provider.calls(Op::CreateOrder), create_calls);
        assert_eq!(n(&shop, "select count(*) from payments").await, 0);
        assert_eq!(n(&shop, "select count(*) from orders where status in ('created','pending')").await, 0);
        assert_eq!(used(&shop, "FREEPASS").await, 1);
        assert_eq!(n(&shop, "select count(*) from coupon_redemptions where enrollment_id is not null and order_id is null").await, 1);
        assert_eq!(n(&shop, "select count(*) from enrollment_events where source = 'free' and reason like '%FREEPASS%'").await, 1);

        // Once they have it, they have it.
        let again = checkout_with(&shop, &shop.buyer, Some("FREEPASS")).await;
        assert_eq!((again.status, code_of(&again)), (StatusCode::CONFLICT, "already_enrolled"));
        assert_eq!(used(&shop, "FREEPASS").await, 1);
    }
}

db_test! {
    async fn a_flat_coupon_worth_the_whole_price_is_free_too(pool: PgPool) {
        let shop = shop(pool).await;
        make(&shop, flat("COVERED", 100_000)).await;
        let res = checkout_with(&shop, &shop.buyer, Some("COVERED")).await;
        assert_eq!((res.status, res.body["status"].as_str()), (StatusCode::CREATED, Some("enrolled")));
        assert_eq!(res.body["price"]["discount_paise"], PRICE, "the discount is the price, not the coupon's face value");
    }
}

db_test! {
    async fn a_free_coupon_cannot_buy_back_access_a_creator_took_away_but_can_after_a_refund(pool: PgPool) {
        let shop = shop(pool).await;
        make(&shop, percent("FREEPASS", 100)).await;
        sqlx::query(
            "insert into enrollments (user_id, course_id, source, granted_by, revoked_at, \
                revoke_reason, revoked_by, revocation_kind) \
             values ($1,$2,'grant',$3, now(), 'misuse', $3, 'creator')",
        )
        .bind(shop.buyer_id).bind(shop.course).bind(shop.creator_id)
        .execute(&shop.pool).await.unwrap();
        let blocked = checkout_with(&shop, &shop.buyer, Some("FREEPASS")).await;
        assert_eq!((blocked.status, code_of(&blocked)), (StatusCode::CONFLICT, "access_revoked_by_creator"));
        assert_eq!(used(&shop, "FREEPASS").await, 0, "a refused enrollment does not use the coupon");

        sqlx::query("update enrollments set revocation_kind = 'payment'").execute(&shop.pool).await.unwrap();
        let back = checkout_with(&shop, &shop.buyer, Some("FREEPASS")).await;
        assert_eq!((back.status, back.body["status"].as_str()), (StatusCode::CREATED, Some("enrolled")), "{:?}", back.body);
        assert_eq!(access(&shop).await, "allowed");
        assert_eq!(n(&shop, "select count(*) from enrollments").await, 1);
        assert_eq!(n(&shop, "select count(*) from enrollment_events where action = 'reactivated'").await, 1);
    }
}

db_test! {
    async fn a_free_coupon_with_a_limit_of_one_person_one_use_is_safe_to_race(pool: PgPool) {
        let shop = shop(pool).await;
        make(&shop, json!({"code": "ONLY1", "kind": "percent", "value": 100, "max_redemptions": 1})).await;
        sqlx::query(
            "insert into users (name, email, password_hash, role) \
             select 'Racer ' || g, 'racer' || g || '@coupons.in', 'x', 'student' from generate_series(1, 12) g",
        ).execute(&shop.pool).await.unwrap();
        let ids: Vec<Uuid> = sqlx::query_scalar("select id from users where email like 'racer%@coupons.in'")
            .fetch_all(&shop.pool).await.unwrap();
        let results = join_all(ids.iter().map(|id| {
            let user = AuthUser { id: *id, role: Role::Student, session_id: Uuid::new_v4() };
            let state = shop.app.state.clone();
            let course = shop.course;
            async move { service::checkout_with(&state, &user, course, None, Some("ONLY1")).await }
        })).await;
        let enrolled = results.iter().filter(|r| matches!(r, Ok(service::Outcome::Enrolled(_)))).count();
        assert_eq!(enrolled, 1, "{}", results.iter().filter_map(|r| r.as_ref().err()).count());
        assert_eq!(n(&shop, "select count(*) from enrollments where source = 'free'").await, 1);
        assert_eq!(used(&shop, "ONLY1").await, 1);
    }
}

// ---- replays ------------------------------------------------------------------------------------------------

db_test! {
    async fn a_replayed_request_with_the_same_key_returns_the_discounted_order(pool: PgPool) {
        let shop = shop(pool).await;
        make(&shop, percent("HALF", 50)).await;
        let send = || shop.app.post(&format!("/v1/checkout/{}", shop.course)).jar(&shop.buyer)
            .header("idempotency-key", "click-1").json(json!({"coupon": "HALF"})).send();
        let first = send().await;
        let second = send().await;
        assert_eq!((first.status, second.status), (StatusCode::CREATED, StatusCode::OK), "{:?}", second.body);
        assert_eq!(first.body["order"]["id"], second.body["order"]["id"]);
        assert_eq!(amount(&second), 24_950);
        assert_eq!(used(&shop, "HALF").await, 1);
    }
}
