//! Invoices: off by default, numbered without gaps when on, immutable once issued, and a plain
//! PDF for the buyer.

mod common;

use std::sync::Arc;

use axum::http::{header, Method, StatusCode};
use chrono::Utc;
use common::shop::*;
use common::*;
use futures_util::future::join_all;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;
use zeno_api::auth::extractor::AuthUser;
use zeno_api::invoices::fy::financial_year;
use zeno_api::orders::service;
use zeno_core::model::Role;

macro_rules! db_test {
    ($(#[$meta:meta])* async fn $name:ident($pool:ident: PgPool) $body:block) => {
        #[sqlx::test(migrator = "zeno_core::db::MIGRATOR")]
        $(#[$meta])*
        async fn $name($pool: PgPool) $body
    };
}

fn number(sequence: u32) -> String {
    format!("ZNO/{}/{sequence:06}", financial_year(Utc::now()))
}

async fn pay_for_real(shop: &Shop, event: &str) -> String {
    let payment = shop.provider.capture(&shop.remote).unwrap();
    let res = deliver(shop, event, captured(&payment.id, &shop.remote, PRICE)).await;
    assert_eq!(outcome(&res), "enrolled", "{:?}", res.body);
    payment.id
}

async fn invoice_json(shop: &Shop, who: &Jar) -> Res {
    shop.app
        .get(&format!("/v1/orders/{}/invoice", shop.order))
        .jar(who)
        .send()
        .await
}

async fn invoice_pdf(shop: &Shop, who: &Jar) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    // The test harness parses JSON, so fetch the raw bytes by calling the router directly.
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let request = Request::builder()
        .method(Method::GET)
        .uri(format!("/v1/orders/{}/invoice.pdf", shop.order))
        .header(header::COOKIE, who.header_for("/v1/").unwrap_or_default())
        .body(Body::empty())
        .unwrap();
    let response = shop.app.router.clone().oneshot(request).await.unwrap();
    let (parts, body) = response.into_parts();
    (
        parts.status,
        parts.headers,
        body.collect().await.unwrap().to_bytes().to_vec(),
    )
}

// ---- off by default -------------------------------------------------------------------------

db_test! {
    async fn invoices_are_off_unless_switched_on_and_payments_do_not_care(pool: PgPool) {
        let shop = shop(pool).await;
        shop.app.settle().await;
        let mails = shop.app.mailer.sent().len();
        pay_for_real(&shop, "evt_1").await;

        // The plain receipt still goes out, without an invoice number.
        let receipt = shop.app.mailer.wait_for(mails + 1).await.pop().unwrap();
        assert!(receipt.subject.contains("Payment received"));
        assert!(!receipt.text.contains("Invoice"), "{}", receipt.text);
        assert_eq!(n(&shop, "select count(*) from invoices").await, 0);
        assert_eq!(n(&shop, "select count(*) from invoice_counters").await, 0);

        let res = invoice_json(&shop, &shop.buyer).await;
        assert_eq!((res.status, res.code()), (StatusCode::CONFLICT, "invoices_unavailable"));
        let (status, _, body) = invoice_pdf(&shop, &shop.buyer).await;
        assert_eq!(status, StatusCode::CONFLICT);
        let error: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(error["error"]["code"], "invoices_unavailable");
        // To anyone else the order simply is not there, flag or no flag.
        shop.app.settle().await;
        let other = signup_verified_as(&shop.app, "other@invoices.in", "student").await;
        assert_eq!(invoice_json(&shop, &other).await.status, StatusCode::NOT_FOUND);
        assert_eq!(invoice_json(&shop, &shop.creator).await.status, StatusCode::NOT_FOUND);
    }
}

// ---- issuing ----------------------------------------------------------------------------------

db_test! {
    async fn a_payment_issues_the_invoice_in_the_same_transaction_and_the_receipt_quotes_it(pool: PgPool) {
        let shop = invoicing_shop(pool).await;
        shop.app.settle().await;
        let mails = shop.app.mailer.sent().len();
        let payment = pay_for_real(&shop, "evt_1").await;

        let (invoice_number, order, amount): (String, Uuid, i64) =
            sqlx::query_as("select number, order_id, amount_paise from invoices")
                .fetch_one(&shop.pool)
                .await
                .unwrap();
        assert_eq!(invoice_number, number(1));
        assert_eq!((order, amount), (shop.order, PRICE));
        let receipt = shop.app.mailer.wait_for(mails + 1).await.pop().unwrap();
        assert!(receipt.text.contains(&format!("Invoice: {}", number(1))), "{}", receipt.text);

        let res = invoice_json(&shop, &shop.buyer).await;
        assert_eq!(res.status, StatusCode::OK, "{:?}", res.body);
        let invoice = &res.body["invoice"];
        assert_eq!(invoice["number"], number(1));
        assert_eq!(invoice["amount_paise"], PRICE);
        let data = &invoice["invoice"];
        assert_eq!(data["seller"]["name"], "Zeno Learning Pvt Ltd");
        assert_eq!(data["buyer"]["name"], "Asha Rao");
        assert_eq!(data["buyer"]["email"], "buyer@hooks.in");
        assert_eq!(data["payment_ref"], payment);
        assert_eq!(data["total_paise"], PRICE);
        assert!(data["tax"].is_null(), "no tax details until the tax side has been advised on");
        let line = &data["lines"][0];
        assert_eq!((line["description"].as_str(), line["amount_paise"].as_i64()), (Some("Paid course"), Some(PRICE)));
        assert_eq!(line["discount_paise"], 0);

        // Delivering the payment again spends no number.
        deliver(&shop, "evt_2", captured(&payment, &shop.remote, PRICE)).await;
        assert_eq!(n(&shop, "select count(*) from invoices").await, 1);
        assert_eq!(n(&shop, "select last_sequence::bigint from invoice_counters").await, 1);
    }
}

db_test! {
    async fn the_invoice_shows_the_discount_that_was_given(pool: PgPool) {
        let shop = invoicing_shop(pool).await;
        let coupon = shop.app.post("/v1/coupons").jar(&shop.creator)
            .json(json!({"code": "LAUNCH10", "kind": "percent", "value": 10})).send().await;
        assert_eq!(coupon.status, StatusCode::CREATED, "{:?}", coupon.body);
        let started = shop.app.post(&format!("/v1/checkout/{}", shop.course)).jar(&shop.buyer)
            .json(json!({"coupon": "launch10"})).send().await;
        let order = started.body["order"]["id"].as_str().unwrap().to_owned();
        let remote = started.body["checkout"]["provider_order_id"].as_str().unwrap().to_owned();
        let payment = shop.provider.capture(&remote).unwrap();
        deliver(&shop, "evt_1", captured(&payment.id, &remote, 44_910)).await;

        let res = shop.app.get(&format!("/v1/orders/{order}/invoice")).jar(&shop.buyer).send().await;
        assert_eq!(res.status, StatusCode::OK, "{:?}", res.body);
        let line = &res.body["invoice"]["invoice"]["lines"][0];
        assert_eq!(line["list_price_paise"], PRICE);
        assert_eq!(line["discount_paise"], 4_990);
        assert_eq!(line["coupon_code"], "LAUNCH10");
        assert_eq!(line["amount_paise"], 44_910);
        assert_eq!(res.body["invoice"]["amount_paise"], 44_910);
    }
}

db_test! {
    async fn the_pdf_is_a_private_download_that_names_the_invoice(pool: PgPool) {
        let shop = invoicing_shop(pool).await;
        pay_for_real(&shop, "evt_1").await;

        let (status, headers, bytes) = invoice_pdf(&shop, &shop.buyer).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers[header::CONTENT_TYPE], "application/pdf");
        assert_eq!(headers[header::CACHE_CONTROL], "private, no-store");
        assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        let disposition = headers[header::CONTENT_DISPOSITION].to_str().unwrap();
        assert_eq!(
            disposition,
            format!("attachment; filename=\"{}.pdf\"", number(1).replace('/', "-"))
        );
        assert!(bytes.starts_with(b"%PDF-1.4"));
        assert!(bytes.ends_with(b"%%EOF\n"));
        let text = String::from_utf8(bytes).unwrap();
        for expected in [number(1).as_str(), "Zeno Learning Pvt Ltd", "Asha Rao", "Paid course", "Rs 499.00"] {
            assert!(text.contains(expected), "missing {expected:?}");
        }

        // Nobody else gets it, and nobody signed out.
        shop.app.settle().await;
        let other = signup_verified_as(&shop.app, "other@invoices.in", "student").await;
        assert_eq!(invoice_pdf(&shop, &other).await.0, StatusCode::NOT_FOUND);
        assert_eq!(invoice_pdf(&shop, &Jar::default()).await.0, StatusCode::UNAUTHORIZED);
    }
}

