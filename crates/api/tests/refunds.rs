//! Refunds: asked for inside the window the course promised, decided by whoever runs the course,
//! made by the provider, and final only when the provider says so.

mod common;

use axum::http::StatusCode;
use common::shop::*;
use common::*;
use futures_util::future::join_all;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;
use zeno_core::payments::{Fault, Op, ProviderError};

macro_rules! db_test {
    ($(#[$meta:meta])* async fn $name:ident($pool:ident: PgPool) $body:block) => {
        #[sqlx::test(migrator = "zeno_core::db::MIGRATOR")]
        $(#[$meta])*
        async fn $name($pool: PgPool) $body
    };
}

/// A shop whose buyer has really paid: the money exists at the provider, and our books know it.
async fn paid(pool: PgPool) -> (Shop, String) {
    let shop = shop(pool).await;
    let payment = shop.provider.capture(&shop.remote).unwrap();
    let res = deliver(
        &shop,
        "evt_paid",
        captured(&payment.id, &shop.remote, PRICE),
    )
    .await;
    assert_eq!(outcome(&res), "enrolled", "{:?}", res.body);
    (shop, payment.id)
}

async fn ask(shop: &Shop, reason: Option<&str>) -> Res {
    let body = match reason {
        Some(reason) => json!({"reason": reason}),
        None => json!({}),
    };
    shop.app
        .post(&format!("/v1/orders/{}/request-refund", shop.order))
        .jar(&shop.buyer)
        .json(body)
        .send()
        .await
}

async fn approve(shop: &Shop, who: &Jar) -> Res {
    shop.app
        .post(&format!("/v1/orders/{}/refund", shop.order))
        .jar(who)
        .json(json!({}))
        .send()
        .await
}

async fn decline(shop: &Shop, who: &Jar, reason: Option<&str>) -> Res {
    let body = match reason {
        Some(reason) => json!({"reason": reason}),
        None => json!({}),
    };
    shop.app
        .post(&format!("/v1/orders/{}/refund/decline", shop.order))
        .jar(who)
        .json(body)
        .send()
        .await
}

fn refund_id(res: &Res) -> Uuid {
    Uuid::parse_str(res.body["refund"]["id"].as_str().unwrap()).unwrap()
}

fn refund_status(res: &Res) -> &str {
    res.body["refund"]["status"].as_str().unwrap_or("")
}

async fn latest_refund_status(shop: &Shop) -> String {
    text(
        shop,
        "select status::text from refunds order by requested_at desc, id desc limit 1",
    )
    .await
}

async fn paid_days_ago(shop: &Shop, days: f64) {
    sqlx::query("update orders set paid_at = now() - make_interval(secs => $2) where id = $1")
        .bind(shop.order)
        .bind(days * 86_400.0)
        .execute(&shop.pool)
        .await
        .unwrap();
}

async fn admin_jar(shop: &Shop) -> Jar {
    signup_creator(&shop.app, "admin@refunds.in").await;
    sqlx::query("update users set role = 'admin' where email = 'admin@refunds.in'")
        .execute(&shop.pool)
        .await
        .unwrap();
    Jar::of(&login(&shop.app, "admin@refunds.in", PASSWORD).await)
}

// ---- asking -------------------------------------------------------------------------------------

db_test! {
    async fn a_student_can_ask_inside_the_window_and_the_creator_is_told(pool: PgPool) {
        let (shop, _) = paid(pool).await;
        // The buyer's receipt is delivered in the background; let it land before counting.
        shop.app.settle().await;
        let mails = shop.app.mailer.sent().len();

        let res = ask(&shop, Some("The course is not what I expected")).await;

        assert_eq!(res.status, StatusCode::CREATED, "{:?}", res.body);
        assert_eq!(refund_status(&res), "requested");
        assert_eq!(res.body["refund"]["amount_paise"], PRICE);
        assert_eq!(res.body["refund"]["origin"], "student");
        assert_eq!(res.body["refund"]["reason"], "The course is not what I expected");
        // Asking moves no money and takes nothing away.
        assert_eq!(order_status(&shop).await, "paid");
        assert_eq!(access(&shop).await, "allowed");
        assert_eq!(shop.provider.refunds().len(), 0);
        let mail = shop.app.mailer.wait_for(mails + 1).await.pop().unwrap();
        assert_eq!(mail.to, "teacher@hooks.in");
        assert!(mail.subject.contains("Refund requested"), "{}", mail.subject);
        assert!(mail.text.contains("not what I expected"));

        // Asking again returns the same request.
        let again = ask(&shop, Some("Please")).await;
        assert_eq!(again.status, StatusCode::OK);
        assert_eq!(refund_id(&again), refund_id(&res));
        assert_eq!(n(&shop, "select count(*) from refunds").await, 1);
    }
}

db_test! {
    async fn the_order_shows_the_buyer_when_they_can_still_ask_and_what_became_of_it(pool: PgPool) {
        let (shop, _) = paid(pool).await;
        let before = shop.app.get(&format!("/v1/orders/{}", shop.order)).jar(&shop.buyer).send().await;
        assert!(before.body["order"]["paid_at"].is_string());
        assert!(before.body["order"]["refundable_until"].is_string());
        assert!(before.body["refund"].is_null());
        ask(&shop, None).await;
        let after = shop.app.get(&format!("/v1/orders/{}", shop.order)).jar(&shop.buyer).send().await;
        assert_eq!(after.body["refund"]["status"], "requested");
    }
}

db_test! {
    async fn the_window_is_the_one_promised_at_purchase_whatever_the_course_says_now(pool: PgPool) {
        let (shop, _) = paid(pool).await;
        // The creator now offers no refunds at all. The order was made under 7 days.
        let version: i64 = sqlx::query_scalar("select version::bigint from courses where id=$1")
            .bind(shop.course)
            .fetch_one(&shop.pool)
            .await
            .unwrap();
        let patched = shop
            .app
            .req(axum::http::Method::PATCH, &format!("/v1/courses/{}", shop.course))
            .jar(&shop.creator)
            .json(json!({"version": version, "refund_window_days": 0}))
            .send()
            .await;
        assert_eq!(patched.status, StatusCode::OK, "{:?}", patched.body);
        assert_eq!(patched.body["course"]["refund_window_days"], 0);
        assert_eq!(ask(&shop, None).await.status, StatusCode::CREATED);
        shop.app.settle().await;

        // A later buyer, who pays under the new policy, gets none.
        let late = signup_verified_as(&shop.app, "late@hooks.in", "student").await;
        let started = shop.app.post(&format!("/v1/checkout/{}", shop.course)).jar(&late).send().await;
        assert_eq!(started.status, StatusCode::CREATED);
        let order = Uuid::parse_str(started.body["order"]["id"].as_str().unwrap()).unwrap();
        let remote = started.body["checkout"]["provider_order_id"].as_str().unwrap().to_owned();
        let payment = shop.provider.capture(&remote).unwrap();
        deliver(&shop, "evt_late", captured(&payment.id, &remote, PRICE)).await;
        let res = shop
            .app
            .post(&format!("/v1/orders/{order}/request-refund"))
            .jar(&late)
            .json(json!({}))
            .send()
            .await;
        assert_eq!((res.status, res.code()), (StatusCode::CONFLICT, "refund_not_available"));
        let read = shop.app.get(&format!("/v1/orders/{order}")).jar(&late).send().await;
        assert!(read.body["order"]["refundable_until"].is_null());
    }
}

db_test! {
    async fn after_the_window_closes_a_student_can_no_longer_ask(pool: PgPool) {
        let (shop, _) = paid(pool).await;
        paid_days_ago(&shop, 6.9).await;
        assert_eq!(ask(&shop, None).await.status, StatusCode::CREATED);
        decline(&shop, &shop.creator, Some("Changed my mind about declining")).await;
        paid_days_ago(&shop, 7.1).await;
        let res = ask(&shop, None).await;
        assert_eq!((res.status, res.code()), (StatusCode::CONFLICT, "refund_window_closed"));
    }
}

db_test! {
    async fn only_a_paid_order_of_your_own_can_be_asked_about(pool: PgPool) {
        let shop = shop(pool).await;
        // Not paid yet.
        let res = ask(&shop, None).await;
        assert_eq!((res.status, res.code()), (StatusCode::CONFLICT, "not_refundable"));
        // Someone else's order, and no order at all.
        let other = signup_verified_as(&shop.app, "other@hooks.in", "student").await;
        let theirs = shop
            .app
            .post(&format!("/v1/orders/{}/request-refund", shop.order))
            .jar(&other)
            .json(json!({}))
            .send()
            .await;
        assert_eq!(theirs.status, StatusCode::NOT_FOUND);
        let missing = shop
            .app
            .post(&format!("/v1/orders/{}/request-refund", Uuid::new_v4()))
            .jar(&shop.buyer)
            .json(json!({}))
            .send()
            .await;
        assert_eq!(missing.status, StatusCode::NOT_FOUND);
        let anonymous = shop
            .app
            .post(&format!("/v1/orders/{}/request-refund", shop.order))
            .json(json!({}))
            .send()
            .await;
        assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    }
}

db_test! {
    async fn notes_must_be_short_and_plain(pool: PgPool) {
        let (shop, _) = paid(pool).await;
        for reason in ["x".repeat(301), "two\nlines".to_owned()] {
            let res = ask(&shop, Some(&reason)).await;
            assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{reason:?}");
        }
        let unknown = shop
            .app
            .post(&format!("/v1/orders/{}/request-refund", shop.order))
            .jar(&shop.buyer)
            .json(json!({"amount_paise": 1}))
            .send()
            .await;
        assert!(unknown.status.is_client_error(), "no field names an amount");
        assert_eq!(n(&shop, "select count(*) from refunds").await, 0);
    }
}

// ---- deciding -----------------------------------------------------------------------------------

db_test! {
    async fn approving_a_request_refunds_the_order_and_ends_access(pool: PgPool) {
        let (shop, payment) = paid(pool).await;
        let asked = ask(&shop, Some("Too advanced")).await;
        shop.app.settle().await;
        let mails = shop.app.mailer.sent().len();

        let res = approve(&shop, &shop.creator).await;

        assert_eq!(res.status, StatusCode::ACCEPTED, "{:?}", res.body);
        assert_eq!(refund_id(&res), refund_id(&asked), "the request itself was approved");
        // The fake provider completes refunds at once, so the books are already settled.
        assert_eq!(refund_status(&res), "succeeded");
        assert_eq!(order_status(&shop).await, "refunded");
        assert_eq!(text(&shop, "select status::text from payments").await, "refunded");
        assert_eq!(access(&shop).await, "access_revoked");
        let refunds = shop.provider.refunds();
        assert_eq!(refunds.len(), 1);
        assert_eq!((refunds[0].payment_id.as_str(), refunds[0].amount_paise), (payment.as_str(), PRICE));
        // The buyer hears twice: approved, then done.
        let mail = shop.app.mailer.wait_for(mails + 2).await;
        let subjects: Vec<&str> = mail[mails..].iter().map(|m| m.subject.as_str()).collect();
        assert!(subjects.iter().any(|s| s.contains("on its way")), "{subjects:?}");
        assert!(subjects.iter().any(|s| s.contains("is complete")), "{subjects:?}");
        assert!(mail[mails..].iter().all(|m| m.to == "buyer@hooks.in"));
    }
}

db_test! {
    async fn a_creator_can_refund_without_being_asked_and_an_admin_can_too(pool: PgPool) {
        let (shop, _) = paid(pool).await;
        let res = approve(&shop, &shop.creator).await;
        assert_eq!(res.status, StatusCode::ACCEPTED, "{:?}", res.body);
        assert_eq!(res.body["refund"]["origin"], "creator");
        shop.app.settle().await;

        // An admin on another order.
        let shop2 = second_buyer(&shop).await;
        shop.app.settle().await;
        let admin = admin_jar(&shop).await;
        let res = shop2
            .app
            .post(&format!("/v1/orders/{}/refund", shop2.order))
            .jar(&admin)
            .json(json!({"reason": "Goodwill"}))
            .send()
            .await;
        assert_eq!(res.status, StatusCode::ACCEPTED, "{:?}", res.body);
        assert_eq!(res.body["refund"]["origin"], "admin");
        assert_eq!(res.body["refund"]["reason"], "Goodwill");
    }
}

/// A second paid order for the same course, bought by another student.
async fn second_buyer(shop: &Shop) -> Shop {
    let buyer = signup_verified_as(&shop.app, "buyer2@hooks.in", "student").await;
    let started = shop
        .app
        .post(&format!("/v1/checkout/{}", shop.course))
        .jar(&buyer)
        .send()
        .await;
    assert_eq!(started.status, StatusCode::CREATED, "{:?}", started.body);
    let remote = started.body["checkout"]["provider_order_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let payment = shop.provider.capture(&remote).unwrap();
    deliver(shop, "evt_second", captured(&payment.id, &remote, PRICE)).await;
    Shop {
        app: App {
            router: shop.app.router.clone(),
            mailer: shop.app.mailer.clone(),
            pool: shop.app.pool.clone(),
            state: shop.app.state.clone(),
        },
        provider: shop.provider.clone(),
        pool: shop.pool.clone(),
        creator: shop.creator.clone(),
        buyer: buyer.clone(),
        buyer_id: sqlx::query_scalar("select id from users where email='buyer2@hooks.in'")
            .fetch_one(&shop.pool)
            .await
            .unwrap(),
        creator_id: shop.creator_id,
        course: shop.course,
        video: shop.video,
        order: Uuid::parse_str(started.body["order"]["id"].as_str().unwrap()).unwrap(),
        remote,
    }
}

db_test! {
    async fn a_refund_is_final_only_when_the_provider_confirms_it(pool: PgPool) {
        let (shop, payment) = paid(pool).await;
        shop.provider.hold_refunds(true);

        let res = approve(&shop, &shop.creator).await;

        assert_eq!(res.status, StatusCode::ACCEPTED, "{:?}", res.body);
        assert_eq!(refund_status(&res), "processing");
        // The provider has it, but nothing is final: the order is paid and the buyer still in.
        assert_eq!(order_status(&shop).await, "paid");
        assert_eq!(access(&shop).await, "allowed");
        let provider_refund = shop.provider.refunds().pop().unwrap();
        assert_eq!(
            text(&shop, "select provider_refund_id from refunds").await,
            provider_refund.id
        );

        // The bank finishes; the provider tells us.
        shop.provider.settle_refund(&provider_refund.id).unwrap();
        let done = deliver(&shop, "evt_refund", refund_processed(&provider_refund.id, &payment, PRICE, None)).await;
        assert_eq!(outcome(&done), "refund_applied");
        assert_eq!(order_status(&shop).await, "refunded");
        assert_eq!(access(&shop).await, "access_revoked");
        assert_eq!(latest_refund_status(&shop).await, "succeeded");
        assert_eq!(n(&shop, "select count(*) from refunds").await, 1);

        // The same event again is a duplicate; the same refund under a new event id is recognised.
        let again = deliver(&shop, "evt_refund_again", refund_processed(&provider_refund.id, &payment, PRICE, None)).await;
        assert_eq!(outcome(&again), "already_refunded");
        assert_eq!(n(&shop, "select count(*) from enrollment_events where action='revoked'").await, 1);
    }
}

db_test! {
    async fn asking_the_provider_twice_refunds_once(pool: PgPool) {
        let (shop, _) = paid(pool).await;
        shop.provider.hold_refunds(true);
        let first = approve(&shop, &shop.creator).await;
        let second = approve(&shop, &shop.creator).await;
        assert_eq!((first.status, second.status), (StatusCode::ACCEPTED, StatusCode::ACCEPTED));
        assert_eq!(refund_id(&first), refund_id(&second));
        assert_eq!(shop.provider.refunds().len(), 1, "same idempotency key, one refund");
        assert_eq!(n(&shop, "select count(*) from refunds").await, 1);
    }
}

db_test! {
    async fn simultaneous_approvals_make_one_refund(pool: PgPool) {
        let (shop, _) = paid(pool).await;
        shop.provider.hold_refunds(true);
        let results = join_all((0..5).map(|_| approve(&shop, &shop.creator))).await;
        assert!(results.iter().all(|r| r.status == StatusCode::ACCEPTED), "{:?}", results.iter().map(|r| r.status).collect::<Vec<_>>());
        let first = refund_id(&results[0]);
        assert!(results.iter().all(|r| refund_id(r) == first));
        assert_eq!(n(&shop, "select count(*) from refunds").await, 1);
        assert_eq!(shop.provider.refunds().len(), 1);
    }
}

db_test! {
    async fn a_provider_timeout_leaves_the_refund_to_be_asked_again_without_refunding_twice(pool: PgPool) {
        let (shop, _) = paid(pool).await;
        // The provider made the refund, but its answer never reached us.
        shop.provider.inject(Op::Refund, Fault::FailAfter(ProviderError::Timeout));
        let first = approve(&shop, &shop.creator).await;
        assert_eq!(first.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(latest_refund_status(&shop).await, "processing");
        assert_eq!(shop.provider.refunds().len(), 1);
        assert_eq!(order_status(&shop).await, "paid", "not final until the provider says so");

        // Asking again reaches the refund that already exists.
        let retry = approve(&shop, &shop.creator).await;
        assert_eq!(retry.status, StatusCode::ACCEPTED, "{:?}", retry.body);
        assert_eq!(shop.provider.refunds().len(), 1, "no second refund");
        assert_eq!(refund_status(&retry), "succeeded");
        assert_eq!(order_status(&shop).await, "refunded");
    }
}

db_test! {
    async fn a_refund_the_provider_refuses_says_why_and_leaves_the_order_paid(pool: PgPool) {
        let (shop, _) = paid(pool).await;
        shop.provider.inject(
            Op::Refund,
            Fault::Fail(ProviderError::Rejected {
                code: "BAD_REQUEST_ERROR".into(),
                description: "internal bank detail".into(),
            }),
        );
        let res = approve(&shop, &shop.creator).await;
        assert_eq!((res.status, res.code()), (StatusCode::BAD_GATEWAY, "refund_failed"));
        assert!(!res.body.to_string().contains("internal bank detail"));
        assert_eq!(order_status(&shop).await, "paid");
        assert_eq!(access(&shop).await, "allowed");
        assert_eq!(latest_refund_status(&shop).await, "failed");
        let note = text(&shop, "select decision_note from refunds").await;
        assert!(note.contains("BAD_REQUEST_ERROR") && !note.contains("internal bank detail"), "{note}");

        // The creator can try again once it is sorted out.
        let retry = approve(&shop, &shop.creator).await;
        assert_eq!(retry.status, StatusCode::ACCEPTED, "{:?}", retry.body);
        assert_eq!(n(&shop, "select count(*) from refunds").await, 2);
        assert_eq!(order_status(&shop).await, "refunded");
    }
}

db_test! {
    async fn a_refund_the_provider_reports_as_failed_is_marked_failed(pool: PgPool) {
        let (shop, payment) = paid(pool).await;
        shop.provider.hold_refunds(true);
        approve(&shop, &shop.creator).await;
        let provider_refund = shop.provider.refunds().pop().unwrap();
        shop.provider.fail_refund(&provider_refund.id).unwrap();
        let res = deliver(&shop, "evt_failed", refund_failed(&provider_refund.id, &payment, PRICE)).await;
        assert_eq!(outcome(&res), "refund_failed");
        assert_eq!(latest_refund_status(&shop).await, "failed");
        assert_eq!(order_status(&shop).await, "paid");
        assert_eq!(access(&shop).await, "allowed");
        assert_eq!(n(&shop, "select count(*) from webhook_events where needs_review").await, 1);
    }
}

db_test! {
    async fn declining_needs_a_reason_and_a_waiting_request(pool: PgPool) {
        let (shop, _) = paid(pool).await;
        // Nothing to decline yet.
        let none = decline(&shop, &shop.creator, Some("No")).await;
        assert_eq!((none.status, none.code()), (StatusCode::CONFLICT, "no_refund_request"));
        let asked = ask(&shop, None).await;
        shop.app.settle().await;
        let mails = shop.app.mailer.sent().len();
        // A reason is required.
        assert_eq!(decline(&shop, &shop.creator, None).await.status, StatusCode::UNPROCESSABLE_ENTITY);

        let res = decline(&shop, &shop.creator, Some("You have watched most of it")).await;

        assert_eq!(res.status, StatusCode::OK, "{:?}", res.body);
        assert_eq!(refund_id(&res), refund_id(&asked));
        assert_eq!(refund_status(&res), "declined");
        assert_eq!(res.body["refund"]["decision_note"], "You have watched most of it");
        assert_eq!(order_status(&shop).await, "paid");
        assert_eq!(access(&shop).await, "allowed");
        let mail = shop.app.mailer.wait_for(mails + 1).await.pop().unwrap();
        assert_eq!(mail.to, "buyer@hooks.in");
        assert!(mail.text.contains("You have watched most of it"));
        // A declined request is over; the student may ask again while the window lasts.
        let again = ask(&shop, Some("Please reconsider")).await;
        assert_eq!(again.status, StatusCode::CREATED);
        assert_ne!(refund_id(&again), refund_id(&asked));
    }
}

db_test! {
    async fn nothing_can_be_refunded_twice(pool: PgPool) {
        let (shop, _) = paid(pool).await;
        assert_eq!(approve(&shop, &shop.creator).await.status, StatusCode::ACCEPTED);
        let again = approve(&shop, &shop.creator).await;
        assert_eq!((again.status, again.code()), (StatusCode::CONFLICT, "already_refunded"));
        let asked = ask(&shop, None).await;
        assert_eq!((asked.status, asked.code()), (StatusCode::CONFLICT, "already_refunded"));
        assert_eq!(shop.provider.refunds().len(), 1);
    }
}

db_test! {
    async fn an_order_that_was_never_paid_cannot_be_refunded(pool: PgPool) {
        let shop = shop(pool).await;
        let res = approve(&shop, &shop.creator).await;
        assert_eq!((res.status, res.code()), (StatusCode::CONFLICT, "not_refundable"));
        assert_eq!(shop.provider.calls(Op::Refund), 0);
    }
}

db_test! {
    async fn refunding_after_access_already_ended_still_returns_the_money(pool: PgPool) {
        let (shop, _) = paid(pool).await;
        // The creator removed the student's access; the student still paid.
        sqlx::query(
            "update enrollments set revoked_at = now(), revoke_reason = 'misuse', \
                revoked_by = $1, revocation_kind = 'creator'",
        )
        .bind(shop.creator_id)
        .execute(&shop.pool)
        .await
        .unwrap();
        let res = approve(&shop, &shop.creator).await;
        assert_eq!(res.status, StatusCode::ACCEPTED, "{:?}", res.body);
        assert_eq!(order_status(&shop).await, "refunded");
    }
}

// ---- who may decide ---------------------------------------------------------------------------

db_test! {
    async fn only_the_courses_creator_or_an_admin_decides(pool: PgPool) {
        let (shop, _) = paid(pool).await;
        ask(&shop, None).await;
        shop.app.settle().await;
        let other_creator = signup_creator(&shop.app, "rival@hooks.in").await;
        let url = format!("/v1/orders/{}/refund", shop.order);
        let post = |jar: &Jar| shop.app.post(&url).jar(jar).json(json!({})).send();

        // A student, even the buyer: they ask, they do not decide.
        assert_eq!(post(&shop.buyer).await.status, StatusCode::FORBIDDEN);
        // Another creator is told the order does not exist.
        assert_eq!(post(&other_creator).await.status, StatusCode::NOT_FOUND);
        let declined = shop
            .app
            .post(&format!("/v1/orders/{}/refund/decline", shop.order))
            .jar(&other_creator)
            .json(json!({"reason": "no"}))
            .send()
            .await;
        assert_eq!(declined.status, StatusCode::NOT_FOUND);
        assert_eq!(shop.app.post(&url).json(json!({})).send().await.status, StatusCode::UNAUTHORIZED);
        assert_eq!(shop.provider.calls(Op::Refund), 0);
        assert_eq!(latest_refund_status(&shop).await, "requested");
        // The creator can.
        assert_eq!(post(&shop.creator).await.status, StatusCode::ACCEPTED);
    }
}

db_test! {
    async fn creators_see_the_refunds_of_their_own_course_only(pool: PgPool) {
        let (shop, _) = paid(pool).await;
        ask(&shop, Some("Too slow")).await;
        shop.app.settle().await;
        let url = format!("/v1/courses/{}/refunds", shop.course);

        let mine = shop.app.get(&url).jar(&shop.creator).send().await;
        assert_eq!(mine.status, StatusCode::OK, "{:?}", mine.body);
        let refunds = mine.body["refunds"].as_array().unwrap();
        assert_eq!(refunds.len(), 1);
        assert_eq!(refunds[0]["status"], "requested");
        assert_eq!(refunds[0]["reason"], "Too slow");
        assert_eq!(refunds[0]["buyer_name"], "Asha Rao");
        assert_eq!(refunds[0]["order_id"], shop.order.to_string());
        assert!(refunds[0].get("provider_refund_id").is_none(), "provider ids stay private");

        let filtered = shop.app.get(&format!("{url}?status=succeeded")).jar(&shop.creator).send().await;
        assert_eq!(filtered.body["refunds"].as_array().unwrap().len(), 0);
        assert_eq!(
            shop.app.get(&format!("{url}?status=bogus")).jar(&shop.creator).send().await.status,
            StatusCode::UNPROCESSABLE_ENTITY
        );
        let rival = signup_creator(&shop.app, "rival@hooks.in").await;
        assert_eq!(shop.app.get(&url).jar(&rival).send().await.status, StatusCode::NOT_FOUND);
        assert_eq!(shop.app.get(&url).jar(&shop.buyer).send().await.status, StatusCode::FORBIDDEN);
        assert_eq!(shop.app.get(&url).send().await.status, StatusCode::UNAUTHORIZED);
        let admin = admin_jar(&shop).await;
        assert_eq!(shop.app.get(&url).jar(&admin).send().await.status, StatusCode::OK);
    }
}

// ---- refunds made outside Zeno -------------------------------------------------------------------

db_test! {
    async fn a_refund_made_in_the_providers_dashboard_is_recorded_and_ends_access(pool: PgPool) {
        let (shop, payment) = paid(pool).await;
        let res = deliver(&shop, "evt_dash", refund_processed("rfnd_dash", &payment, PRICE, None)).await;
        assert_eq!(outcome(&res), "refund_applied");
        assert_eq!(order_status(&shop).await, "refunded");
        assert_eq!(access(&shop).await, "access_revoked");
        let (origin, status, provider_id): (String, String, String) = sqlx::query_as(
            "select origin, status::text, provider_refund_id from refunds",
        )
        .fetch_one(&shop.pool)
        .await
        .unwrap();
        assert_eq!((origin.as_str(), status.as_str(), provider_id.as_str()), ("provider", "succeeded", "rfnd_dash"));
    }
}

db_test! {
    async fn partial_refunds_add_up_and_the_last_one_ends_access(pool: PgPool) {
        let (shop, payment) = paid(pool).await;
        let first = deliver(&shop, "evt_1", refund_processed("rfnd_a", &payment, 20_000, None)).await;
        assert_eq!(outcome(&first), "refund_partial");
        assert_eq!(access(&shop).await, "allowed");
        assert_eq!(order_status(&shop).await, "paid");
        assert_eq!(n(&shop, "select count(*) from webhook_events where needs_review").await, 1);

        // What is left can still be refunded by the creator, and only what is left.
        shop.provider.hold_refunds(true);
        let res = approve(&shop, &shop.creator).await;
        assert_eq!(res.status, StatusCode::ACCEPTED, "{:?}", res.body);
        assert_eq!(res.body["refund"]["amount_paise"], PRICE - 20_000);

        let second = deliver(&shop, "evt_2", refund_processed("rfnd_b", &payment, PRICE - 20_000, None)).await;
        assert_eq!(outcome(&second), "refund_applied");
        assert_eq!(access(&shop).await, "access_revoked");
        assert_eq!(order_status(&shop).await, "refunded");
        assert_eq!(n(&shop, "select coalesce(sum(amount_paise),0)::bigint from refunds where status='succeeded'").await, PRICE);
    }
}

db_test! {
    async fn a_provider_partial_result_for_our_full_request_counts_only_money_actually_returned(pool: PgPool) {
        let (shop, payment) = paid(pool).await;
        shop.provider.hold_refunds(true);
        let started = approve(&shop, &shop.creator).await;
        assert_eq!(started.status, StatusCode::ACCEPTED);
        let remote = shop.provider.refunds().pop().unwrap();

        let result = deliver(
            &shop,
            "evt_short_refund",
            refund_processed(&remote.id, &payment, 100, None),
        )
        .await;
        assert_eq!(outcome(&result), "refund_partial", "{:?}", result.body);
        assert_eq!(order_status(&shop).await, "paid");
        assert_eq!(access(&shop).await, "allowed");
        assert_eq!(text(&shop, "select status::text from refunds").await, "succeeded");
        assert_eq!(n(&shop, "select sum(amount_paise)::bigint from refunds where status='succeeded'").await, 100);
    }
}

db_test! {
    async fn a_dashboard_partial_refund_does_not_claim_our_unanswered_full_request(pool: PgPool) {
        let (shop, payment) = paid(pool).await;
        shop.provider.inject(Op::Refund, Fault::Fail(ProviderError::Timeout));
        let first = approve(&shop, &shop.creator).await;
        assert_eq!(first.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(latest_refund_status(&shop).await, "processing");

        let result = deliver(
            &shop,
            "evt_dashboard_partial",
            refund_processed("rfnd_dashboard", &payment, 100, None),
        )
        .await;
        assert_eq!(outcome(&result), "refund_partial", "{:?}", result.body);
        assert_eq!(order_status(&shop).await, "paid");
        assert_eq!(access(&shop).await, "allowed");
        assert_eq!(n(&shop, "select count(*) from refunds where status='processing'").await, 1);
        assert_eq!(n(&shop, "select sum(amount_paise)::bigint from refunds where status='succeeded'").await, 100);
        assert_eq!(text(&shop, "select origin from refunds where provider_refund_id='rfnd_dashboard'").await, "provider");
    }
}

db_test! {
    async fn a_webhook_before_the_provider_reply_uses_our_receipt_to_close_our_refund(pool: PgPool) {
        let (shop, payment) = paid(pool).await;
        shop.provider.inject(Op::Refund, Fault::FailAfter(ProviderError::Timeout));
        let started = approve(&shop, &shop.creator).await;
        assert_eq!(started.status, StatusCode::SERVICE_UNAVAILABLE);
        let local_id: Uuid = sqlx::query_scalar("select id from refunds")
            .fetch_one(&shop.pool)
            .await
            .unwrap();
        let remote = shop.provider.refunds().pop().unwrap();
        let mut entity = refund(&remote.id, &payment, PRICE, "processed");
        entity["receipt"] = json!(local_id.to_string());
        let result = deliver(&shop, "evt_before_reply", event("refund.processed", json!({"refund": {"entity": entity}}))).await;
        assert_eq!(outcome(&result), "refund_applied", "{:?}", result.body);
        assert_eq!(n(&shop, "select count(*) from refunds").await, 1);
        assert_eq!(text(&shop, "select status::text from refunds").await, "succeeded");
        assert_eq!(text(&shop, "select provider_refund_id from refunds").await, remote.id);
        assert_eq!(access(&shop).await, "access_revoked");
    }
}

db_test! {
    async fn an_unrelated_failed_dashboard_refund_does_not_fail_our_pending_request(pool: PgPool) {
        let (shop, payment) = paid(pool).await;
        shop.provider.inject(Op::Refund, Fault::Fail(ProviderError::Timeout));
        assert_eq!(approve(&shop, &shop.creator).await.status, StatusCode::SERVICE_UNAVAILABLE);
        let result = deliver(&shop, "evt_other_failed", refund_failed("rfnd_other", &payment, PRICE)).await;
        assert_eq!(outcome(&result), "refund_failed");
        assert_eq!(latest_refund_status(&shop).await, "processing");
        assert_eq!(order_status(&shop).await, "paid");
        assert_eq!(access(&shop).await, "allowed");
    }
}

// ---- disputes -----------------------------------------------------------------------------------

db_test! {
    async fn a_dispute_is_recorded_against_the_order_and_flagged_but_changes_nothing(pool: PgPool) {
        let (shop, payment) = paid(pool).await;
        let body = event(
            "payment.dispute.created",
            json!({"dispute": {"entity": {
                "id": "disp_1", "payment_id": payment, "amount": PRICE, "status": "open",
                "reason_code": "fraudulent"
            }}}),
        );
        let res = deliver(&shop, "evt_dispute", body.clone()).await;
        assert_eq!((res.status, outcome(&res)), (StatusCode::OK, "dispute_noted"));
        assert_eq!(order_status(&shop).await, "paid");
        assert_eq!(access(&shop).await, "allowed");
        assert_eq!(n(&shop, "select count(*) from webhook_events where needs_review").await, 1);
        let details: Value = sqlx::query_scalar("select details from order_events where kind='dispute'")
            .fetch_one(&shop.pool)
            .await
            .unwrap();
        assert_eq!(details["dispute"], "disp_1");
        assert_eq!(details["status"], "open");
        // Each step of a dispute is its own event and gets its own note.
        let won = event(
            "payment.dispute.won",
            json!({"dispute": {"entity": {"id": "disp_1", "payment_id": payment, "status": "won"}}}),
        );
        assert_eq!(outcome(&deliver(&shop, "evt_dispute_won", won).await), "dispute_noted");
        assert_eq!(n(&shop, "select count(*) from order_events where kind='dispute'").await, 2);
        // A dispute about a payment we do not know is flagged too.
        let stray = event(
            "payment.dispute.created",
            json!({"dispute": {"entity": {"id": "disp_9", "payment_id": "pay_unknown", "status": "open"}}}),
        );
        assert_eq!(outcome(&deliver(&shop, "evt_stray", stray).await), "dispute_noted");
        assert_eq!(n(&shop, "select count(*) from webhook_events where needs_review").await, 3);
    }
}

// ---- the knob ------------------------------------------------------------------------------------

db_test! {
    async fn creators_set_the_refund_window_within_bounds(pool: PgPool) {
        let (shop, _) = paid(pool).await;
        let patch = |days: i64| {
            let shop = &shop;
            async move {
                let version: i64 = sqlx::query_scalar("select version::bigint from courses where id=$1")
                    .bind(shop.course)
                    .fetch_one(&shop.pool)
                    .await
                    .unwrap();
                shop.app
                    .req(axum::http::Method::PATCH, &format!("/v1/courses/{}", shop.course))
                    .jar(&shop.creator)
                    .json(json!({"version": version, "refund_window_days": days}))
                    .send()
                    .await
            }
        };
        for bad in [-1, 366, 1000] {
            assert_eq!(patch(bad).await.status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
        }
        for good in [0, 1, 30, 365] {
            let res = patch(good).await;
            assert_eq!(res.status, StatusCode::OK, "{good}: {:?}", res.body);
            assert_eq!(res.body["course"]["refund_window_days"], good);
        }
        // Visitors see the policy before they buy.
        let slug: String = sqlx::query_scalar("select slug from courses where id=$1")
            .bind(shop.course)
            .fetch_one(&shop.pool)
            .await
            .unwrap();
        let page = shop.app.get(&format!("/v1/public/courses/{slug}")).send().await;
        assert_eq!(page.body["course"]["refund_window_days"], 365);
    }
}

db_test! {
    async fn the_orders_own_snapshot_holds_the_window_it_was_sold_under(pool: PgPool) {
        let shop = shop(pool).await;
        let snapshot: Value = sqlx::query_scalar("select price_snapshot from orders where id=$1")
            .bind(shop.order)
            .fetch_one(&shop.pool)
            .await
            .unwrap();
        assert_eq!(snapshot["refund_window_days"], 7);
    }
}
