//! The earnings ledger: every sale and refund reaches the book in the transaction that makes it,
//! the book balances and cannot be edited, and what creators see adds up.

mod common;

use axum::http::{header, Method, StatusCode};
use common::shop::*;
use common::*;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde_json::{json, Value};
use sqlx::{Connection, PgPool};
use uuid::Uuid;
use zeno_api::ledger::fee::{self, Fee, FeeSource};
use zeno_api::ledger::integrity;
use zeno_api::ledger::post::{self, Sale};
use zeno_api::ledger::repo as ledger_repo;
use zeno_api::reconcile::run_once;

macro_rules! db_test {
    ($(#[$meta:meta])* async fn $name:ident($pool:ident: PgPool) $body:block) => {
        #[sqlx::test(migrator = "zeno_core::db::MIGRATOR")]
        $(#[$meta])*
        async fn $name($pool: PgPool) $body
    };
}

// ---- helpers --------------------------------------------------------------------------------------

async fn earnings(shop: &Shop) -> Value {
    let res = shop
        .app
        .get("/v1/me/earnings")
        .jar(&shop.creator)
        .send()
        .await;
    assert_eq!(res.status, StatusCode::OK, "{:?}", res.body);
    res.body["earnings"].clone()
}

async fn account(shop: &Shop, account: &str) -> i64 {
    n(
        shop,
        &format!("select coalesce(sum(amount_paise), 0)::bigint from ledger_entries where account = '{account}'"),
    )
    .await
}

async fn creator_total(shop: &Shop) -> i64 {
    account(shop, &format!("creator:{}", shop.creator_id)).await
}

async fn journal_kinds(shop: &Shop) -> Vec<String> {
    sqlx::query_scalar("select kind from ledger_journals order by kind, created_at")
        .fetch_all(&shop.pool)
        .await
        .unwrap()
}

/// The book as a whole adds up to nothing, and so does each journal.
async fn assert_book_balances(shop: &Shop) {
    assert_eq!(
        n(
            shop,
            "select coalesce(sum(amount_paise), 0)::bigint from ledger_entries"
        )
        .await,
        0
    );
    assert_eq!(
        n(shop, "select count(*) from (select journal_id from ledger_entries group by journal_id having sum(amount_paise) <> 0) x").await,
        0
    );
    assert_eq!(
        n(shop, "select count(*) from ledger_journals j where not exists (select 1 from ledger_entries e where e.journal_id = j.id)").await,
        0
    );
}

/// The customer pays; the provider says its fee was `fee`.
async fn pay_with_fee(shop: &Shop, event: &str, fee: Option<i64>) -> String {
    let payment = shop.provider.capture(&shop.remote).unwrap();
    let res = deliver(
        shop,
        event,
        captured_with_fee(&payment.id, &shop.remote, PRICE, fee),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{:?}", res.body);
    assert_eq!(outcome(&res), "enrolled", "{:?}", res.body);
    payment.id
}

/// A second buyer pays for the same course (and so the same creator).
async fn another_sale(shop: &Shop, email: &str, fee: Option<i64>) -> (Uuid, String) {
    shop.app.settle().await;
    let buyer = signup_verified_as(&shop.app, email, "student").await;
    let started = shop
        .app
        .post(&format!("/v1/checkout/{}", shop.course))
        .jar(&buyer)
        .send()
        .await;
    assert_eq!(started.status, StatusCode::CREATED, "{:?}", started.body);
    let order = Uuid::parse_str(started.body["order"]["id"].as_str().unwrap()).unwrap();
    let remote = started.body["checkout"]["provider_order_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let payment = shop.provider.capture(&remote).unwrap();
    let res = deliver(
        shop,
        &format!("evt_{email}"),
        captured_with_fee(&payment.id, &remote, PRICE, fee),
    )
    .await;
    assert_eq!(outcome(&res), "enrolled", "{:?}", res.body);
    (order, payment.id)
}

async fn age_sale(shop: &Shop, order: Uuid, days: i64) {
    sqlx::query("update orders set paid_at = now() - make_interval(days => $2::int) where id = $1")
        .bind(order)
        .bind(days as i32)
        .execute(&shop.pool)
        .await
        .unwrap();
}

/// Runs statements with the database's own safety triggers off, as an operator with a superuser
/// connection (or a corrupt restore) could. Only tests do this, to prove the checks notice.
async fn as_a_careless_operator(shop: &Shop, statements: &[&str]) {
    let mut conn = shop.pool.acquire().await.unwrap();
    sqlx::query("set session_replication_role = replica")
        .execute(&mut *conn)
        .await
        .unwrap();
    for statement in statements {
        sqlx::query(statement).execute(&mut *conn).await.unwrap();
    }
    sqlx::query("set session_replication_role = origin")
        .execute(&mut *conn)
        .await
        .unwrap();
}

// ---- a sale reaches the book -----------------------------------------------------------------------------

db_test! {
    async fn a_sale_credits_the_creator_in_full_and_charges_them_the_providers_fee(pool: PgPool) {
        let shop = shop(pool).await;
        let payment = pay_with_fee(&shop, "evt_1", Some(1_500)).await;

        assert_eq!(journal_kinds(&shop).await, ["gateway_fee", "sale"]);
        assert_eq!(creator_total(&shop).await, PRICE - 1_500);
        assert_eq!(account(&shop, "customers").await, -PRICE);
        assert_eq!(account(&shop, "gateway_fees").await, 1_500);
        assert_book_balances(&shop).await;
        // Both journals are tied to the order and keyed by the payment.
        assert_eq!(
            n(&shop, &format!("select count(*) from ledger_journals j join payments p on p.id = j.source_id where j.order_id = '{}' and p.provider_ref = '{payment}'", shop.order)).await,
            2
        );
        assert_eq!(n(&shop, "select fee_paise::bigint from payments").await, 1_500);

        let seen = earnings(&shop).await;
        assert_eq!(seen["balance_paise"], PRICE - 1_500);
        assert_eq!(seen["lifetime"]["sales_paise"], PRICE);
        assert_eq!(seen["lifetime"]["gateway_fees_paise"], 1_500);
        assert_eq!(seen["lifetime"]["refunds_paise"], 0);
        assert_eq!(seen["currency"], "INR");
    }
}

db_test! {
    async fn without_a_reported_fee_it_is_estimated_at_the_configured_rate(pool: PgPool) {
        let shop = shop(pool).await;
        pay_with_fee(&shop, "evt_1", None).await;
        // 2.36% of Rs 499.00 = 1177.64 paise, rounded to 1178.
        assert_eq!(account(&shop, "gateway_fees").await, 1_178);
        assert_eq!(creator_total(&shop).await, PRICE - 1_178);
        assert_eq!(n(&shop, "select count(*) from payments where fee_paise is null").await, 1);
        assert_book_balances(&shop).await;
    }
}

db_test! {
    async fn a_later_reported_fee_corrects_the_estimate_once(pool: PgPool) {
        let shop = shop(pool).await;
        let payment = pay_with_fee(&shop, "evt_1", None).await;
        assert_eq!(creator_total(&shop).await, PRICE - 1_178);

        let later = captured_with_fee(&payment, &shop.remote, PRICE, Some(1_500));
        assert_eq!(outcome(&deliver(&shop, "evt_fee", later.clone()).await), "already_applied");
        assert_eq!(creator_total(&shop).await, PRICE - 1_500);
        assert_eq!(account(&shop, "gateway_fees").await, 1_500);
        assert_eq!(n(&shop, "select fee_paise::bigint from payments").await, 1_500);
        assert_eq!(n(&shop, "select count(*) from ledger_journals where kind = 'gateway_fee_correction'").await, 1);
        assert_eq!(earnings(&shop).await["lifetime"]["gateway_fees_paise"], 1_500);
        assert!(integrity::check(&shop.pool).await.unwrap().is_empty());

        deliver(&shop, "evt_fee_replay", later).await;
        deliver(&shop, "evt_fee_conflict", captured_with_fee(&payment, &shop.remote, PRICE, Some(1_200))).await;
        assert_eq!(n(&shop, "select count(*) from ledger_journals where kind = 'gateway_fee_correction'").await, 1);
        assert_eq!(creator_total(&shop).await, PRICE - 1_500);
        assert_eq!(n(&shop, "select fee_paise::bigint from payments").await, 1_500);
        assert_book_balances(&shop).await;
    }
}

db_test! {
    async fn a_later_waived_fee_returns_the_whole_estimate(pool: PgPool) {
        let shop = shop(pool).await;
        let payment = pay_with_fee(&shop, "evt_1", None).await;
        deliver(&shop, "evt_zero_fee", captured_with_fee(&payment, &shop.remote, PRICE, Some(0))).await;
        assert_eq!(creator_total(&shop).await, PRICE);
        assert_eq!(account(&shop, "gateway_fees").await, 0);
        assert_eq!(n(&shop, "select fee_paise::bigint from payments").await, 0);
        assert!(integrity::check(&shop.pool).await.unwrap().is_empty());
    }
}

db_test! {
    async fn a_later_fee_after_a_full_refund_still_corrects_the_book(pool: PgPool) {
        let shop = shop(pool).await;
        let payment = pay_with_fee(&shop, "evt_1", None).await;
        deliver(&shop, "evt_refund", refund_processed("rfnd_fee_later", &payment, PRICE, None)).await;
        assert_eq!(creator_total(&shop).await, -1_178);

        let later = captured_with_fee(&payment, &shop.remote, PRICE, Some(1_500));
        assert_eq!(outcome(&deliver(&shop, "evt_fee_after_refund", later).await), "already_refunded");
        assert_eq!(creator_total(&shop).await, -1_500);
        assert_eq!(account(&shop, "gateway_fees").await, 1_500);
        assert_eq!(earnings(&shop).await["lifetime"]["gateway_fees_paise"], 1_500);
        assert!(integrity::check(&shop.pool).await.unwrap().is_empty());
        assert_book_balances(&shop).await;
    }
}

db_test! {
    async fn a_waived_fee_posts_no_fee_journal_and_an_impossible_one_is_estimated(pool: PgPool) {
        let shop = shop(pool).await;
        pay_with_fee(&shop, "evt_1", Some(0)).await;
        assert_eq!(journal_kinds(&shop).await, ["sale"], "a fee of nothing posts nothing");
        assert_eq!(creator_total(&shop).await, PRICE);

        let (order, _) = another_sale(&shop, "second@ledger.in", Some(PRICE + 1)).await;
        // More than the payment is not a fee; the estimate stands in.
        assert_eq!(n(&shop, &format!("select coalesce(-sum(e.amount_paise), 0)::bigint from ledger_entries e join ledger_journals j on j.id = e.journal_id where j.kind = 'gateway_fee' and j.order_id = '{order}' and e.creator_id is not null")).await, 1_178);
        assert_book_balances(&shop).await;
    }
}

db_test! {
    async fn the_same_payment_arriving_again_in_any_form_is_posted_once(pool: PgPool) {
        let shop = shop(pool).await;
        let payment = pay_with_fee(&shop, "evt_1", Some(1_500)).await;
        // The same event, the same payment under another event id, and order.paid for it.
        deliver(&shop, "evt_1", captured_with_fee(&payment, &shop.remote, PRICE, Some(1_500))).await;
        deliver(&shop, "evt_2", captured_with_fee(&payment, &shop.remote, PRICE, Some(1_500))).await;
        deliver(&shop, "evt_3", order_paid(&payment, &shop.remote, PRICE)).await;
        assert_eq!(journal_kinds(&shop).await, ["gateway_fee", "sale"]);
        assert_eq!(creator_total(&shop).await, PRICE - 1_500);
    }
}

db_test! {
    async fn reconciliation_posts_the_sale_with_the_fee_the_provider_reports(pool: PgPool) {
        let shop = shop(pool).await;
        shop.provider.set_fee_bps(Some(200)); // 2.00%
        let payment = shop.provider.capture(&shop.remote).unwrap();
        assert_eq!(payment.fee_paise, Some(998));
        sqlx::query("update orders set created_at = now() - interval '20 minutes', expires_at = now() + interval '10 minutes'")
            .execute(&shop.pool).await.unwrap();

        let report = run_once(&shop.app.state).await.unwrap();

        assert_eq!(report.orders_settled, 1, "{report:?}");
        assert_eq!(account(&shop, "gateway_fees").await, 998);
        assert_eq!(creator_total(&shop).await, PRICE - 998);
        // The webhook turning up afterwards changes nothing.
        deliver(&shop, "evt_late", captured_with_fee(&payment.id, &shop.remote, PRICE, Some(998))).await;
        assert_eq!(journal_kinds(&shop).await, ["gateway_fee", "sale"]);
        assert_book_balances(&shop).await;
    }
}

db_test! {
    async fn the_sale_and_the_payment_stand_or_fall_together(pool: PgPool) {
        let shop = shop(pool).await;
        // Make the ledger refuse every entry: the payment, the order change and the enrollment,
        // which all came first in the same transaction, must go with it.
        sqlx::query("create function refuse_entries() returns trigger language plpgsql as $$ begin raise exception 'simulated ledger failure'; end $$")
            .execute(&shop.pool).await.unwrap();
        sqlx::query("create trigger refuse_entries before insert on ledger_entries for each row execute function refuse_entries()")
            .execute(&shop.pool).await.unwrap();
        let payment = shop.provider.capture(&shop.remote).unwrap();
        let body = captured_with_fee(&payment.id, &shop.remote, PRICE, Some(1_500));

        let failed = deliver(&shop, "evt_1", body.clone()).await;

        assert_eq!(failed.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(order_status(&shop).await, "pending");
        assert_eq!(n(&shop, "select count(*) from payments").await, 0);
        assert_eq!(n(&shop, "select count(*) from enrollments").await, 0);
        assert_eq!(n(&shop, "select count(*) from ledger_journals").await, 0);
        assert_eq!(n(&shop, "select count(*) from webhook_events").await, 0);

        sqlx::query("drop trigger refuse_entries on ledger_entries").execute(&shop.pool).await.unwrap();
        let retried = deliver(&shop, "evt_1", body).await;
        assert_eq!(outcome(&retried), "enrolled");
        assert_eq!(journal_kinds(&shop).await, ["gateway_fee", "sale"]);
        assert_eq!(creator_total(&shop).await, PRICE - 1_500);
    }
}

db_test! {
    async fn a_discounted_sale_credits_what_was_actually_paid(pool: PgPool) {
        let shop = shop(pool).await;
        let coupon = shop.app.post("/v1/coupons").jar(&shop.creator)
            .json(json!({"code": "HALF", "kind": "percent", "value": 50})).send().await;
        assert_eq!(coupon.status, StatusCode::CREATED, "{:?}", coupon.body);
        let started = shop.app.post(&format!("/v1/checkout/{}", shop.course)).jar(&shop.buyer)
            .json(json!({"coupon": "HALF"})).send().await;
        let remote = started.body["checkout"]["provider_order_id"].as_str().unwrap().to_owned();
        let payment = shop.provider.capture(&remote).unwrap();
        deliver(&shop, "evt_1", captured_with_fee(&payment.id, &remote, 24_950, Some(600))).await;
        assert_eq!(creator_total(&shop).await, 24_950 - 600);
        assert_eq!(account(&shop, "customers").await, -24_950);
        let seen = earnings(&shop).await;
        assert_eq!(seen["lifetime"]["sales_paise"], 24_950, "the list price never reaches the book");
    }
}

db_test! {
    async fn payments_that_are_not_sales_credit_nobody(pool: PgPool) {
        let shop = shop(pool).await;
        // Wrong amount, an order we never made, a failed attempt.
        deliver(&shop, "evt_a", captured_with_fee("pay_a", &shop.remote, 100, Some(1))).await;
        deliver(&shop, "evt_b", captured_with_fee("pay_b", "order_unknown", PRICE, Some(1))).await;
        deliver(&shop, "evt_c", failed("pay_c", &shop.remote, PRICE)).await;
        assert_eq!(n(&shop, "select count(*) from ledger_journals").await, 0);

        // The real payment, then a second one for the same order: money we should not have
        // taken, held for a refund, not counted as the creator's.
        pay_with_fee(&shop, "evt_1", Some(1_500)).await;
        let dup = deliver(&shop, "evt_2", captured_with_fee("pay_dup", &shop.remote, PRICE, Some(1_500))).await;
        assert_eq!(outcome(&dup), "duplicate_payment");
        assert_eq!(n(&shop, "select count(*) from ledger_journals where kind = 'sale'").await, 1);
        assert_eq!(creator_total(&shop).await, PRICE - 1_500);
    }
}

db_test! {
    async fn money_taken_for_access_a_creator_blocked_is_still_the_creators(pool: PgPool) {
        let shop = shop(pool).await;
        sqlx::query(
            "insert into enrollments (user_id, course_id, source, granted_by, revoked_at, \
                revoke_reason, revoked_by, revocation_kind) \
             values ($1,$2,'grant',$3, now(), 'misuse', $3, 'creator')",
        )
        .bind(shop.buyer_id).bind(shop.course).bind(shop.creator_id)
        .execute(&shop.pool).await.unwrap();
        let payment = shop.provider.capture(&shop.remote).unwrap();
        let res = deliver(&shop, "evt_1", captured_with_fee(&payment.id, &shop.remote, PRICE, Some(1_500))).await;
        assert_eq!(outcome(&res), "paid_access_blocked");
        // The sale happened; a refund (flagged for review) will reverse it.
        assert_eq!(creator_total(&shop).await, PRICE - 1_500);
    }
}

// ---- refunds reach the book -----------------------------------------------------------------------------

db_test! {
    async fn a_full_refund_returns_the_price_and_the_creator_keeps_bearing_the_fee(pool: PgPool) {
        let shop = shop(pool).await;
        pay_with_fee(&shop, "evt_1", Some(1_500)).await;

        let res = shop.app.post(&format!("/v1/orders/{}/refund", shop.order)).jar(&shop.creator).json(json!({})).send().await;
        assert_eq!(res.status, StatusCode::ACCEPTED, "{:?}", res.body);

        assert_eq!(journal_kinds(&shop).await, ["gateway_fee", "refund", "sale"]);
        // The provider does not give its fee back, so the creator ends Rs 15 down, not at zero.
        assert_eq!(creator_total(&shop).await, -1_500);
        assert_eq!(account(&shop, "customers").await, 0, "the customer has their money back");
        assert_eq!(account(&shop, "gateway_fees").await, 1_500);
        assert_book_balances(&shop).await;
        let seen = earnings(&shop).await;
        assert_eq!(seen["lifetime"]["refunds_paise"], PRICE);
        assert_eq!(seen["balance_paise"], -1_500);
        assert_eq!(seen["available_paise"], 0, "a negative balance pays out nothing");
        // The refund journal is keyed by our refund.
        assert_eq!(
            n(&shop, "select count(*) from ledger_journals j join refunds r on r.id = j.source_id where j.kind = 'refund' and r.status = 'succeeded'").await,
            1
        );
    }
}

db_test! {
    async fn partial_refunds_each_post_and_add_up(pool: PgPool) {
        let shop = shop(pool).await;
        let payment = pay_with_fee(&shop, "evt_1", Some(1_500)).await;
        deliver(&shop, "evt_r1", refund_processed("rfnd_a", &payment, 10_000, None)).await;
        assert_eq!(creator_total(&shop).await, PRICE - 1_500 - 10_000);
        deliver(&shop, "evt_r2", refund_processed("rfnd_b", &payment, 5_000, None)).await;
        assert_eq!(creator_total(&shop).await, PRICE - 1_500 - 15_000);
        // The same refunds again, under new event ids, post nothing more.
        deliver(&shop, "evt_r3", refund_processed("rfnd_a", &payment, 10_000, None)).await;
        deliver(&shop, "evt_r4", refund_processed("rfnd_b", &payment, 5_000, None)).await;
        assert_eq!(n(&shop, "select count(*) from ledger_journals where kind = 'refund'").await, 2);
        assert_eq!(account(&shop, "customers").await, -PRICE + 15_000);
        assert_book_balances(&shop).await;
    }
}

db_test! {
    async fn a_refund_made_in_the_providers_dashboard_reaches_the_book_too(pool: PgPool) {
        let shop = shop(pool).await;
        let payment = pay_with_fee(&shop, "evt_1", Some(1_500)).await;
        deliver(&shop, "evt_dash", refund_processed("rfnd_dash", &payment, PRICE, None)).await;
        assert_eq!(creator_total(&shop).await, -1_500);
        assert_eq!(journal_kinds(&shop).await, ["gateway_fee", "refund", "sale"]);
    }
}

db_test! {
    async fn refunds_of_money_that_was_never_credited_post_nothing(pool: PgPool) {
        let shop = shop(pool).await;
        // A refund that overtook its payment: nothing was ever credited, so nothing is taken back.
        let entity = payment("pay_1", &shop.remote, PRICE, "refunded");
        deliver(&shop, "evt_1", refund_processed("rfnd_1", "pay_1", PRICE, Some(entity))).await;
        deliver(&shop, "evt_2", captured_with_fee("pay_1", &shop.remote, PRICE, Some(1_500))).await;
        assert_eq!(n(&shop, "select count(*) from ledger_journals").await, 0);
    }
}

db_test! {
    async fn refunding_a_duplicate_payment_leaves_the_creators_sale_alone(pool: PgPool) {
        let shop = shop(pool).await;
        pay_with_fee(&shop, "evt_1", Some(1_500)).await;
        deliver(&shop, "evt_2", captured_with_fee("pay_dup", &shop.remote, PRICE, Some(1_500))).await;
        deliver(&shop, "evt_3", refund_processed("rfnd_dup", "pay_dup", PRICE, None)).await;
        assert_eq!(creator_total(&shop).await, PRICE - 1_500);
        assert_eq!(n(&shop, "select count(*) from ledger_journals where kind = 'refund'").await, 0);
    }
}

// ---- what is payable --------------------------------------------------------------------------------------

db_test! {
    async fn money_inside_the_refund_window_is_pending_and_not_payable(pool: PgPool) {
        let shop = shop(pool).await;
        pay_with_fee(&shop, "evt_1", Some(1_500)).await;
        let inside = earnings(&shop).await;
        assert_eq!(inside["balance_paise"], PRICE - 1_500);
        assert_eq!(inside["pending_paise"], PRICE - 1_500);
        assert_eq!(inside["available_paise"], 0);

        // The window the buyer was promised (7 days) passes.
        age_sale(&shop, shop.order, 8).await;
        let after = earnings(&shop).await;
        assert_eq!(after["pending_paise"], 0);
        assert_eq!(after["available_paise"], PRICE - 1_500);
    }
}

db_test! {
    async fn a_course_without_refunds_pays_out_at_once(pool: PgPool) {
        let shop = shop(pool).await;
        sqlx::query("update orders set price_snapshot = jsonb_set(price_snapshot, '{refund_window_days}', '0')")
            .execute(&shop.pool).await.unwrap();
        pay_with_fee(&shop, "evt_1", Some(1_500)).await;
        let seen = earnings(&shop).await;
        assert_eq!((seen["pending_paise"].as_i64(), seen["available_paise"].as_i64()), (Some(0), Some(PRICE - 1_500)));
    }
}

db_test! {
    async fn only_the_sales_still_inside_their_window_are_held_back(pool: PgPool) {
        let shop = shop(pool).await;
        pay_with_fee(&shop, "evt_1", Some(1_500)).await;
        age_sale(&shop, shop.order, 30).await;
        let (_, _) = another_sale(&shop, "second@ledger.in", Some(1_500)).await;
        let seen = earnings(&shop).await;
        assert_eq!(seen["balance_paise"], 2 * (PRICE - 1_500));
        assert_eq!(seen["pending_paise"], PRICE - 1_500, "only the new sale");
        assert_eq!(seen["available_paise"], PRICE - 1_500);
    }
}

db_test! {
    async fn a_sale_refunded_past_its_net_does_not_shrink_what_other_sales_hold_back(pool: PgPool) {
        let shop = shop(pool).await;
        let payment = pay_with_fee(&shop, "evt_1", Some(1_500)).await;
        // Rs 495 of Rs 499 is returned: still a paid order, but the creator is Rs 11 down on it.
        deliver(&shop, "evt_r", refund_processed("rfnd_big", &payment, 49_500, None)).await;
        another_sale(&shop, "second@ledger.in", Some(1_500)).await;

        let seen = earnings(&shop).await;

        assert_eq!(seen["balance_paise"], (PRICE - 1_500) - 1_100);
        // The second sale is held back in full; the first sale's shortfall is a debt, not a credit
        // against it.
        assert_eq!(seen["pending_paise"], PRICE - 1_500);
        assert_eq!(seen["available_paise"], 0);
    }
}

db_test! {
    async fn a_refund_inside_the_window_releases_what_was_pending(pool: PgPool) {
        let shop = shop(pool).await;
        pay_with_fee(&shop, "evt_1", Some(1_500)).await;
        let res = shop.app.post(&format!("/v1/orders/{}/refund", shop.order)).jar(&shop.creator).json(json!({})).send().await;
        assert_eq!(res.status, StatusCode::ACCEPTED);
        let seen = earnings(&shop).await;
        assert_eq!(seen["pending_paise"], 0);
        assert_eq!(seen["balance_paise"], -1_500);
    }
}

db_test! {
    async fn the_numbers_a_creator_sees_always_add_up(pool: PgPool) {
        let shop = shop(pool).await;
        let payment = pay_with_fee(&shop, "evt_1", Some(1_500)).await;
        let (order2, _) = another_sale(&shop, "second@ledger.in", None).await;
        deliver(&shop, "evt_r", refund_processed("rfnd_a", &payment, 7_000, None)).await;
        age_sale(&shop, shop.order, 30).await;
        age_sale(&shop, order2, 30).await;
        let admin = admin_jar(&shop).await;
        let adjusted = shop.app.post("/v1/admin/ledger/adjustments").jar(&admin)
            .json(json!({"creator_id": shop.creator_id, "amount_paise": -2_500, "reason": "Chargeback lost on an order"})).send().await;
        assert_eq!(adjusted.status, StatusCode::CREATED, "{:?}", adjusted.body);
        let paid_out = shop.app.post("/v1/me/payouts").jar(&shop.creator).json(json!({"amount_paise": 60_000})).send().await;
        assert_eq!(paid_out.status, StatusCode::CREATED, "{:?}", paid_out.body);

        let seen = earnings(&shop).await;
        let l = &seen["lifetime"];
        let identity = l["sales_paise"].as_i64().unwrap() - l["gateway_fees_paise"].as_i64().unwrap()
            - l["refunds_paise"].as_i64().unwrap() + l["adjustments_paise"].as_i64().unwrap()
            - l["paid_out_paise"].as_i64().unwrap();
        assert_eq!(seen["balance_paise"].as_i64().unwrap(), identity, "{seen}");
        assert_eq!(l["sales_paise"], 2 * PRICE);
        assert_eq!(l["refunds_paise"], 7_000);
        assert_eq!(l["adjustments_paise"], -2_500);
        assert_eq!(l["paid_out_paise"], 60_000);
        assert_book_balances(&shop).await;
    }
}

// ---- the rules the database keeps -----------------------------------------------------------------------

db_test! {
    async fn nothing_in_the_book_can_be_edited_or_removed(pool: PgPool) {
        let shop = shop(pool).await;
        pay_with_fee(&shop, "evt_1", Some(1_500)).await;
        let admin = admin_jar(&shop).await;
        shop.app.post("/v1/admin/ledger/adjustments").jar(&admin)
            .json(json!({"creator_id": shop.creator_id, "amount_paise": 100, "reason": "Goodwill credit"})).send().await;
        for statement in [
            "update ledger_entries set amount_paise = amount_paise + 1",
            "delete from ledger_entries",
            "truncate ledger_entries",
            "update ledger_journals set memo = 'x'",
            "delete from ledger_journals",
            "truncate ledger_journals cascade",
            "update ledger_adjustments set amount_paise = 1",
            "delete from ledger_adjustments",
            "truncate ledger_adjustments",
        ] {
            let error = sqlx::query(statement).execute(&shop.pool).await.unwrap_err().to_string();
            assert!(error.contains("append-only"), "{statement}: {error}");
        }
        assert_book_balances(&shop).await;
    }
}

db_test! {
    async fn the_database_refuses_journals_that_do_not_add_up(pool: PgPool) {
        let shop = shop(pool).await;
        let creator = shop.creator_id;
        let post = |entries: Vec<(String, Option<Uuid>, i64)>| {
            let pool = shop.pool.clone();
            async move {
                let mut tx = pool.begin().await?;
                let journal: Uuid = sqlx::query_scalar("insert into ledger_journals (kind, source_id) values ('adjustment', gen_random_uuid()) returning id")
                    .fetch_one(&mut *tx).await?;
                for (account, creator_id, amount) in entries {
                    sqlx::query("insert into ledger_entries (journal_id, account, creator_id, amount_paise) values ($1,$2,$3,$4)")
                        .bind(journal).bind(account).bind(creator_id).bind(amount)
                        .execute(&mut *tx).await?;
                }
                tx.commit().await
            }
        };
        let mine = format!("creator:{creator}");
        // One side only; two sides that differ; entries in the wrong direction by a paisa.
        assert!(post(vec![(mine.clone(), Some(creator), 100)]).await.unwrap_err().to_string().contains("does not balance"));
        assert!(post(vec![(mine.clone(), Some(creator), 100), ("adjustments".into(), None, -99)]).await.unwrap_err().to_string().contains("does not balance"));
        assert!(post(vec![(mine.clone(), Some(creator), 100), ("adjustments".into(), None, -101)]).await.is_err());
        // Balanced is fine.
        post(vec![(mine.clone(), Some(creator), 100), ("adjustments".into(), None, -100)]).await.unwrap();
        // A journal with no entries at all is not a journal (the check below finds it); an entry
        // of nothing, an account that disagrees with its creator, and an unknown account are
        // refused outright.
        assert!(post(vec![(mine.clone(), Some(creator), 0), ("adjustments".into(), None, 0)]).await.is_err());
        assert!(post(vec![("creator:00000000-0000-0000-0000-000000000000".into(), Some(creator), 5), ("adjustments".into(), None, -5)]).await.is_err());
        assert!(post(vec![("invented".into(), None, 5), ("adjustments".into(), None, -5)]).await.is_err());
        assert!(post(vec![(mine, None, 5), ("adjustments".into(), None, -5)]).await.is_err());
        assert_eq!(n(&shop, "select count(*) from ledger_journals").await, 1, "only the balanced one survived");
    }
}

db_test! {
    async fn a_journal_cannot_be_extended_later_even_with_a_balanced_pair(pool: PgPool) {
        let shop = shop(pool).await;
        pay_with_fee(&shop, "evt_1", Some(1_500)).await;
        let journal: Uuid = sqlx::query_scalar("select id from ledger_journals where kind = 'sale'")
            .fetch_one(&shop.pool).await.unwrap();
        let mut tx = shop.pool.begin().await.unwrap();
        let first = sqlx::query("insert into ledger_entries (journal_id, account, amount_paise) values ($1, 'adjustments', 5)")
            .bind(journal).execute(&mut *tx).await;
        assert!(first.unwrap_err().to_string().contains("transaction that created it"));
    }
}

db_test! {
    async fn an_event_has_one_journal_however_often_it_is_posted(pool: PgPool) {
        let shop = shop(pool).await;
        let source = Uuid::new_v4();
        let mut tx = shop.pool.begin().await.unwrap();
        let first = post::adjustment(&mut tx, source, shop.creator_id, 500, "first time").await.unwrap();
        let second = post::adjustment(&mut tx, source, shop.creator_id, 500, "again").await.unwrap();
        tx.commit().await.unwrap();
        assert!(first && !second);
        assert_eq!(creator_total(&shop).await, 500);
        // Posting outside a transaction is refused rather than done by halves: the journal and
        // its entries would otherwise commit separately.
        let mut conn = shop.pool.acquire().await.unwrap();
        let alone = post::adjustment(&mut conn, Uuid::new_v4(), shop.creator_id, 700, "no transaction").await;
        assert!(alone.unwrap_err().to_string().contains("transaction that created it"));
        assert_eq!(creator_total(&shop).await, 500);
    }
}

// ---- the nightly check -------------------------------------------------------------------------------------

db_test! {
    async fn a_book_that_agrees_with_everything_else_has_no_problems(pool: PgPool) {
        let shop = shop(pool).await;
        let payment = pay_with_fee(&shop, "evt_1", Some(1_500)).await;
        deliver(&shop, "evt_r", refund_processed("rfnd_a", &payment, 7_000, None)).await;
        let admin = admin_jar(&shop).await;
        pay_for_payout(&shop, &admin).await;
        shop.app.post("/v1/admin/ledger/adjustments").jar(&admin)
            .json(json!({"creator_id": shop.creator_id, "amount_paise": 300, "reason": "Goodwill credit"})).send().await;

        assert_eq!(integrity::check(&shop.pool).await.unwrap(), vec![]);
        let res = shop.app.get("/v1/admin/ledger/integrity").jar(&admin).send().await;
        assert_eq!((res.status, &res.body["ok"], res.body["problems"].as_array().map(Vec::len)), (StatusCode::OK, &json!(true), Some(0)));
    }
}

/// Makes the creator's money payable and requests Rs 500 of it, so payouts are part of what is
/// checked. (One sale nets less than the Rs 500 minimum, so the admin tops it up first.)
async fn pay_for_payout(shop: &Shop, admin: &Jar) {
    let credit = shop.app.post("/v1/admin/ledger/adjustments").jar(admin)
        .json(json!({"creator_id": shop.creator_id, "amount_paise": 100_000, "reason": "Seed balance for a test"})).send().await;
    assert_eq!(credit.status, StatusCode::CREATED, "{:?}", credit.body);
    age_sale(shop, shop.order, 30).await;
    let res = shop
        .app
        .post("/v1/me/payouts")
        .jar(&shop.creator)
        .json(json!({"amount_paise": 50_000}))
        .send()
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{:?}", res.body);
}

db_test! {
    async fn the_check_finds_each_kind_of_disagreement(pool: PgPool) {
        let shop = shop(pool).await;
        let payment = pay_with_fee(&shop, "evt_1", Some(1_500)).await;
        let admin = admin_jar(&shop).await;
        pay_for_payout(&shop, &admin).await;
        let kinds = |found: Vec<integrity::Problem>| -> Vec<&'static str> {
            let mut kinds: Vec<_> = found.into_iter().map(|p| p.kind).collect();
            kinds.sort_unstable();
            kinds.dedup();
            kinds
        };
        assert!(kinds(integrity::check(&shop.pool).await.unwrap()).is_empty());

        // A payment that paid an order but was never posted.
        as_a_careless_operator(&shop, &[&format!(
            "insert into payments (provider, provider_ref, order_id, amount_paise, status, raw_event) \
             values ('fake', 'pay_orphan', '{}', 100, 'duplicate', '{{}}')", shop.order)]).await;
        let orphan: Uuid = sqlx::query_scalar("select id from payments where provider_ref = 'pay_orphan'")
            .fetch_one(&shop.pool).await.unwrap();
        sqlx::query("insert into order_events (order_id, kind, payment_id) values ($1, 'paid', $2)")
            .bind(shop.order).bind(orphan).execute(&shop.pool).await.unwrap();
        // The payment's amount changed after it was posted; the provider's fee likewise.
        as_a_careless_operator(&shop, &[
            &format!("update payments set amount_paise = amount_paise + 1 where provider_ref = '{payment}'"),
            &format!("update payments set fee_paise = 1 where provider_ref = '{payment}'"),
            // A refund that succeeded but was never posted; a payout and an adjustment that were
            // never posted; a journal that does not balance; a refund journal for nothing.
            &format!("insert into refunds (order_id, payment_id, amount_paise, status, origin, completed_at) \
                      select '{}', id, 100, 'succeeded', 'provider', now() from payments where provider_ref = '{payment}'", shop.order),
            &format!("insert into ledger_adjustments (creator_id, amount_paise, reason) values ('{}', 5, 'never posted')", shop.creator_id),
            "insert into ledger_journals (kind, source_id) values ('refund', gen_random_uuid())",
            "insert into ledger_entries (journal_id, account, amount_paise) select id, 'customers', 7 from ledger_journals where kind = 'refund' and not exists (select 1 from refunds r where r.id = source_id)",
        ]).await;
        sqlx::query("update payouts set status = 'cancelled', cancelled_at = now()").execute(&shop.pool).await.unwrap();

        let found = kinds(integrity::check(&shop.pool).await.unwrap());
        for expected in [
            "adjustment_mismatch",
            "fee_mismatch",
            "missing_sale",
            "order_net_mismatch",
            "payout_not_reversed",
            "refund_not_posted",
            "refund_without_refund",
            "sale_amount_mismatch",
            "unbalanced_journal",
        ] {
            assert!(found.contains(&expected), "missing {expected}: {found:?}");
        }
    }
}

db_test! {
    async fn old_sales_and_refunds_can_be_brought_into_the_book(pool: PgPool) {
        let shop = shop(pool).await;
        let admin = admin_jar(&shop).await;
        // A sale and a refund from before the ledger existed: money moved, nothing was posted.
        sqlx::query("update orders set status = 'paid', paid_at = now() where id = $1").bind(shop.order).execute(&shop.pool).await.unwrap();
        sqlx::query("insert into payments (provider, provider_ref, order_id, amount_paise, status, raw_event, fee_paise) values ('fake', 'pay_old', $1, $2, 'confirmed', '{}', 1200)")
            .bind(shop.order).bind(PRICE).execute(&shop.pool).await.unwrap();
        let old: Uuid = sqlx::query_scalar("select id from payments where provider_ref = 'pay_old'").fetch_one(&shop.pool).await.unwrap();
        sqlx::query("insert into order_events (order_id, kind, payment_id) values ($1, 'paid', $2)").bind(shop.order).bind(old).execute(&shop.pool).await.unwrap();
        sqlx::query("insert into refunds (order_id, payment_id, amount_paise, status, origin, completed_at) values ($1, $2, 10000, 'succeeded', 'provider', now())")
            .bind(shop.order).bind(old).execute(&shop.pool).await.unwrap();
        let before = integrity::check(&shop.pool).await.unwrap();
        assert!(before.iter().any(|p| p.kind == "missing_sale"), "{before:?}");

        let res = shop.app.post("/v1/admin/ledger/backfill").jar(&admin).json(json!({})).send().await;
        assert_eq!(res.status, StatusCode::OK, "{:?}", res.body);
        assert_eq!((res.body["sales_posted"].as_i64(), res.body["refunds_posted"].as_i64()), (Some(1), Some(1)));

        assert_eq!(creator_total(&shop).await, PRICE - 1_200 - 10_000);
        assert_eq!(integrity::check(&shop.pool).await.unwrap(), vec![], "the book now agrees");
        // Running it again finds nothing to do.
        let again = shop.app.post("/v1/admin/ledger/backfill").jar(&admin).json(json!({})).send().await;
        assert_eq!((again.body["sales_posted"].as_i64(), again.body["refunds_posted"].as_i64()), (Some(0), Some(0)));
    }
}

// ---- adjustments ----------------------------------------------------------------------------------------------

db_test! {
    async fn an_admin_corrects_a_balance_with_a_reason_on_record(pool: PgPool) {
        let shop = shop(pool).await;
        let admin = admin_jar(&shop).await;
        let up = shop.app.post("/v1/admin/ledger/adjustments").jar(&admin)
            .json(json!({"creator_id": shop.creator_id, "amount_paise": 5_000, "reason": "Launch bonus"})).send().await;
        let down = shop.app.post("/v1/admin/ledger/adjustments").jar(&admin)
            .json(json!({"creator_id": shop.creator_id, "amount_paise": -1_200, "reason": "Chargeback lost on order X"})).send().await;
        assert_eq!((up.status, down.status), (StatusCode::CREATED, StatusCode::CREATED), "{:?} {:?}", up.body, down.body);
        assert_eq!(creator_total(&shop).await, 3_800);
        assert_eq!(account(&shop, "adjustments").await, -3_800);
        assert_eq!(
            text(&shop, "select string_agg(memo, ' | ' order by memo) from ledger_journals where kind = 'adjustment'").await,
            "Chargeback lost on order X | Launch bonus"
        );
        assert_eq!(n(&shop, "select count(*) from ledger_adjustments where created_by is not null").await, 2);
        assert_eq!(earnings(&shop).await["lifetime"]["adjustments_paise"], 3_800);
        assert_book_balances(&shop).await;
    }
}

db_test! {
    async fn adjustments_are_admin_only_and_only_for_creators(pool: PgPool) {
        let shop = shop(pool).await;
        let admin = admin_jar(&shop).await;
        let body = |creator: Uuid, amount: i64, reason: &str| json!({"creator_id": creator, "amount_paise": amount, "reason": reason});
        let send = |jar: &Jar, b: Value| shop.app.post("/v1/admin/ledger/adjustments").jar(jar).json(b).send();
        assert_eq!(send(&shop.creator, body(shop.creator_id, 100, "Self service")).await.status, StatusCode::FORBIDDEN);
        assert_eq!(send(&shop.buyer, body(shop.creator_id, 100, "Self service")).await.status, StatusCode::FORBIDDEN);
        assert_eq!(shop.app.post("/v1/admin/ledger/adjustments").json(body(shop.creator_id, 100, "Anonymous")).send().await.status, StatusCode::UNAUTHORIZED);
        // A student is not a creator; nobody is not either.
        assert_eq!(send(&admin, body(shop.buyer_id, 100, "Wrong person")).await.status, StatusCode::NOT_FOUND);
        assert_eq!(send(&admin, body(Uuid::new_v4(), 100, "Nobody")).await.status, StatusCode::NOT_FOUND);
        for (amount, reason) in [(0, "Zero"), (100_000_001, "Too much"), (-100_000_001, "Too much"), (100, "no")] {
            assert_eq!(send(&admin, body(shop.creator_id, amount, reason)).await.status, StatusCode::UNPROCESSABLE_ENTITY, "{amount} {reason}");
        }
        assert_eq!(n(&shop, "select count(*) from ledger_adjustments").await, 0);
    }
}

// ---- reading it back -----------------------------------------------------------------------------------------

db_test! {
    async fn entries_are_listed_newest_first_a_page_at_a_time_without_gaps(pool: PgPool) {
        let shop = shop(pool).await;
        pay_with_fee(&shop, "evt_1", Some(1_500)).await;
        let admin = admin_jar(&shop).await;
        for (i, amount) in [100, 200, 300].into_iter().enumerate() {
            shop.app.post("/v1/admin/ledger/adjustments").jar(&admin)
                .json(json!({"creator_id": shop.creator_id, "amount_paise": amount, "reason": format!("Credit number {i}")})).send().await;
        }
        let mut seen: Vec<String> = Vec::new();
        let mut cursor: Option<String> = None;
        let mut pages = 0;
        loop {
            let url = match &cursor {
                Some(c) => format!("/v1/me/earnings/entries?limit=2&cursor={c}"),
                None => "/v1/me/earnings/entries?limit=2".to_owned(),
            };
            let res = shop.app.get(&url).jar(&shop.creator).send().await;
            assert_eq!(res.status, StatusCode::OK, "{:?}", res.body);
            for entry in res.body["entries"].as_array().unwrap() {
                seen.push(entry["id"].as_str().unwrap().to_owned());
            }
            pages += 1;
            match res.body["next_cursor"].as_str() {
                Some(next) => cursor = Some(next.to_owned()),
                None => break,
            }
        }
        // sale + fee + three adjustments, two a page.
        assert_eq!((seen.len(), pages), (5, 3));
        let mut unique = seen.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), 5, "no entry twice");
        let all = shop.app.get("/v1/me/earnings/entries?limit=100").jar(&shop.creator).send().await;
        let in_one_go: Vec<String> = all.body["entries"].as_array().unwrap().iter().map(|e| e["id"].as_str().unwrap().to_owned()).collect();
        assert_eq!(seen, in_one_go, "the pages are the whole list, in order");
        let kinds: Vec<&str> = all.body["entries"].as_array().unwrap().iter().map(|e| e["kind"].as_str().unwrap()).collect();
        assert!(kinds.contains(&"sale") && kinds.contains(&"gateway_fee") && kinds.contains(&"adjustment"));
        let sale = all.body["entries"].as_array().unwrap().iter().find(|e| e["kind"] == "sale").unwrap();
        assert_eq!((sale["amount_paise"].as_i64(), sale["course_title"].as_str()), (Some(PRICE), Some("Paid course")));
        // A forged cursor is refused.
        let bad = shop.app.get("/v1/me/earnings/entries?cursor=garbage").jar(&shop.creator).send().await;
        assert_eq!((bad.status, bad.code()), (StatusCode::BAD_REQUEST, "invalid_cursor"));
    }
}

async fn csv(shop: &Shop, query: &str) -> (StatusCode, axum::http::HeaderMap, String) {
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let request = Request::builder()
        .method(Method::GET)
        .uri(format!("/v1/me/earnings/export.csv{query}"))
        .header(
            header::COOKIE,
            shop.creator.header_for("/v1/").unwrap_or_default(),
        )
        .body(Body::empty())
        .unwrap();
    let response = shop.app.router.clone().oneshot(request).await.unwrap();
    let (parts, body) = response.into_parts();
    let bytes = body.collect().await.unwrap().to_bytes();
    (
        parts.status,
        parts.headers,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

db_test! {
    async fn the_export_is_a_private_spreadsheet_that_cannot_run_anything(pool: PgPool) {
        let shop = shop(pool).await;
        // A course title chosen to hurt whoever opens the file.
        sqlx::query("update orders set price_snapshot = jsonb_set(price_snapshot, '{course_title}', to_jsonb($1::text))")
            .bind("=HYPERLINK(\"http://evil\",\"click\"), \"Maths\"")
            .execute(&shop.pool).await.unwrap();
        pay_with_fee(&shop, "evt_1", Some(1_500)).await;

        let (status, headers, body) = csv(&shop, "").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers[header::CONTENT_TYPE], "text/csv; charset=utf-8");
        assert_eq!(headers[header::CACHE_CONTROL], "private, no-store");
        assert!(headers[header::CONTENT_DISPOSITION].to_str().unwrap().starts_with("attachment; filename=\"earnings-"));
        let mut lines = body.lines();
        assert_eq!(lines.next().unwrap(), "date_utc,kind,order_id,course,amount_paise,amount_inr,note,entry_id");
        let rows: Vec<&str> = lines.collect();
        assert_eq!(rows.len(), 2, "{body}");
        let sale = rows.iter().find(|r| r.contains(",sale,")).unwrap();
        assert!(sale.contains(",49900,499.00,"), "{sale}");
        // The formula is defused and quoted: it begins with ' inside a quoted cell.
        assert!(sale.contains("\"'=HYPERLINK(\"\"http://evil\"\",\"\"click\"\"), \"\"Maths\"\"\""), "{sale}");
        let fee = rows.iter().find(|r| r.contains(",gateway_fee,")).unwrap();
        assert!(fee.contains(",-1500,-15.00,"), "negative numbers are plain numbers: {fee}");
    }
}

db_test! {
    async fn export_ranges_are_checked_and_follow_the_indian_calendar(pool: PgPool) {
        let shop = shop(pool).await;
        pay_with_fee(&shop, "evt_1", Some(1_500)).await;
        // The sale at 23:59 Indian time on 31 March, the fee a minute later on 1 April.
        as_a_careless_operator(&shop, &[
            &format!("update ledger_entries set created_at = '2027-03-31 18:29:00+00' where creator_id = '{c}' and journal_id = (select id from ledger_journals where kind = 'sale')", c = shop.creator_id),
            &format!("update ledger_entries set created_at = '2027-03-31 18:30:00+00' where creator_id = '{c}' and journal_id = (select id from ledger_journals where kind = 'gateway_fee')", c = shop.creator_id),
        ]).await;
        let rows = |body: &str| body.lines().skip(1).count();
        assert_eq!(rows(&csv(&shop, "?from=2027-03-31&to=2027-03-31").await.2), 1, "the sale");
        assert_eq!(rows(&csv(&shop, "?from=2027-04-01&to=2027-04-01").await.2), 1, "the fee");
        assert_eq!(rows(&csv(&shop, "?from=2027-03-31&to=2027-04-01").await.2), 2);
        assert_eq!(rows(&csv(&shop, "?from=2027-04-02&to=2027-04-30").await.2), 0);
    }
}

db_test! {
    async fn export_ranges_that_make_no_sense_are_refused(pool: PgPool) {
        let shop = shop(pool).await;
        for (query, label) in [
            ("?from=2027-04-02&to=2027-04-01", "backwards"),
            ("?from=2026-01-01&to=2027-06-01", "more than a year"),
            ("?from=yesterday", "not a date"),
            ("?from=2027-13-01&to=2027-13-02", "not a day"),
            ("?to=31-03-2027", "wrong order"),
        ] {
            let (status, _, _) = csv(&shop, query).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{label}");
        }
        // Exactly a year is allowed.
        assert_eq!(csv(&shop, "?from=2026-04-01&to=2027-03-31").await.0, StatusCode::OK);
    }
}

db_test! {
    async fn downloading_exports_over_and_over_is_slowed(pool: PgPool) {
        let shop = shop(pool).await;
        let mut statuses = Vec::new();
        for _ in 0..8 {
            statuses.push(csv(&shop, "").await.0);
        }
        let ok = statuses.iter().filter(|s| **s == StatusCode::OK).count();
        let slowed = statuses.iter().filter(|s| **s == StatusCode::TOO_MANY_REQUESTS).count();
        assert_eq!((ok, slowed), (6, 2), "{statuses:?}");
    }
}

db_test! {
    async fn an_export_too_big_to_build_is_refused_not_attempted(pool: PgPool) {
        let shop = shop(pool).await;
        as_a_careless_operator(&shop, &[&format!(
            "with j as (insert into ledger_journals (kind, source_id) \
                        select 'adjustment', gen_random_uuid() from generate_series(1, 50001) returning id) \
             insert into ledger_entries (journal_id, account, creator_id, amount_paise) \
             select id, 'creator:{c}', '{c}'::uuid, 1 from j \
             union all select id, 'adjustments', null, -1 from j", c = shop.creator_id)]).await;
        let (status, _, body) = csv(&shop, "").await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(body.contains("export_too_large"), "{body}");
        // The list endpoint is paged, so it never has this problem.
        assert_eq!(shop.app.get("/v1/me/earnings/entries?limit=100").jar(&shop.creator).send().await.status, StatusCode::OK);
    }
}

db_test! {
    async fn creators_see_only_their_own_money_and_students_none(pool: PgPool) {
        let shop = shop(pool).await;
        pay_with_fee(&shop, "evt_1", Some(1_500)).await;
        shop.app.settle().await;
        let rival = signup_creator(&shop.app, "rival@ledger.in").await;
        let theirs = shop.app.get("/v1/me/earnings").jar(&rival).send().await;
        assert_eq!(theirs.status, StatusCode::OK);
        assert_eq!(theirs.body["earnings"]["balance_paise"], 0);
        let entries = shop.app.get("/v1/me/earnings/entries").jar(&rival).send().await;
        assert_eq!(entries.body["entries"].as_array().unwrap().len(), 0);
        assert_eq!(csv_for(&shop, &rival).await.lines().count(), 1, "just the header");
        for url in ["/v1/me/earnings", "/v1/me/earnings/entries", "/v1/me/earnings/export.csv"] {
            assert_eq!(shop.app.get(url).jar(&shop.buyer).send().await.status, StatusCode::FORBIDDEN, "{url}");
            assert_eq!(shop.app.get(url).send().await.status, StatusCode::UNAUTHORIZED, "{url}");
        }
    }
}

async fn csv_for(shop: &Shop, jar: &Jar) -> String {
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let request = Request::builder()
        .uri("/v1/me/earnings/export.csv")
        .header(header::COOKIE, jar.header_for("/v1/").unwrap_or_default())
        .body(Body::empty())
        .unwrap();
    let response = shop.app.router.clone().oneshot(request).await.unwrap();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).into_owned()
}

db_test! {
    async fn admins_check_and_repair_the_book_and_nobody_else_can(pool: PgPool) {
        let shop = shop(pool).await;
        for (method, url) in [(Method::GET, "/v1/admin/ledger/integrity"), (Method::POST, "/v1/admin/ledger/backfill")] {
            let as_creator = shop.app.req(method.clone(), url).jar(&shop.creator).json(json!({})).send().await;
            let as_buyer = shop.app.req(method.clone(), url).jar(&shop.buyer).json(json!({})).send().await;
            let anonymous = shop.app.req(method, url).json(json!({})).send().await;
            assert_eq!((as_creator.status, as_buyer.status, anonymous.status), (StatusCode::FORBIDDEN, StatusCode::FORBIDDEN, StatusCode::UNAUTHORIZED), "{url}");
        }
    }
}

// ---- the arithmetic, at random ---------------------------------------------------------------------------------

db_test! {
    async fn whatever_happens_the_balance_is_what_the_formula_says(pool: PgPool) {
        // Several creators, hundreds of random sales, refunds, payouts, cancellations and
        // corrections, checked against a model that never touches the database.
        let shop = shop(pool).await;
        let mut creators = vec![shop.creator_id];
        for i in 0..2 {
            let id: Uuid = sqlx::query_scalar("insert into users (name, email, password_hash, role) values ($1, $2, 'x', 'creator') returning id")
                .bind(format!("Creator {i}")).bind(format!("model{i}@ledger.in")).fetch_one(&shop.pool).await.unwrap();
            creators.push(id);
        }
        let (mut gross_total, mut refunded_total, mut fee_total) = (0i64, 0i64, 0i64);
        for seed in [1u64, 2, 3] {
            let mut rng = StdRng::seed_from_u64(seed);
            let mut conn = shop.pool.acquire().await.unwrap();
            struct Sold { payment: Uuid, creator: usize, left: i64 }
            let mut sold: Vec<Sold> = Vec::new();
            let mut open_payouts: Vec<(Uuid, usize, i64)> = Vec::new();
            let mut expected = vec![0i64; creators.len()];
            let before: Vec<i64> = {
                let mut v = Vec::new();
                for c in &creators { v.push(ledger_repo::position(&mut *conn, *c).await.unwrap().total_paise); }
                v
            };
            for _ in 0..300 {
                match rng.gen_range(0..10) {
                    0..=3 => {
                        let creator = rng.gen_range(0..creators.len());
                        let gross = rng.gen_range(100..=5_000_000i64);
                        let fee = fee::resolve(gross, rng.gen_bool(0.5).then(|| rng.gen_range(0..=gross / 10)), 236);
                        let payment = Uuid::new_v4();
                        let mut tx = conn.begin().await.unwrap();
                        post::sale(&mut tx, &Sale { payment_id: payment, order_id: shop.order, creator_id: creators[creator], gross_paise: gross, fee }).await.unwrap();
                        tx.commit().await.unwrap();
                        expected[creator] += gross - fee.paise;
                        gross_total += gross;
                        fee_total += fee.paise;
                        sold.push(Sold { payment, creator, left: gross });
                    }
                    4..=5 if !sold.is_empty() => {
                        let at = rng.gen_range(0..sold.len());
                        if sold[at].left == 0 { continue; }
                        let amount = rng.gen_range(1..=sold[at].left);
                        let refund = Uuid::new_v4();
                        let mut tx = conn.begin().await.unwrap();
                        assert!(post::refund(&mut tx, sold[at].payment, refund, amount).await.unwrap());
                        tx.commit().await.unwrap();
                        sold[at].left -= amount;
                        expected[sold[at].creator] -= amount;
                        refunded_total += amount;
                    }
                    6 => {
                        let creator = rng.gen_range(0..creators.len());
                        let amount = rng.gen_range(100..=200_000i64);
                        let payout = Uuid::new_v4();
                        let mut tx = conn.begin().await.unwrap();
                        post::payout(&mut tx, payout, creators[creator], amount).await.unwrap();
                        tx.commit().await.unwrap();
                        expected[creator] -= amount;
                        open_payouts.push((payout, creator, amount));
                    }
                    7 if !open_payouts.is_empty() => {
                        let (payout, creator, amount) = open_payouts.remove(rng.gen_range(0..open_payouts.len()));
                        let mut tx = conn.begin().await.unwrap();
                        post::payout_reversal(&mut tx, payout, creators[creator], amount).await.unwrap();
                        tx.commit().await.unwrap();
                        expected[creator] += amount;
                    }
                    _ => {
                        let creator = rng.gen_range(0..creators.len());
                        let amount = rng.gen_range(-50_000..=50_000i64);
                        if amount == 0 { continue; }
                        let mut tx = conn.begin().await.unwrap();
                        post::adjustment(&mut tx, Uuid::new_v4(), creators[creator], amount, "model").await.unwrap();
                        tx.commit().await.unwrap();
                        expected[creator] += amount;
                    }
                }
            }
            for (i, creator) in creators.iter().enumerate() {
                let now = ledger_repo::position(&mut *conn, *creator).await.unwrap().total_paise;
                assert_eq!(now - before[i], expected[i], "seed {seed}, creator {i}");
            }
            // Money is only ever moved: the whole book still adds up to nothing.
            assert_book_balances(&shop).await;
        }
        // The customers' and the provider's accounts agree with the totals.
        assert_eq!(account(&shop, "customers").await, -(gross_total - refunded_total));
        assert_eq!(account(&shop, "gateway_fees").await, fee_total);
        assert_eq!(account(&shop, "customers").await + account(&shop, "gateway_fees").await
            + sum_of_creators(&shop).await + account(&shop, "payouts").await + account(&shop, "adjustments").await, 0);
        // Posting any of it again changes nothing: the keys are the events.
        let events = n(&shop, "select count(*) from ledger_journals").await;
        assert!(events > 600);
    }
}

async fn sum_of_creators(shop: &Shop) -> i64 {
    n(shop, "select coalesce(sum(amount_paise), 0)::bigint from ledger_entries where creator_id is not null").await
}

#[test]
fn the_fee_rule_is_the_one_the_book_uses() {
    // The unit tests of the rule live beside it; this pins the numbers the integration tests rely on.
    assert_eq!(fee::estimate(49_900, 236), 1_178);
    let reported = Fee {
        paise: 5,
        source: FeeSource::Reported,
    };
    assert_eq!(fee::resolve(49_900, Some(5), 236), reported);
}