db_test! {
    async fn awkward_names_still_make_a_valid_invoice_and_are_kept_exactly_in_the_data(pool: PgPool) {
        let shop = invoicing_shop(pool).await;
        let name = "Asha (Rao) \\ Ünï \u{0939}\u{093f}\u{0902}\u{0926}\u{0940} \u{1F642}";
        sqlx::query("update users set name = $1 where email = 'buyer@hooks.in'")
            .bind(name).execute(&shop.pool).await.unwrap();
        pay_for_real(&shop, "evt_1").await;

        let json = invoice_json(&shop, &shop.buyer).await;
        assert_eq!(json.body["invoice"]["invoice"]["buyer"]["name"], name, "the data is exact");
        // A PDF that replaces part of the buyer's legal name with '?' is misleading, so none is
        // made; the JSON invoice above is the exact record.
        let (status, _, body) = invoice_pdf(&shop, &shop.buyer).await;
        assert_eq!(status, StatusCode::CONFLICT);
        let error: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(error["error"]["code"], "invoice_pdf_unsupported_text");
    }
}

db_test! {
    async fn an_unpaid_or_missing_order_has_no_invoice(pool: PgPool) {
        let shop = invoicing_shop(pool).await;
        let res = invoice_json(&shop, &shop.buyer).await;
        assert_eq!((res.status, res.code()), (StatusCode::CONFLICT, "not_invoiceable"));
        assert_eq!(n(&shop, "select count(*) from invoice_counters").await, 0, "no number was spent");
        let missing = shop.app.get(&format!("/v1/orders/{}/invoice", Uuid::new_v4())).jar(&shop.buyer).send().await;
        assert_eq!(missing.status, StatusCode::NOT_FOUND);
        let garbage = shop.app.get("/v1/orders/not-a-uuid/invoice.pdf").jar(&shop.buyer).send().await;
        assert_eq!(garbage.status, StatusCode::NOT_FOUND);
    }
}

