//! Checkout: the server prices the order, the provider order is made exactly once, and every way
//! the provider can misbehave leaves a state the next request can recover from.

mod common;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::http::{Method, StatusCode};
use common::*;
use futures_util::future::join_all;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;
use zeno_api::auth::extractor::AuthUser;
use zeno_api::mail::MemoryMailer;
use zeno_api::orders::service;
use zeno_core::model::Role;
use zeno_core::payments::{
    CreateOrder, FakeProvider, Fault, Op, PaymentProvider, ProviderError, ProviderOrder,
    ProviderPayment, ProviderRefund, RefundRequest,
};

macro_rules! db_test {
    ($(#[$meta:meta])* async fn $name:ident($pool:ident: PgPool) $body:block) => {
        #[sqlx::test(migrator = "zeno_core::db::MIGRATOR")]
        $(#[$meta])*
        async fn $name($pool: PgPool) $body
    };
}

const PRICE: i64 = 49_900;

struct Course {
    id: Uuid,
}

async fn owner_of(pool: &PgPool, course: Uuid) -> Uuid {
    sqlx::query_scalar("select creator_id from courses where id=$1")
        .bind(course)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn user_id(pool: &PgPool, email: &str) -> Uuid {
    sqlx::query_scalar("select id from users where email=$1")
        .bind(email)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// A published course with one ready lesson, at `price_paise`.
async fn course(app: &App, pool: &PgPool, creator: &Jar, title: &str, price_paise: i64) -> Course {
    let response = app
        .post("/v1/courses")
        .jar(creator)
        .json(json!({"title": title, "description": "Course", "price_paise": price_paise}))
        .send()
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);
    let id = Uuid::parse_str(response.body["course"]["id"].as_str().unwrap()).unwrap();
    let owner = owner_of(pool, id).await;
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
    Course { id }
}

struct World {
    app: App,
    provider: FakeProvider,
    creator: Jar,
    buyer: Jar,
    course: Course,
}

async fn world(pool: PgPool) -> World {
    let (app, provider) = app_with_provider(pool.clone());
    let creator = signup_creator(&app, "teacher@checkout.in").await;
    let buyer = signup_verified_as(&app, "buyer@checkout.in", "student").await;
    let course = course(&app, &pool, &creator, "Paid course", PRICE).await;
    World {
        app,
        provider,
        creator,
        buyer,
        course,
    }
}

async fn checkout(app: &App, who: &Jar, course: Uuid, key: Option<&str>) -> Res {
    let mut request = app.post(&format!("/v1/checkout/{course}")).jar(who);
    if let Some(key) = key {
        request = request.header("idempotency-key", key);
    }
    request.send().await
}

fn order_id(res: &Res) -> Uuid {
    Uuid::parse_str(res.body["order"]["id"].as_str().unwrap()).unwrap()
}

async fn rows(pool: &PgPool, sql: &str) -> i64 {
    count(pool, sql).await
}

/// Lets a held claim on the provider call lapse without waiting for its full duration.
async fn claim_lapses(pool: &PgPool, order: Uuid) {
    sqlx::query("update orders set attempt_lease_until = now() - interval '1 second' where id=$1")
        .bind(order)
        .execute(pool)
        .await
        .unwrap();
}

async fn order_runs_out_of_time(pool: &PgPool, order: Uuid) {
    sqlx::query("update orders set expires_at = now() - interval '1 minute' where id=$1")
        .bind(order)
        .execute(pool)
        .await
        .unwrap();
}

db_test! {
    async fn checkout_prices_the_order_itself_and_opens_it_at_the_provider(pool: PgPool) {
        let w = world(pool.clone()).await;
        // A client that tries to name its own price and currency is simply not listened to.
        let res = w
            .app
            .post(&format!("/v1/checkout/{}", w.course.id))
            .jar(&w.buyer)
            .json(json!({"amount_paise": 100, "currency": "USD", "title": "Cheap"}))
            .send()
            .await;
        assert_eq!(res.status, StatusCode::CREATED, "{:?}", res.body);
        assert_eq!(res.body["order"]["amount_paise"], PRICE);
        assert_eq!(res.body["order"]["currency"], "INR");
        assert_eq!(res.body["order"]["status"], "pending");
        assert_eq!(res.body["order"]["course"]["title"], "Paid course");
        let checkout = &res.body["checkout"];
        assert_eq!(checkout["provider"], "fake");
        assert_eq!(checkout["key"], "fake_key");
        assert_eq!(checkout["amount_paise"], PRICE);
        assert_eq!(checkout["currency"], "INR");

        // The provider holds one order, for our price, labelled with our order id.
        let order = order_id(&res);
        let at_provider = w.provider.orders();
        assert_eq!(at_provider.len(), 1);
        assert_eq!(at_provider[0].amount_paise, PRICE);
        assert_eq!(at_provider[0].currency, "INR");
        assert_eq!(at_provider[0].receipt.as_deref(), Some(order.to_string().as_str()));
        assert_eq!(checkout["provider_order_id"], at_provider[0].id.as_str());

        // Nothing is granted by starting checkout.
        assert_eq!(rows(&pool, "select count(*) from enrollments").await, 0);
        assert_eq!(rows(&pool, "select count(*) from payments").await, 0);

        let expires: (bool,) = sqlx::query_as(
            "select expires_at between now() + interval '29 minutes' \
                                   and now() + interval '31 minutes' from orders",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(expires.0, "the order should be payable for about 30 minutes");
    }
}

db_test! {
    async fn the_snapshot_keeps_what_the_buyer_was_shown(pool: PgPool) {
        let w = world(pool.clone()).await;
        let res = checkout(&w.app, &w.buyer, w.course.id, None).await;
        let order = order_id(&res);
        let snapshot: Value = sqlx::query_scalar("select price_snapshot from orders where id=$1")
            .bind(order)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(snapshot["course_title"], "Paid course");
        assert_eq!(snapshot["list_price_paise"], PRICE);
        assert_eq!(snapshot["discount_paise"], 0);
        assert_eq!(snapshot["amount_paise"], PRICE);
        assert_eq!(snapshot["currency"], "INR");
        assert_eq!(snapshot["v"], 1);

        // The creator raises the price and renames the course mid-checkout: the order is unchanged.
        sqlx::query("update courses set price_paise = 99900, title = 'Renamed' where id=$1")
            .bind(w.course.id)
            .execute(&pool)
            .await
            .unwrap();
        let again = w.app.get(&format!("/v1/orders/{order}")).jar(&w.buyer).send().await;
        assert_eq!(again.status, StatusCode::OK);
        assert_eq!(again.body["order"]["amount_paise"], PRICE);
        assert_eq!(again.body["order"]["course"]["title"], "Paid course");
        assert_eq!(again.body["checkout"]["amount_paise"], PRICE);
        // Starting checkout again shares the open order, at the old price.
        let resumed = checkout(&w.app, &w.buyer, w.course.id, None).await;
        assert_eq!(resumed.status, StatusCode::OK);
        assert_eq!(order_id(&resumed), order);
        assert_eq!(resumed.body["order"]["amount_paise"], PRICE);
    }
}

db_test! {
    async fn a_second_click_shares_the_open_order(pool: PgPool) {
        let w = world(pool.clone()).await;
        let first = checkout(&w.app, &w.buyer, w.course.id, None).await;
        let second = checkout(&w.app, &w.buyer, w.course.id, None).await;
        assert_eq!(first.status, StatusCode::CREATED);
        assert_eq!(second.status, StatusCode::OK);
        assert_eq!(order_id(&first), order_id(&second));
        assert_eq!(first.body["checkout"], second.body["checkout"]);
        assert_eq!(w.provider.calls(Op::CreateOrder), 1);
        assert_eq!(rows(&pool, "select count(*) from orders").await, 1);
    }
}

db_test! {
    async fn ten_parallel_requests_make_one_order(pool: PgPool) {
        // Ten a minute is the limit, so ten at once is the most one account can send.
        let w = world(pool.clone()).await;
        let results = join_all((0..10).map(|_| checkout(&w.app, &w.buyer, w.course.id, None))).await;
        let created = results.iter().filter(|r| r.status == StatusCode::CREATED).count();
        let shared = results.iter().filter(|r| r.status == StatusCode::OK).count();
        assert_eq!((created, shared), (1, 9), "{:?}", results.iter().map(|r| r.status).collect::<Vec<_>>());
        let first = order_id(&results[0]);
        assert!(results.iter().all(|r| order_id(r) == first));
        assert_eq!(rows(&pool, "select count(*) from orders").await, 1);
        assert_eq!(w.provider.orders().len(), 1);
        assert_eq!(w.provider.calls(Op::CreateOrder), 1);
        // Anything beyond that is turned away before it reaches the database.
        assert_eq!(checkout(&w.app, &w.buyer, w.course.id, None).await.status, StatusCode::TOO_MANY_REQUESTS);
    }
}

db_test! {
    async fn twenty_parallel_checkouts_make_one_order_even_without_the_rate_limit(pool: PgPool) {
        // The service on its own, as a bug or a second API instance would reach it.
        let w = world(pool.clone()).await;
        let user = AuthUser {
            id: user_id(&pool, "buyer@checkout.in").await,
            role: Role::Student,
            session_id: Uuid::new_v4(),
        };
        let results = join_all(
            (0..20).map(|_| service::checkout(&w.app.state, &user, w.course.id, None)),
        )
        .await;
        let orders: Vec<_> = results.into_iter().map(|r| r.expect("every call succeeds")).collect();
        assert_eq!(orders.iter().filter(|o| o.created).count(), 1);
        let first = orders[0].order.id;
        assert!(orders.iter().all(|o| o.order.id == first));
        assert!(orders.iter().all(|o| o.order.provider_order_id.is_some()));
        assert_eq!(rows(&pool, "select count(*) from orders").await, 1);
        assert_eq!(w.provider.orders().len(), 1);
        assert_eq!(w.provider.calls(Op::CreateOrder), 1);
    }
}

db_test! {
    async fn parallel_checkouts_wait_for_a_slow_provider_instead_of_creating_again(pool: PgPool) {
        let w = world(pool.clone()).await;
        w.provider.inject(Op::CreateOrder, Fault::Delay(Duration::from_millis(400)));
        let results = join_all((0..8).map(|_| checkout(&w.app, &w.buyer, w.course.id, None))).await;
        for result in &results {
            assert!(
                result.status == StatusCode::CREATED || result.status == StatusCode::OK,
                "{:?} {:?}",
                result.status,
                result.body
            );
            assert_eq!(result.body["order"]["status"], "pending");
        }
        assert_eq!(w.provider.orders().len(), 1);
        assert_eq!(w.provider.calls(Op::CreateOrder), 1);
    }
}

db_test! {
    async fn a_slow_recovery_cannot_outlive_its_claim_and_create_a_second_provider_order(pool: PgPool) {
        let w = world(pool.clone()).await;
        let user = AuthUser {
            id: user_id(&pool, "buyer@checkout.in").await,
            role: Role::Student,
            session_id: Uuid::new_v4(),
        };

        // The first create gave no answer. The recovered attempt finds no order, then its
        // create call remains in flight longer than the old 15-second claim.
        w.provider.inject(Op::CreateOrder, Fault::Fail(ProviderError::Timeout));
        assert!(service::checkout(&w.app.state, &user, w.course.id, None).await.is_err());
        let order: Uuid = sqlx::query_scalar("select id from orders").fetch_one(&pool).await.unwrap();
        claim_lapses(&pool, order).await;
        w.provider.inject(Op::CreateOrder, Fault::Delay(Duration::from_secs(17)));

        let state = w.app.state.clone();
        let course_id = w.course.id;
        let first = tokio::spawn(async move { service::checkout(&state, &user, course_id, None).await });
        let started = tokio::time::Instant::now();
        while w.provider.calls(Op::CreateOrder) < 2 {
            assert!(started.elapsed() < Duration::from_secs(3), "recovery never reached create");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        tokio::time::sleep(Duration::from_millis(15_200)).await;

        let other = AuthUser {
            id: user_id(&pool, "buyer@checkout.in").await,
            role: Role::Student,
            session_id: Uuid::new_v4(),
        };
        let peer = service::checkout(&w.app.state, &other, w.course.id, None).await;
        if let Ok(peer) = peer {
            assert_eq!(peer.order.id, order, "the peer must share the recovered order");
        }
        assert!(first.await.unwrap().is_ok());
        assert_eq!(w.provider.calls(Op::CreateOrder), 2, "one uncertain call, one recovery");
        assert_eq!(w.provider.orders().len(), 1, "no duplicate provider order");
    }
}

db_test! {
    async fn different_buyers_get_their_own_orders(pool: PgPool) {
        let w = world(pool.clone()).await;
        let other = signup_verified_as(&w.app, "other@checkout.in", "student").await;
        let a = checkout(&w.app, &w.buyer, w.course.id, None).await;
        let b = checkout(&w.app, &other, w.course.id, None).await;
        assert_eq!(a.status, StatusCode::CREATED);
        assert_eq!(b.status, StatusCode::CREATED);
        assert_ne!(order_id(&a), order_id(&b));
        assert_eq!(w.provider.orders().len(), 2);
    }
}

db_test! {
    async fn idempotency_keys_return_the_order_they_made(pool: PgPool) {
        let w = world(pool.clone()).await;
        let first = checkout(&w.app, &w.buyer, w.course.id, Some("click-1")).await;
        assert_eq!(first.status, StatusCode::CREATED);
        let again = checkout(&w.app, &w.buyer, w.course.id, Some("click-1")).await;
        assert_eq!(again.status, StatusCode::OK);
        assert_eq!(order_id(&first), order_id(&again));

        // Same key, other course: refused, not silently answered with the wrong order.
        let teacher = w.creator.clone();
        let other_course = course(&w.app, &pool, &teacher, "Second course", 29_900).await;
        let reused = checkout(&w.app, &w.buyer, other_course.id, Some("click-1")).await;
        assert_eq!(reused.status, StatusCode::CONFLICT);
        assert_eq!(reused.code(), "idempotency_key_reused");

        // Keys belong to the person who sent them.
        let other = signup_verified_as(&w.app, "other@checkout.in", "student").await;
        let theirs = checkout(&w.app, &other, w.course.id, Some("click-1")).await;
        assert_eq!(theirs.status, StatusCode::CREATED);
        assert_ne!(order_id(&theirs), order_id(&first));
    }
}

db_test! {
    async fn a_key_replayed_after_payment_still_returns_that_order(pool: PgPool) {
        let w = world(pool.clone()).await;
        let first = checkout(&w.app, &w.buyer, w.course.id, Some("retry-me")).await;
        // The order is paid and the buyer enrolled by the (later) webhook; simulate its effect.
        sqlx::query("update orders set status='paid' where id=$1")
            .bind(order_id(&first))
            .execute(&pool)
            .await
            .unwrap();
        let replay = checkout(&w.app, &w.buyer, w.course.id, Some("retry-me")).await;
        assert_eq!(replay.status, StatusCode::OK);
        assert_eq!(order_id(&replay), order_id(&first));
        assert_eq!(replay.body["order"]["status"], "paid");
        assert!(replay.body["checkout"].is_null(), "a paid order offers nothing to pay");
    }
}

db_test! {
    async fn malformed_idempotency_keys_are_refused(pool: PgPool) {
        let w = world(pool.clone()).await;
        for key in ["", "has space", "a/b", &"k".repeat(65)] {
            let res = checkout(&w.app, &w.buyer, w.course.id, Some(key)).await;
            assert_eq!(res.status, StatusCode::BAD_REQUEST, "{key:?}");
            assert_eq!(res.code(), "invalid_idempotency_key");
        }
        assert_eq!(w.provider.calls(Op::CreateOrder), 0);
    }
}

db_test! {
    async fn browsers_may_send_the_idempotency_key_cross_origin(pool: PgPool) {
        let w = world(pool.clone()).await;
        let res = w
            .app
            .req(Method::OPTIONS, &format!("/v1/checkout/{}", w.course.id))
            .origin(ORIGIN)
            .header("access-control-request-method", "POST")
            .header("access-control-request-headers", "idempotency-key")
            .send()
            .await;
        let allowed = res.headers["access-control-allow-headers"].to_str().unwrap().to_lowercase();
        assert!(allowed.contains("idempotency-key"), "{allowed}");
    }
}

db_test! {
    async fn courses_that_cannot_be_bought_say_why(pool: PgPool) {
        let w = world(pool.clone()).await;

        // Free.
        let free = course(&w.app, &pool, &w.creator, "Free course", 0).await;
        let res = checkout(&w.app, &w.buyer, free.id, None).await;
        assert_eq!((res.status, res.code()), (StatusCode::CONFLICT, "not_payable"));

        // Not published.
        let draft = w
            .app
            .post("/v1/courses")
            .jar(&w.creator)
            .json(json!({"title": "Draft course", "price_paise": PRICE}))
            .send()
            .await;
        let draft = Uuid::parse_str(draft.body["course"]["id"].as_str().unwrap()).unwrap();
        let res = checkout(&w.app, &w.buyer, draft, None).await;
        assert_eq!((res.status, res.code()), (StatusCode::CONFLICT, "course_unavailable"));

        // Unpublished again after being live.
        let live = course(&w.app, &pool, &w.creator, "Withdrawn course", PRICE).await;
        sqlx::query("update courses set status='draft' where id=$1")
            .bind(live.id)
            .execute(&pool)
            .await
            .unwrap();
        let res = checkout(&w.app, &w.buyer, live.id, None).await;
        assert_eq!((res.status, res.code()), (StatusCode::CONFLICT, "course_unavailable"));

        // Deleted, unknown, and not an id at all.
        let gone = course(&w.app, &pool, &w.creator, "Deleted course", PRICE).await;
        sqlx::query("update courses set deleted_at=now() where id=$1")
            .bind(gone.id)
            .execute(&pool)
            .await
            .unwrap();
        for id in [gone.id.to_string(), Uuid::new_v4().to_string(), "not-a-uuid".to_owned()] {
            let res = w.app.post(&format!("/v1/checkout/{id}")).jar(&w.buyer).send().await;
            assert_eq!(res.status, StatusCode::NOT_FOUND, "{id}");
        }

        // The creator buying their own course.
        let res = checkout(&w.app, &w.creator, w.course.id, None).await;
        assert_eq!((res.status, res.code()), (StatusCode::CONFLICT, "own_course"));

        assert_eq!(w.provider.calls(Op::CreateOrder), 0, "no provider call for any of these");
        assert_eq!(rows(&pool, "select count(*) from orders").await, 0);
    }
}

db_test! {
    async fn people_who_already_have_access_cannot_pay_again(pool: PgPool) {
        let w = world(pool.clone()).await;
        let buyer = user_id(&pool, "buyer@checkout.in").await;
        let creator = user_id(&pool, "teacher@checkout.in").await;
        sqlx::query("insert into enrollments (user_id, course_id, source, granted_by) values ($1,$2,'grant',$3)")
            .bind(buyer)
            .bind(w.course.id)
            .bind(creator)
            .execute(&pool)
            .await
            .unwrap();
        let res = checkout(&w.app, &w.buyer, w.course.id, None).await;
        assert_eq!((res.status, res.code()), (StatusCode::CONFLICT, "already_enrolled"));
        assert_eq!(w.provider.calls(Op::CreateOrder), 0);
    }
}

db_test! {
    async fn a_creator_revocation_cannot_be_bought_back_but_a_refund_can(pool: PgPool) {
        let w = world(pool.clone()).await;
        let buyer = user_id(&pool, "buyer@checkout.in").await;
        let creator = user_id(&pool, "teacher@checkout.in").await;
        sqlx::query(
            "insert into enrollments \
                (user_id, course_id, source, granted_by, revoked_at, revoke_reason, revoked_by, revocation_kind) \
             values ($1,$2,'grant',$3, now(), 'misuse', $3, 'creator')",
        )
        .bind(buyer)
        .bind(w.course.id)
        .bind(creator)
        .execute(&pool)
        .await
        .unwrap();
        let res = checkout(&w.app, &w.buyer, w.course.id, None).await;
        assert_eq!((res.status, res.code()), (StatusCode::CONFLICT, "access_revoked_by_creator"));

        sqlx::query("update enrollments set revocation_kind='payment' where user_id=$1")
            .bind(buyer)
            .execute(&pool)
            .await
            .unwrap();
        let res = checkout(&w.app, &w.buyer, w.course.id, None).await;
        assert_eq!(res.status, StatusCode::CREATED, "{:?}", res.body);
    }
}

db_test! {
    async fn an_expired_grant_can_be_renewed_by_paying(pool: PgPool) {
        let w = world(pool.clone()).await;
        let buyer = user_id(&pool, "buyer@checkout.in").await;
        sqlx::query(
            "insert into enrollments (user_id, course_id, source, expires_at) \
             values ($1,$2,'grant', now() - interval '1 day')",
        )
        .bind(buyer)
        .bind(w.course.id)
        .execute(&pool)
        .await
        .unwrap();
        let res = checkout(&w.app, &w.buyer, w.course.id, None).await;
        assert_eq!(res.status, StatusCode::CREATED, "{:?}", res.body);
    }
}

db_test! {
    async fn only_signed_in_people_can_check_out(pool: PgPool) {
        let w = world(pool.clone()).await;
        let res = w
            .app
            .post(&format!("/v1/checkout/{}", w.course.id))
            .send()
            .await;
        assert_eq!(res.status, StatusCode::UNAUTHORIZED);
        assert_eq!(w.provider.calls(Op::CreateOrder), 0);
    }
}

db_test! {
    async fn a_provider_timeout_after_creating_the_order_does_not_create_a_second(pool: PgPool) {
        let w = world(pool.clone()).await;
        w.provider.inject(Op::CreateOrder, Fault::FailAfter(ProviderError::Timeout));
        let first = checkout(&w.app, &w.buyer, w.course.id, None).await;
        assert_eq!(first.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(first.code(), "unavailable");
        // The provider has the order; we never heard so. Our order is waiting, not open to pay.
        assert_eq!(w.provider.orders().len(), 1);
        let (order, status): (Uuid, String) =
            sqlx::query_as("select id, status::text from orders").fetch_one(&pool).await.unwrap();
        assert_eq!(status, "created");

        // Trying again straight away must not race an answer that may still arrive.
        let too_soon = checkout(&w.app, &w.buyer, w.course.id, None).await;
        assert_eq!(too_soon.status, StatusCode::CONFLICT);
        assert_eq!(too_soon.code(), "checkout_in_progress");
        assert_eq!(w.provider.calls(Op::CreateOrder), 1);

        // Once the claim lapses, the retry finds the provider's order by receipt and adopts it.
        claim_lapses(&pool, order).await;
        let retry = checkout(&w.app, &w.buyer, w.course.id, None).await;
        assert_eq!(retry.status, StatusCode::OK, "{:?}", retry.body);
        assert_eq!(order_id(&retry), order);
        assert_eq!(retry.body["order"]["status"], "pending");
        assert_eq!(w.provider.orders().len(), 1, "no orphan, no second order");
        assert_eq!(w.provider.calls(Op::CreateOrder), 1);
        assert_eq!(w.provider.calls(Op::FindOrder), 1);
        assert_eq!(
            retry.body["checkout"]["provider_order_id"],
            w.provider.orders()[0].id.as_str()
        );
    }
}

db_test! {
    async fn a_provider_timeout_before_creating_leaves_nothing_behind_and_the_retry_creates_it(pool: PgPool) {
        let w = world(pool.clone()).await;
        w.provider.inject(Op::CreateOrder, Fault::Fail(ProviderError::Timeout));
        let first = checkout(&w.app, &w.buyer, w.course.id, None).await;
        assert_eq!(first.status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(w.provider.orders().is_empty());
        let order: Uuid = sqlx::query_scalar("select id from orders").fetch_one(&pool).await.unwrap();
        claim_lapses(&pool, order).await;
        let retry = checkout(&w.app, &w.buyer, w.course.id, None).await;
        assert_eq!(retry.status, StatusCode::OK, "{:?}", retry.body);
        assert_eq!(retry.body["order"]["status"], "pending");
        assert_eq!(w.provider.orders().len(), 1);
    }
}

db_test! {
    async fn a_clear_refusal_frees_the_order_for_an_immediate_retry(pool: PgPool) {
        let w = world(pool.clone()).await;
        w.provider.inject(
            Op::CreateOrder,
            Fault::Fail(ProviderError::Unreachable("connection refused".into())),
        );
        let first = checkout(&w.app, &w.buyer, w.course.id, None).await;
        assert_eq!(first.status, StatusCode::SERVICE_UNAVAILABLE);
        // Nothing was sent, so there is nothing to wait for.
        let retry = checkout(&w.app, &w.buyer, w.course.id, None).await;
        assert_eq!(retry.status, StatusCode::OK, "{:?}", retry.body);
        assert_eq!(retry.body["order"]["status"], "pending");
        assert_eq!(w.provider.orders().len(), 1);
        assert_eq!(rows(&pool, "select count(*) from orders").await, 1);
    }
}

db_test! {
    async fn provider_errors_become_the_right_status_without_leaking_their_words(pool: PgPool) {
        let w = world(pool.clone()).await;
        let cases = [
            (ProviderError::Unauthorized, StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
            (ProviderError::RateLimited, StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
            (ProviderError::Unavailable("HTTP 502".into()), StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
            (
                ProviderError::Rejected {
                    code: "BAD_REQUEST_ERROR".into(),
                    description: "secret internal detail".into(),
                },
                StatusCode::BAD_GATEWAY,
                "payment_provider_error",
            ),
            (ProviderError::Malformed("garbage".into()), StatusCode::BAD_GATEWAY, "payment_provider_error"),
        ];
        for (error, status, code) in cases {
            let unknown_outcome = error.outcome_unknown();
            w.provider.inject(Op::CreateOrder, Fault::Fail(error));
            let res = checkout(&w.app, &w.buyer, w.course.id, None).await;
            assert_eq!((res.status, res.code()), (status, code));
            assert!(!res.body.to_string().contains("secret internal detail"));
            let order: Uuid = sqlx::query_scalar("select id from orders").fetch_one(&pool).await.unwrap();
            if unknown_outcome {
                claim_lapses(&pool, order).await;
            }
        }
        // After all that the order is still one, and the next healthy call opens it.
        assert_eq!(rows(&pool, "select count(*) from orders").await, 1);
        let ok = checkout(&w.app, &w.buyer, w.course.id, None).await;
        assert_eq!(ok.status, StatusCode::OK, "{:?}", ok.body);
        assert_eq!(ok.body["order"]["status"], "pending");
    }
}

/// A provider that answers with a different amount than it was asked for.
struct Tampering {
    inner: FakeProvider,
    amount_delta: i64,
    receipt: Option<String>,
}

#[async_trait]
impl PaymentProvider for Tampering {
    fn name(&self) -> &'static str {
        "fake"
    }
    fn checkout_key(&self) -> String {
        self.inner.checkout_key()
    }
    async fn create_order(&self, order: &CreateOrder) -> Result<ProviderOrder, ProviderError> {
        let mut created = self.inner.create_order(order).await?;
        created.amount_paise += self.amount_delta;
        if let Some(receipt) = &self.receipt {
            created.receipt = Some(receipt.clone());
        }
        Ok(created)
    }
    async fn find_order_by_receipt(
        &self,
        receipt: &str,
    ) -> Result<Option<ProviderOrder>, ProviderError> {
        self.inner.find_order_by_receipt(receipt).await
    }
    async fn fetch_order(&self, id: &str) -> Result<ProviderOrder, ProviderError> {
        self.inner.fetch_order(id).await
    }
    async fn fetch_payment(&self, id: &str) -> Result<ProviderPayment, ProviderError> {
        self.inner.fetch_payment(id).await
    }
    async fn fetch_order_payments(&self, id: &str) -> Result<Vec<ProviderPayment>, ProviderError> {
        self.inner.fetch_order_payments(id).await
    }
    async fn fetch_refund(&self, id: &str) -> Result<ProviderRefund, ProviderError> {
        self.inner.fetch_refund(id).await
    }
    async fn refund(&self, request: &RefundRequest) -> Result<ProviderRefund, ProviderError> {
        self.inner.refund(request).await
    }
    fn verify_webhook(&self, body: &[u8], signature: &str) -> bool {
        self.inner.verify_webhook(body, signature)
    }
}

db_test! {
    async fn an_order_that_differs_from_what_we_asked_for_is_never_opened(pool: PgPool) {
        for (delta, receipt) in [(-100, None), (0, Some("someone-elses-order".to_owned()))] {
            let mut tx = pool.begin().await.unwrap();
            sqlx::query("delete from orders").execute(&mut *tx).await.unwrap();
            tx.commit().await.unwrap();
            let tampering = Arc::new(Tampering { inner: FakeProvider::new(), amount_delta: delta, receipt });
            let mailer = MemoryMailer::default();
            let state = make_state(pool.clone(), false, 0, Arc::new(mailer.clone()))
                .with_payments(tampering);
            let app = App {
                router: zeno_api::build_app(state.clone()),
                mailer,
                pool: pool.clone(),
                state,
            };
            let creator = signup_creator(&app, &format!("teacher{delta}@checkout.in")).await;
            let buyer = signup_verified_as(&app, &format!("buyer{delta}@checkout.in"), "student").await;
            let course = course(&app, &pool, &creator, "Tamper course", PRICE).await;
            let res = checkout(&app, &buyer, course.id, None).await;
            assert_eq!(res.status, StatusCode::BAD_GATEWAY, "{:?}", res.body);
            assert_eq!(res.code(), "payment_provider_error");
            let status: String = sqlx::query_scalar("select status::text from orders")
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(status, "created", "a mismatching provider order must not be attached");
            let attached: Option<String> = sqlx::query_scalar("select provider_order_id from orders")
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(attached, None);
        }
    }
}

db_test! {
    async fn an_order_past_its_expiry_reads_as_expired_and_offers_nothing_to_pay(pool: PgPool) {
        let w = world(pool.clone()).await;
        let first = checkout(&w.app, &w.buyer, w.course.id, None).await;
        let order = order_id(&first);
        order_runs_out_of_time(&pool, order).await;
        let read = w.app.get(&format!("/v1/orders/{order}")).jar(&w.buyer).send().await;
        assert_eq!(read.status, StatusCode::OK);
        assert_eq!(read.body["order"]["status"], "expired");
        assert!(read.body["checkout"].is_null());
    }
}

db_test! {
    async fn checking_out_after_expiry_starts_a_fresh_order_at_todays_price(pool: PgPool) {
        let w = world(pool.clone()).await;
        let first = checkout(&w.app, &w.buyer, w.course.id, None).await;
        let old = order_id(&first);
        order_runs_out_of_time(&pool, old).await;
        sqlx::query("update courses set price_paise = 59900 where id=$1")
            .bind(w.course.id)
            .execute(&pool)
            .await
            .unwrap();

        let second = checkout(&w.app, &w.buyer, w.course.id, None).await;
        assert_eq!(second.status, StatusCode::CREATED, "{:?}", second.body);
        assert_ne!(order_id(&second), old);
        assert_eq!(second.body["order"]["amount_paise"], 59_900);
        let old_status: String = sqlx::query_scalar("select status::text from orders where id=$1")
            .bind(old)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(old_status, "expired");
        assert_eq!(rows(&pool, "select count(*) from orders where status in ('created','pending')").await, 1);
    }
}

db_test! {
    async fn an_expired_order_that_was_paid_at_the_last_moment_blocks_a_second_one(pool: PgPool) {
        let w = world(pool.clone()).await;
        let first = checkout(&w.app, &w.buyer, w.course.id, None).await;
        let old = order_id(&first);
        let provider_order = first.body["checkout"]["provider_order_id"].as_str().unwrap().to_owned();
        // The customer paid just as the order ran out; the webhook has not arrived yet.
        w.provider.capture(&provider_order).unwrap();
        order_runs_out_of_time(&pool, old).await;

        let second = checkout(&w.app, &w.buyer, w.course.id, None).await;
        assert_eq!(second.status, StatusCode::CONFLICT, "{:?}", second.body);
        assert_eq!(second.code(), "payment_processing");
        assert_eq!(rows(&pool, "select count(*) from orders").await, 1);
        assert_eq!(w.provider.orders().len(), 1, "no second order next to a payment in flight");
    }
}

db_test! {
    async fn when_the_provider_cannot_say_whether_an_expired_order_was_paid_nothing_new_is_made(pool: PgPool) {
        let w = world(pool.clone()).await;
        let first = checkout(&w.app, &w.buyer, w.course.id, None).await;
        order_runs_out_of_time(&pool, order_id(&first)).await;
        w.provider.inject(Op::FetchOrder, Fault::Fail(ProviderError::Timeout));
        let second = checkout(&w.app, &w.buyer, w.course.id, None).await;
        assert_eq!(second.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(w.provider.orders().len(), 1);
        // The next try, with the provider answering, goes through.
        let third = checkout(&w.app, &w.buyer, w.course.id, None).await;
        assert_eq!(third.status, StatusCode::CREATED, "{:?}", third.body);
    }
}

db_test! {
    async fn orders_are_visible_to_their_buyer_only(pool: PgPool) {
        let w = world(pool.clone()).await;
        let order = order_id(&checkout(&w.app, &w.buyer, w.course.id, None).await);
        let url = format!("/v1/orders/{order}");

        let mine = w.app.get(&url).jar(&w.buyer).send().await;
        assert_eq!(mine.status, StatusCode::OK);
        assert_eq!(mine.body["order"]["id"], order.to_string());
        assert_eq!(mine.body["checkout"]["provider"], "fake");

        let other = signup_verified_as(&w.app, "other@checkout.in", "student").await;
        assert_eq!(w.app.get(&url).jar(&other).send().await.status, StatusCode::NOT_FOUND);
        // Not even the course's own creator sees what a student is buying.
        assert_eq!(w.app.get(&url).jar(&w.creator).send().await.status, StatusCode::NOT_FOUND);
        assert_eq!(w.app.get(&url).send().await.status, StatusCode::UNAUTHORIZED);
        assert_eq!(
            w.app.get("/v1/orders/not-a-uuid").jar(&w.buyer).send().await.status,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            w.app.get(&format!("/v1/orders/{}", Uuid::new_v4())).jar(&w.buyer).send().await.status,
            StatusCode::NOT_FOUND
        );
    }
}

db_test! {
    async fn checkout_is_rate_limited_per_account(pool: PgPool) {
        let w = world(pool.clone()).await;
        let free = course(&w.app, &pool, &w.creator, "Free course", 0).await;
        for _ in 0..10 {
            let res = checkout(&w.app, &w.buyer, free.id, None).await;
            assert_eq!(res.status, StatusCode::CONFLICT);
        }
        let res = checkout(&w.app, &w.buyer, free.id, None).await;
        assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS);
        // Another account is not affected.
        let other = signup_verified_as(&w.app, "other@checkout.in", "student").await;
        assert_eq!(checkout(&w.app, &other, free.id, None).await.status, StatusCode::CONFLICT);
    }
}

db_test! {
    async fn the_database_refuses_orders_that_break_the_rules(pool: PgPool) {
        let w = world(pool.clone()).await;
        let res = checkout(&w.app, &w.buyer, w.course.id, None).await;
        let order = order_id(&res);
        let buyer = user_id(&pool, "buyer@checkout.in").await;

        let insert = |status: &'static str, provider_order: Option<&'static str>, amount: i64, currency: &'static str| {
            let pool = pool.clone();
            async move {
                sqlx::query(
                    "insert into orders (user_id, course_id, amount_paise, currency, price_snapshot, \
                        status, provider, provider_order_id, expires_at) \
                     select $1, course_id, $2, $3, '{}'::jsonb, $4::order_status, 'fake', $5, now() \
                     from orders where id = $6",
                )
                .bind(buyer)
                .bind(amount)
                .bind(currency)
                .bind(status)
                .bind(provider_order)
                .bind(order)
                .execute(&pool)
                .await
            }
        };
        // A second open order for the same person and course.
        assert!(insert("pending", Some("order_other"), PRICE, "INR").await.is_err());
        // A closed one is fine, once.
        assert!(insert("expired", None, PRICE, "INR").await.is_ok());
        // Paid or pending without naming the provider's order.
        assert!(insert("paid", None, PRICE, "INR").await.is_err());
        // Wrong amounts and currencies.
        assert!(insert("expired", None, 0, "INR").await.is_err());
        assert!(insert("expired", None, 99, "INR").await.is_err());
        assert!(insert("expired", None, PRICE, "USD").await.is_err());
        // The provider's order belongs to one of our orders only.
        let taken: String = sqlx::query_scalar("select provider_order_id from orders where id=$1")
            .bind(order)
            .fetch_one(&pool)
            .await
            .unwrap();
        let taken: &'static str = Box::leak(taken.into_boxed_str());
        assert!(insert("paid", Some(taken), PRICE, "INR").await.is_err());
    }
}
