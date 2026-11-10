//! Reconciliation: payment and refund states that webhooks never settled get settled anyway, by
//! asking the provider, through the same code. Time is moved by editing timestamps, as the job
//! only ever compares them with the database's clock.

mod common;

use std::collections::BTreeMap;
use std::time::Duration;

use axum::http::StatusCode;
use common::shop::*;
use common::*;
use futures_util::future::join;
use serde_json::json;
use sqlx::PgPool;
use zeno_api::reconcile::{run_once, Report};
use zeno_core::payments::{Fault, Op, PaymentProvider, ProviderError, RefundRequest};

macro_rules! db_test {
    ($(#[$meta:meta])* async fn $name:ident($pool:ident: PgPool) $body:block) => {
        #[sqlx::test(migrator = "zeno_core::db::MIGRATOR")]
        $(#[$meta])*
        async fn $name($pool: PgPool) $body
    };
}

async fn run(shop: &Shop) -> Report {
    run_once(&shop.app.state).await.expect("a pass completes")
}

/// Makes the order `minutes` old (so it expires `30 - minutes` from now).
async fn age_order(shop: &Shop, minutes: i64) {
    sqlx::query(
        "update orders set created_at = now() - make_interval(mins => $2::int), \
            expires_at = now() - make_interval(mins => $2::int) + interval '30 minutes' \
         where id = $1",
    )
    .bind(shop.order)
    .bind(minutes)
    .execute(&shop.pool)
    .await
    .unwrap();
}

async fn clear_backoff(shop: &Shop) {
    sqlx::query("update orders set reconcile_after = null")
        .execute(&shop.pool)
        .await
        .unwrap();
}

/// The customer paid at the provider; no webhook ever reached us.
fn customer_pays(shop: &Shop) -> String {
    shop.provider.capture(&shop.remote).unwrap().id
}

async fn refund_status_of(shop: &Shop) -> String {
    text(shop, "select status::text from refunds").await
}

async fn age_refund(shop: &Shop, minutes: i32) {
    sqlx::query("update refunds set updated_at = now() - make_interval(mins => $1)")
        .bind(minutes)
        .execute(&shop.pool)
        .await
        .unwrap();
}

async fn paid_for_real(pool: PgPool) -> (Shop, String) {
    let shop = shop(pool).await;
    let payment = customer_pays(&shop);
    let res = deliver(&shop, "evt_paid", captured(&payment, &shop.remote, PRICE)).await;
    assert_eq!(outcome(&res), "enrolled", "{:?}", res.body);
    (shop, payment)
}

// ---- payments the webhook missed ---------------------------------------------------------------

db_test! {
    async fn a_payment_whose_webhook_never_came_is_found_and_enrolls_the_buyer(pool: PgPool) {
        let shop = shop(pool).await;
        let payment = customer_pays(&shop);
        age_order(&shop, 20).await;
        shop.app.settle().await;
        let mails = shop.app.mailer.sent().len();

        let report = run(&shop).await;

        assert_eq!((report.orders_checked, report.orders_settled), (1, 1), "{report:?}");
        assert_eq!(order_status(&shop).await, "paid");
        assert_eq!(access(&shop).await, "allowed");
        let status: String = sqlx::query_scalar("select status::text from payments where provider_ref = $1")
            .bind(&payment)
            .fetch_one(&shop.pool)
            .await
            .unwrap();
        assert_eq!(status, "confirmed");
        let (event_type, event_outcome): (String, String) = sqlx::query_as(
            "select event_type, outcome from webhook_events where provider_event_id like 'reconcile:%'",
        )
        .fetch_one(&shop.pool)
        .await
        .unwrap();
        assert_eq!((event_type.as_str(), event_outcome.as_str()), ("payment.captured", "enrolled"));
        // The buyer gets the same receipt a webhook would have produced.
        let receipt = shop.app.mailer.wait_for(mails + 1).await.pop().unwrap();
        assert!(receipt.subject.contains("Payment received"), "{}", receipt.subject);

        // Nothing is left to do.
        assert_eq!(run(&shop).await, Report::default());
        assert_eq!(n(&shop, "select count(*) from enrollments").await, 1);
    }
}

db_test! {
    async fn a_young_order_is_left_for_the_webhook(pool: PgPool) {
        let shop = shop(pool).await;
        customer_pays(&shop);
        age_order(&shop, 10).await;
        assert_eq!(run(&shop).await, Report::default());
        assert_eq!(shop.provider.calls(Op::FetchOrder), 0, "the provider is not even asked");
        assert_eq!(order_status(&shop).await, "pending");
    }
}

db_test! {
    async fn the_webhook_arriving_after_reconciliation_changes_nothing(pool: PgPool) {
        let shop = shop(pool).await;
        let payment = customer_pays(&shop);
        age_order(&shop, 20).await;
        run(&shop).await;
        let late = deliver(&shop, "evt_late", captured(&payment, &shop.remote, PRICE)).await;
        assert_eq!(outcome(&late), "already_applied");
        assert_eq!(n(&shop, "select count(*) from enrollments").await, 1);
        assert_eq!(n(&shop, "select count(*) from payments").await, 1);
        assert_eq!(n(&shop, "select count(*) from order_events where kind='paid'").await, 1);
    }
}

db_test! {
    async fn reconciliation_racing_a_live_webhook_enrolls_once(pool: PgPool) {
        let shop = shop(pool).await;
        let payment = customer_pays(&shop);
        age_order(&shop, 20).await;
        let (report, hook) = join(
            run_once(&shop.app.state),
            deliver(&shop, "evt_live", captured(&payment, &shop.remote, PRICE)),
        )
        .await;
        report.unwrap();
        assert_eq!(hook.status, StatusCode::OK, "{:?}", hook.body);
        assert_eq!(n(&shop, "select count(*) from enrollments").await, 1);
        assert_eq!(n(&shop, "select count(*) from payments").await, 1);
        assert_eq!(n(&shop, "select count(*) from order_events where kind='paid'").await, 1);
        assert_eq!(order_status(&shop).await, "paid");
    }
}

db_test! {
    async fn a_late_payment_on_an_order_that_already_expired_is_found_for_a_day(pool: PgPool) {
        let shop = shop(pool).await;
        customer_pays(&shop);
        sqlx::query("update orders set status='expired', expires_at = now() - interval '3 hours', created_at = now() - interval '4 hours'")
            .execute(&shop.pool)
            .await
            .unwrap();

        let report = run(&shop).await;

        assert_eq!(report.orders_settled, 1, "{report:?}");
        assert_eq!(order_status(&shop).await, "paid");
        assert_eq!(access(&shop).await, "allowed");
    }
}

db_test! {
    async fn after_a_day_an_expired_order_is_no_longer_asked_about(pool: PgPool) {
        let shop = shop(pool).await;
        customer_pays(&shop);
        sqlx::query("update orders set status='expired', expires_at = now() - interval '25 hours', created_at = now() - interval '26 hours'")
            .execute(&shop.pool)
            .await
            .unwrap();
        assert_eq!(run(&shop).await, Report::default());
        assert_eq!(shop.provider.calls(Op::FetchOrder), 0);
        assert_eq!(order_status(&shop).await, "expired");
    }
}

db_test! {
    async fn a_payment_refunded_before_we_ever_heard_of_it_grants_nothing(pool: PgPool) {
        let shop = shop(pool).await;
        let payment = customer_pays(&shop);
        shop.provider
            .refund(&RefundRequest {
                payment_id: payment,
                amount_paise: PRICE,
                idempotency_key: "dashboard".into(),
                notes: BTreeMap::new(),
            })
            .await
            .unwrap();
        age_order(&shop, 20).await;
        shop.app.settle().await;
        let mails = shop.app.mailer.sent().len();

        run(&shop).await;
        shop.app.settle().await;

        assert_eq!(order_status(&shop).await, "refunded");
        assert_eq!(n(&shop, "select count(*) from enrollments").await, 0);
        assert_eq!(access(&shop).await, "access_revoked");
        assert_eq!(text(&shop, "select status::text from payments").await, "refunded");
        assert_eq!(shop.app.mailer.sent().len(), mails, "no receipt for money that was returned");
    }
}

// ---- time alone ------------------------------------------------------------------------------------

db_test! {
    async fn an_unpaid_order_past_its_time_expires_and_the_buyer_can_start_again(pool: PgPool) {
        let shop = shop(pool).await;
        age_order(&shop, 40).await;

        let report = run(&shop).await;

        assert_eq!((report.orders_checked, report.orders_expired), (1, 1), "{report:?}");
        assert_eq!(order_status(&shop).await, "expired");
        let again = shop.app.post(&format!("/v1/checkout/{}", shop.course)).jar(&shop.buyer).send().await;
        assert_eq!(again.status, StatusCode::CREATED, "{:?}", again.body);
    }
}

db_test! {
    async fn an_order_that_is_not_yet_due_stays_open(pool: PgPool) {
        let shop = shop(pool).await;
        age_order(&shop, 20).await;
        let report = run(&shop).await;
        assert_eq!((report.orders_checked, report.orders_expired), (1, 0));
        assert_eq!(order_status(&shop).await, "pending");
        // It is looked at again in minutes, not on the next pass.
        assert_eq!(run(&shop).await, Report::default());
    }
}

db_test! {
    async fn a_payment_attempt_in_flight_gets_an_hour_of_grace(pool: PgPool) {
        let shop = shop(pool).await;
        shop.provider.fail_payment(&shop.remote).unwrap(); // the order is now "attempted"
        age_order(&shop, 40).await;
        let report = run(&shop).await;
        assert_eq!(report.orders_expired, 0, "{report:?}");
        assert_eq!(order_status(&shop).await, "pending");

        // The hour passes with no payment.
        age_order(&shop, 150).await;
        clear_backoff(&shop).await;
        let report = run(&shop).await;
        assert_eq!(report.orders_expired, 1, "{report:?}");
        assert_eq!(order_status(&shop).await, "expired");
    }
}

db_test! {
    async fn an_order_that_never_reached_the_provider_expires_without_asking_it(pool: PgPool) {
        let shop = shop(pool).await;
        // A second buyer whose checkout never got an answer from the provider.
        shop.provider.inject(Op::CreateOrder, Fault::Fail(ProviderError::Timeout));
        let other = signup_verified_as(&shop.app, "other@hooks.in", "student").await;
        let res = shop.app.post(&format!("/v1/checkout/{}", shop.course)).jar(&other).send().await;
        assert_eq!(res.status, StatusCode::SERVICE_UNAVAILABLE);
        sqlx::query(
            "update orders set created_at = now() - interval '40 minutes', \
                expires_at = now() - interval '10 minutes' where provider_order_id is null",
        )
        .execute(&shop.pool)
        .await
        .unwrap();
        let before = shop.provider.calls(Op::FetchOrder);

        let report = run(&shop).await;

        assert_eq!(report.orders_expired, 1, "{report:?}");
        assert_eq!(shop.provider.calls(Op::FetchOrder), before);
        assert_eq!(
            n(&shop, "select count(*) from orders where status='expired' and provider_order_id is null").await,
            1
        );
    }
}

// ---- when the provider is the problem ------------------------------------------------------------

db_test! {
    async fn provider_errors_back_off_and_recover(pool: PgPool) {
        let shop = shop(pool).await;
        customer_pays(&shop);
        age_order(&shop, 20).await;
        shop.provider.inject(Op::FetchOrder, Fault::Fail(ProviderError::Timeout));

        let report = run(&shop).await;
        assert_eq!((report.orders_checked, report.order_errors), (1, 1), "{report:?}");
        assert_eq!(order_status(&shop).await, "pending");
        let (failures, backoff_secs): (i32, f64) = sqlx::query_as(
            "select reconcile_failures, extract(epoch from reconcile_after - now())::float8 from orders",
        )
        .fetch_one(&shop.pool)
        .await
        .unwrap();
        assert_eq!(failures, 1);
        assert!((44.0..=76.0).contains(&backoff_secs), "about a minute, got {backoff_secs}");

        // Not asked again until the backoff is over.
        assert_eq!(run(&shop).await, Report::default());
        assert_eq!(shop.provider.calls(Op::FetchOrder), 1);

        // Then it works, and the failure count starts over.
        clear_backoff(&shop).await;
        let report = run(&shop).await;
        assert_eq!(report.orders_settled, 1, "{report:?}");
        assert_eq!(order_status(&shop).await, "paid");
        assert_eq!(n(&shop, "select reconcile_failures::bigint from orders").await, 0);
    }
}

db_test! {
    async fn a_provider_that_fails_while_listing_payments_is_retried(pool: PgPool) {
        let shop = shop(pool).await;
        customer_pays(&shop);
        age_order(&shop, 20).await;
        shop.provider.inject(Op::FetchOrderPayments, Fault::Fail(ProviderError::RateLimited));
        assert_eq!(run(&shop).await.order_errors, 1);
        assert_eq!(order_status(&shop).await, "pending");
        clear_backoff(&shop).await;
        assert_eq!(run(&shop).await.orders_settled, 1);
        assert_eq!(order_status(&shop).await, "paid");
    }
}

/// The shop's order plus four more, all unpaid and due for a first look.
async fn five_due_orders(shop: &Shop) {
    for i in 0..4 {
        let jar = signup_verified_as(&shop.app, &format!("extra{i}@hooks.in"), "student").await;
        shop.app.settle().await;
        let res = shop
            .app
            .post(&format!("/v1/checkout/{}", shop.course))
            .jar(&jar)
            .send()
            .await;
        assert_eq!(res.status, StatusCode::CREATED, "{:?}", res.body);
    }
    sqlx::query(
        "update orders set created_at = now() - interval '20 minutes', \
            expires_at = now() + interval '10 minutes'",
    )
    .execute(&shop.pool)
    .await
    .unwrap();
}

db_test! {
    async fn two_instances_running_at_once_check_each_order_once(pool: PgPool) {
        let shop = shop(pool).await;
        five_due_orders(&shop).await;

        let (a, b) = join(run_once(&shop.app.state), run_once(&shop.app.state)).await;
        let (a, b) = (a.unwrap(), b.unwrap());

        assert_eq!(a.orders_checked + b.orders_checked, 5, "{a:?} {b:?}");
        assert_eq!(shop.provider.calls(Op::FetchOrder), 5);
    }
}

db_test! {
    async fn an_instance_starting_while_another_is_midway_leaves_its_orders_alone(pool: PgPool) {
        let shop = shop(pool).await;
        five_due_orders(&shop).await;
        // The first instance is stuck on a slow provider call, holding five claimed orders.
        shop.provider.inject(Op::FetchOrder, Fault::Delay(Duration::from_millis(1500)));
        let first = tokio::spawn({
            let state = shop.app.state.clone();
            async move { run_once(&state).await }
        });
        for _ in 0..200 {
            if shop.provider.calls(Op::FetchOrder) >= 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(shop.provider.calls(Op::FetchOrder), 1, "the first instance is mid-pass");

        // A second instance starts now. Everything is already claimed, so it has nothing to do.
        let second = run_once(&shop.app.state).await.unwrap();
        assert_eq!(second, Report::default(), "{second:?}");

        let first = first.await.unwrap().unwrap();
        assert_eq!(first.orders_checked, 5);
        assert_eq!(shop.provider.calls(Op::FetchOrder), 5, "no order was asked about twice");
    }
}

// ---- refunds that were left hanging ------------------------------------------------------------------

db_test! {
    async fn a_refund_the_provider_never_answered_is_asked_again_and_made_once(pool: PgPool) {
        let (shop, _) = paid_for_real(pool).await;
        shop.provider.inject(Op::Refund, Fault::FailAfter(ProviderError::Timeout));
        let res = shop
            .app
            .post(&format!("/v1/orders/{}/refund", shop.order))
            .jar(&shop.creator)
            .json(json!({}))
            .send()
            .await;
        assert_eq!(res.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(refund_status_of(&shop).await, "processing");
        // Too fresh to touch: the request that made it may still be on its way.
        assert_eq!(run(&shop).await.refunds_checked, 0);

        age_refund(&shop, 3).await;
        let report = run(&shop).await;

        assert_eq!((report.refunds_checked, report.refunds_asked_again), (1, 1), "{report:?}");
        assert_eq!(shop.provider.refunds().len(), 1, "the same idempotency key: one refund");
        assert_eq!(refund_status_of(&shop).await, "succeeded");
        assert_eq!(order_status(&shop).await, "refunded");
        assert_eq!(access(&shop).await, "access_revoked");
    }
}

db_test! {
    async fn a_pending_refund_that_finished_without_a_webhook_is_settled(pool: PgPool) {
        let (shop, _) = paid_for_real(pool).await;
        shop.provider.hold_refunds(true);
        let res = shop
            .app
            .post(&format!("/v1/orders/{}/refund", shop.order))
            .jar(&shop.creator)
            .json(json!({}))
            .send()
            .await;
        assert_eq!(res.status, StatusCode::ACCEPTED);
        let provider_refund = shop.provider.refunds().pop().unwrap();
        shop.provider.settle_refund(&provider_refund.id).unwrap();

        age_refund(&shop, 5).await;
        assert_eq!(run(&shop).await.refunds_checked, 0, "given ten minutes for the webhook first");
        age_refund(&shop, 11).await;
        let report = run(&shop).await;

        assert_eq!((report.refunds_checked, report.refunds_settled), (1, 1), "{report:?}");
        assert_eq!(refund_status_of(&shop).await, "succeeded");
        assert_eq!(order_status(&shop).await, "refunded");
        assert_eq!(access(&shop).await, "access_revoked");
    }
}

db_test! {
    async fn a_refund_still_pending_at_the_provider_is_checked_again_later(pool: PgPool) {
        let (shop, _) = paid_for_real(pool).await;
        shop.provider.hold_refunds(true);
        shop.app
            .post(&format!("/v1/orders/{}/refund", shop.order))
            .jar(&shop.creator)
            .json(json!({}))
            .send()
            .await;
        age_refund(&shop, 11).await;

        let report = run(&shop).await;
        assert_eq!((report.refunds_checked, report.refunds_settled), (1, 0), "{report:?}");
        assert_eq!(refund_status_of(&shop).await, "processing");
        assert_eq!(order_status(&shop).await, "paid");
        // Half an hour before the next look.
        assert_eq!(run(&shop).await, Report::default());
        assert_eq!(shop.provider.calls(Op::FetchRefund), 1);
    }
}

db_test! {
    async fn a_refund_the_provider_gave_up_on_is_marked_failed_and_the_order_stays_paid(pool: PgPool) {
        let (shop, _) = paid_for_real(pool).await;
        shop.provider.hold_refunds(true);
        shop.app
            .post(&format!("/v1/orders/{}/refund", shop.order))
            .jar(&shop.creator)
            .json(json!({}))
            .send()
            .await;
        let provider_refund = shop.provider.refunds().pop().unwrap();
        shop.provider.fail_refund(&provider_refund.id).unwrap();
        age_refund(&shop, 11).await;

        let report = run(&shop).await;

        assert_eq!(report.refunds_failed, 1, "{report:?}");
        assert_eq!(refund_status_of(&shop).await, "failed");
        assert!(text(&shop, "select decision_note from refunds").await.contains("could not complete"));
        assert_eq!(order_status(&shop).await, "paid");
        assert_eq!(access(&shop).await, "allowed");
    }
}

// ---- the timer ---------------------------------------------------------------------------------------

db_test! {
    async fn the_timer_runs_passes_on_its_own(pool: PgPool) {
        let shop = shop(pool).await;
        customer_pays(&shop);
        age_order(&shop, 20).await;
        zeno_api::reconcile::spawn_periodic(shop.app.state.clone(), Duration::from_millis(50));

        let mut paid = false;
        for _ in 0..100 {
            if order_status(&shop).await == "paid" {
                paid = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(paid, "the periodic job never settled the order");
        assert_eq!(n(&shop, "select count(*) from enrollments").await, 1);
    }
}