db_test! {
    async fn an_order_paid_before_invoices_were_switched_on_is_invoiced_when_first_asked_for(pool: PgPool) {
        // Paid with invoices off...
        let off = shop(pool.clone()).await;
        let payment = off.provider.capture(&off.remote).unwrap();
        deliver(&off, "evt_1", captured(&payment.id, &off.remote, PRICE)).await;
        assert_eq!(n(&off, "select count(*) from invoices").await, 0);

        // ...then the same database, served by an instance with them on.
        let provider = off.provider.clone();
        let mailer = zeno_api::mail::MemoryMailer::default();
        let state = make_state_with(pool.clone(), false, 0, Arc::new(mailer.clone()), |config| {
            config.invoices = zeno_api::invoices::InvoicesConfig {
                enabled: true,
                seller_name: "Zeno Learning Pvt Ltd".into(),
                seller_address: None,
                prefix: "ZNO".into(),
            };
        })
        .with_payments(Arc::new(provider));
        let on = App { router: zeno_api::build_app(state.clone()), mailer, pool: pool.clone(), state };

        let first = on.get(&format!("/v1/orders/{}/invoice", off.order)).jar(&off.buyer).send().await;
        assert_eq!(first.status, StatusCode::OK, "{:?}", first.body);
        assert_eq!(first.body["invoice"]["number"], number(1));
        let again = on.get(&format!("/v1/orders/{}/invoice", off.order)).jar(&off.buyer).send().await;
        assert_eq!(again.body["invoice"]["number"], number(1), "asking twice does not spend a second number");
        assert_eq!(n(&off, "select count(*) from invoices").await, 1);
    }
}

db_test! {
    async fn eight_requests_for_the_same_uninvoiced_order_spend_one_number(pool: PgPool) {
        let off = shop(pool.clone()).await;
        let payment = off.provider.capture(&off.remote).unwrap();
        deliver(&off, "evt_1", captured(&payment.id, &off.remote, PRICE)).await;

        let provider = off.provider.clone();
        let mailer = zeno_api::mail::MemoryMailer::default();
        let state = make_state_with(pool.clone(), false, 0, Arc::new(mailer.clone()), |config| {
            config.invoices = zeno_api::invoices::InvoicesConfig {
                enabled: true,
                seller_name: "Zeno Learning Pvt Ltd".into(),
                seller_address: None,
                prefix: "ZNO".into(),
            };
        })
        .with_payments(Arc::new(provider));
        let on = App { router: zeno_api::build_app(state.clone()), mailer, pool: pool.clone(), state };

        let results = join_all((0..8).map(|_| {
            on.get(&format!("/v1/orders/{}/invoice", off.order)).jar(&off.buyer).send()
        }))
        .await;

        assert!(
            results.iter().all(|r| r.status == StatusCode::OK),
            "{:?}",
            results.iter().map(|r| (r.status, r.body.to_string())).collect::<Vec<_>>()
        );
        assert!(results.iter().all(|r| r.body["invoice"]["number"] == number(1).as_str()));
        assert_eq!(n(&off, "select count(*) from invoices").await, 1);
        assert_eq!(n(&off, "select last_sequence::bigint from invoice_counters").await, 1);
    }
}

db_test! {
    async fn a_refunded_order_keeps_its_invoice(pool: PgPool) {
        let shop = invoicing_shop(pool).await;
        pay_for_real(&shop, "evt_1").await;
        shop.app.settle().await;
        let refund = shop.app.post(&format!("/v1/orders/{}/refund", shop.order)).jar(&shop.creator).json(json!({})).send().await;
        assert_eq!(refund.status, StatusCode::ACCEPTED, "{:?}", refund.body);
        assert_eq!(order_status(&shop).await, "refunded");
        // An invoice is never withdrawn; a correction would be a new document (not built).
        let res = invoice_json(&shop, &shop.buyer).await;
        assert_eq!(res.status, StatusCode::OK, "{:?}", res.body);
        assert_eq!(res.body["invoice"]["number"], number(1));
    }
}

// ---- numbering -----------------------------------------------------------------------------------

db_test! {
    async fn payments_that_are_not_sales_do_not_take_numbers(pool: PgPool) {
        let shop = invoicing_shop(pool).await;
        // A failed attempt, a mismatched amount, a stranger's order: no invoice.
        deliver(&shop, "evt_f", failed("pay_f", &shop.remote, PRICE)).await;
        deliver(&shop, "evt_m", captured("pay_m", &shop.remote, 100)).await;
        deliver(&shop, "evt_u", captured("pay_u", "order_unknown", PRICE)).await;
        assert_eq!(n(&shop, "select count(*) from invoices").await, 0);
        assert_eq!(n(&shop, "select count(*) from invoice_counters").await, 0);
        // The real payment is number one; a second payment for the same order is a duplicate,
        // flagged, and not invoiced.
        let payment = pay_for_real(&shop, "evt_1").await;
        let dup = deliver(&shop, "evt_2", captured("pay_dup", &shop.remote, PRICE)).await;
        assert_eq!(outcome(&dup), "duplicate_payment");
        let _ = payment;
        assert_eq!(n(&shop, "select count(*) from invoices").await, 1);
        assert_eq!(n(&shop, "select last_sequence::bigint from invoice_counters").await, 1);
    }
}

db_test! {
    async fn a_failure_after_the_number_is_taken_gives_the_number_back(pool: PgPool) {
        let shop = invoicing_shop(pool).await;
        // The receipt is queued after the invoice is written; make that fail.
        sqlx::query("create function fail_mail() returns trigger language plpgsql as $$ begin raise exception 'simulated'; end $$")
            .execute(&shop.pool).await.unwrap();
        sqlx::query("create trigger fail_mail before insert on email_outbox for each row execute function fail_mail()")
            .execute(&shop.pool).await.unwrap();
        let payment = shop.provider.capture(&shop.remote).unwrap();
        let failed_delivery = deliver(&shop, "evt_1", captured(&payment.id, &shop.remote, PRICE)).await;
        assert_eq!(failed_delivery.status, StatusCode::INTERNAL_SERVER_ERROR);
        // Nothing was kept: no invoice, and the counter did not move.
        assert_eq!(n(&shop, "select count(*) from invoices").await, 0);
        assert_eq!(n(&shop, "select count(*) from invoice_counters").await, 0);

        sqlx::query("drop trigger fail_mail on email_outbox").execute(&shop.pool).await.unwrap();
        let retry = deliver(&shop, "evt_1", captured(&payment.id, &shop.remote, PRICE)).await;
        assert_eq!(outcome(&retry), "enrolled");
        let issued: String = sqlx::query_scalar("select number from invoices").fetch_one(&shop.pool).await.unwrap();
        assert_eq!(issued, number(1), "the number that was rolled back is the one that is used");
    }
}

db_test! {
    async fn ten_simultaneous_sales_get_ten_numbers_with_no_gap_and_no_repeat(pool: PgPool) {
        let shop = invoicing_shop(pool).await;
        sqlx::query(
            "insert into users (name, email, password_hash, role) \
             select 'Buyer ' || g, 'sale' || g || '@invoices.in', 'x', 'student' from generate_series(1, 10) g",
        ).execute(&shop.pool).await.unwrap();
        let ids: Vec<Uuid> = sqlx::query_scalar("select id from users where email like 'sale%@invoices.in' order by email")
            .fetch_all(&shop.pool).await.unwrap();

        // Ten orders, each paid at the provider.
        let mut bodies = Vec::new();
        for (i, id) in ids.iter().enumerate() {
            let user = AuthUser { id: *id, role: Role::Student, session_id: Uuid::new_v4() };
            let order = service::checkout(&shop.app.state, &user, shop.course, None).await.unwrap().order;
            let remote = order.provider_order_id.clone().unwrap();
            let payment = shop.provider.capture(&remote).unwrap();
            bodies.push((format!("evt_sale_{i}"), captured(&payment.id, &remote, PRICE)));
        }
        let results = join_all(bodies.into_iter().map(|(id, body)| {
            let shop = &shop;
            async move { deliver(shop, &id, body).await }
        })).await;
        assert!(results.iter().all(|r| r.status == StatusCode::OK), "{:?}", results.iter().map(|r| r.status).collect::<Vec<_>>());

        let mut sequences: Vec<i32> = sqlx::query_scalar("select sequence from invoices order by sequence")
            .fetch_all(&shop.pool).await.unwrap();
        assert_eq!(sequences, (1..=10).collect::<Vec<_>>(), "no gap, no repeat");
        sequences.dedup();
        assert_eq!(sequences.len(), 10);
        assert_eq!(n(&shop, "select count(distinct number) from invoices").await, 10);
        assert_eq!(n(&shop, "select last_sequence::bigint from invoice_counters").await, 10);
    }
}

db_test! {
    async fn each_financial_year_counts_from_one(pool: PgPool) {
        let shop = invoicing_shop(pool).await;
        let mut conn = shop.pool.acquire().await.unwrap();
        use zeno_api::invoices::repo::next_sequence;
        assert_eq!(next_sequence(&mut conn, "2026-27").await.unwrap(), 1);
        assert_eq!(next_sequence(&mut conn, "2026-27").await.unwrap(), 2);
        assert_eq!(next_sequence(&mut conn, "2027-28").await.unwrap(), 1, "a new year starts again");
        assert_eq!(next_sequence(&mut conn, "2026-27").await.unwrap(), 3, "and the old one carries on");
    }
}

// ---- immutability ------------------------------------------------------------------------------------

db_test! {
    async fn an_issued_invoice_cannot_be_changed_or_removed(pool: PgPool) {
        let shop = invoicing_shop(pool).await;
        pay_for_real(&shop, "evt_1").await;
        for statement in [
            "update invoices set amount_paise = 1",
            "update invoices set data = '{}'::jsonb",
            "update invoices set number = 'FORGED/1'",
            "delete from invoices",
            "truncate invoices",
        ] {
            let error = sqlx::query(statement).execute(&shop.pool).await.unwrap_err().to_string();
            assert!(error.contains("cannot be changed or removed"), "{statement}: {error}");
        }
        assert_eq!(n(&shop, "select count(*) from invoices").await, 1);
        let kept: Value = sqlx::query_scalar("select data from invoices").fetch_one(&shop.pool).await.unwrap();
        assert_eq!(kept["number"], number(1));
    }
}

db_test! {
    async fn an_invoice_shows_what_was_issued_even_if_the_course_is_renamed_later(pool: PgPool) {
        let shop = invoicing_shop(pool).await;
        pay_for_real(&shop, "evt_1").await;
        sqlx::query("update courses set title = 'Renamed later', price_paise = 1000 where id = $1")
            .bind(shop.course).execute(&shop.pool).await.unwrap();
        sqlx::query("update users set name = 'Someone Else' where email = 'buyer@hooks.in'")
            .execute(&shop.pool).await.unwrap();
        let res = invoice_json(&shop, &shop.buyer).await;
        let data = &res.body["invoice"]["invoice"];
        assert_eq!(data["lines"][0]["description"], "Paid course");
        assert_eq!(data["buyer"]["name"], "Asha Rao");
        assert_eq!(data["total_paise"], PRICE);
    }
}
