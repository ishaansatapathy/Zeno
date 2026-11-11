//! Chaos for the payment path: kill the connection halfway, abandon requests at every instant,
//! drop the database's connections while payments pour in, and shuffle everything that can
//! arrive. After all of it, one thing: every payment is applied exactly once, completely, and the
//! book agrees with the tables.
//!
//! The individual guarantees have their own tests (`webhooks`, `reconcile`, `ledger`). This file
//! is for the ways they could fail to compose.

mod common;

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::shop::*;
use common::*;
use futures_util::future::join_all;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use sqlx::{Connection, PgConnection, PgPool};
use uuid::Uuid;
use zeno_api::auth::extractor::AuthUser;
use zeno_api::ledger::integrity;
use zeno_api::orders::service as orders;
use zeno_api::reconcile::run_once;
use zeno_core::model::Role;

macro_rules! db_test {
    ($(#[$meta:meta])* async fn $name:ident($pool:ident: PgPool) $body:block) => {
        #[sqlx::test(migrator = "zeno_core::db::MIGRATOR")]
        $(#[$meta])*
        async fn $name($pool: PgPool) $body
    };
}

// ---- what must always be true ------------------------------------------------------------------------

/// Counts the ways the tables disagree about what a payment did. A payment is either all of
/// itself (a confirmed or refunded payment, the paid order, the enrollment, the sale in the book)
/// or none of it, at every instant, whatever was interrupted.
async fn half_done(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "select \
           (select count(*) from payments p where p.status in ('confirmed','refunded') \
              and not exists (select 1 from enrollments e where e.payment_id = p.id)) \
         + (select count(*) from payments p where p.status in ('confirmed','refunded') \
              and not exists (select 1 from ledger_journals j where j.kind = 'sale' and j.source_id = p.id)) \
         + (select count(*) from ledger_journals j where j.kind = 'sale' \
              and not exists (select 1 from payments p where p.id = j.source_id and p.status in ('confirmed','refunded'))) \
         + (select count(*) from enrollments e where e.payment_id is not null \
              and not exists (select 1 from payments p where p.id = e.payment_id and p.status in ('confirmed','refunded'))) \
         + (select count(*) from orders o where o.status in ('paid','refunded') \
              and not exists (select 1 from payments p where p.order_id = o.id and p.status in ('confirmed','refunded'))) \
         + (select count(*) from payments p where p.status in ('confirmed','refunded') \
              and not exists (select 1 from orders o where o.id = p.order_id and o.status in ('paid','refunded')))",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn the_book_agrees(pool: &PgPool) {
    assert_eq!(half_done(pool).await, 0, "a payment was left half-done");
    assert_eq!(
        integrity::check(pool).await.unwrap(),
        vec![],
        "the book disagrees with the tables"
    );
}

/// A buyer with an open order for the shop's course.
#[derive(Clone)]
struct Buyer {
    id: Uuid,
    order: Uuid,
    remote: String,
}

async fn another_order(shop: &Shop, email: &str) -> Buyer {
    // Straight to the database and the service: signing hundreds of people up through the front
    // door would only test the sign-up rate limit.
    let id: Uuid = sqlx::query_scalar(
        "insert into users (name, email, password_hash, role) values ($1, $2, 'x', 'student') returning id",
    )
    .bind("Chaos buyer")
    .bind(email)
    .fetch_one(&shop.pool)
    .await
    .unwrap();
    let user = AuthUser {
        id,
        role: Role::Student,
        session_id: Uuid::new_v4(),
    };
    let started = orders::checkout(&shop.app.state, &user, shop.course, None)
        .await
        .expect("checkout starts");
    assert!(started.created);
    Buyer {
        id,
        order: started.order.id,
        remote: started
            .order
            .provider_order_id
            .expect("the provider has the order"),
    }
}

/// The provider's side of a delivery: send it again, as it does, until it is accepted.
/// A shop whose app talks to the database through a pool configured as production's is: the test
/// harness's own pool does not check a connection before handing it out, which hides exactly what
/// a failover test is about.
async fn shop_on_a_production_pool(test_pool: PgPool) -> Shop {
    let options = (*test_pool.connect_options()).clone();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(5))
        .connect_with(options)
        .await
        .unwrap();
    let (app, provider) = app_with_provider(pool.clone());
    shop_in(app, provider, pool).await
}

/// How much is going on, for a failover that wants to strike while there is something to disturb,
/// and for a test that wants to know it did.
#[derive(Default)]
struct Traffic {
    in_flight: AtomicUsize,
    interrupted: AtomicU32,
}

async fn deliver_until_accepted(shop: &Shop, traffic: &Traffic, id: &str, body: Vec<u8>) -> Res {
    for attempt in 0..400u64 {
        traffic.in_flight.fetch_add(1, Ordering::SeqCst);
        let res = deliver(shop, id, body.clone()).await;
        traffic.in_flight.fetch_sub(1, Ordering::SeqCst);
        if !res.status.is_server_error() {
            return res;
        }
        traffic.interrupted.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(5 + attempt % 7)).await;
    }
    panic!("{id} was never accepted");
}

/// Kills the connection of whoever runs this, the way a database failover or a crash would.
const KILL_SELF: &str = "pg_terminate_backend(pg_backend_pid())";

// ---- 1. the connection dies at each step -------------------------------------------------------------

db_test! {
    async fn a_connection_lost_at_any_step_of_a_payment_leaves_nothing_behind(pool: PgPool) {
        let shop = shop(pool).await;
        sqlx::query(&format!(
            "create function die() returns trigger language plpgsql as $$ begin perform {KILL_SELF}; return new; end $$"
        ))
        .execute(&shop.pool).await.unwrap();

        // Every write a payment makes, in the order it makes them; the last is the one just
        // before the commit.
        let steps = [
            ("before insert", "webhook_events"),
            ("before insert", "payments"),
            ("before insert", "order_events"),
            ("before insert", "enrollments"),
            ("before insert", "enrollment_events"),
            ("before insert", "ledger_journals"),
            ("before insert", "ledger_entries"),
            ("before insert", "email_outbox"),
            ("before update", "webhook_events"),
        ];
        for (when, table) in steps {
            sqlx::query(&format!("create trigger die {when} on {table} for each row execute function die()"))
                .execute(&shop.pool).await.unwrap();

            let res = pay(&shop, "evt_1", "pay_1").await;

            assert!(res.status.is_server_error(), "{when} {table}: {} {:?}", res.status, res.body);
            sqlx::query(&format!("drop trigger die on {table}")).execute(&shop.pool).await.unwrap();
            assert_eq!(order_status(&shop).await, "pending", "{when} {table}");
            for kept in ["payments", "enrollments", "ledger_journals", "ledger_entries", "webhook_events", "order_events"] {
                assert_eq!(n(&shop, &format!("select count(*) from {kept}")).await, 0, "{when} {table}: {kept}");
            }
            assert_eq!(n(&shop, "select count(*) from email_outbox where subject like 'Payment received%'").await, 0, "{when} {table}");
        }

        // After all that, the provider's retry goes through, once.
        let retried = pay(&shop, "evt_1", "pay_1").await;
        assert_eq!(outcome(&retried), "enrolled", "{:?}", retried.body);
        assert_eq!(pay(&shop, "evt_1", "pay_1").await.body["duplicate"], true);
        assert_eq!(access(&shop).await, "allowed");
        the_book_agrees(&shop.pool).await;
    }
}

// ---- 2. the request is abandoned at every instant ----------------------------------------------------------

db_test! {
    async fn a_delivery_abandoned_at_any_instant_is_all_or_nothing(pool: PgPool) {
        let shop = shop(pool).await;
        let mut buyers = vec![Buyer {
            id: shop.buyer_id,
            order: shop.order,
            remote: shop.remote.clone(),
        }];
        for i in 0..18 {
            buyers.push(another_order(&shop, &format!("abandon{i}@chaos.in")).await);
        }

        let body_of = |b: &Buyer| captured_with_fee(&format!("pay_{}", b.order.simple()), &b.remote, PRICE, Some(1_000));

        // How long a delivery takes here, so the sweep is spread over its whole life on any machine.
        let mut slowest = Duration::ZERO;
        for (i, b) in buyers.iter().take(3).enumerate() {
            let started = Instant::now();
            let res = deliver(&shop, &format!("evt_calibrate_{i}"), body_of(b)).await;
            assert_eq!(outcome(&res), "enrolled", "{:?}", res.body);
            slowest = slowest.max(started.elapsed());
        }
        let rest = &buyers[3..];

        let mut dropped_unpaid = 0;
        for (i, b) in rest.iter().enumerate() {
            // From "dropped before it begins" to "three times as long as it takes".
            let wait = slowest.mul_f64(3.0 * i as f64 / (rest.len() - 1) as f64);
            let abandoned = tokio::time::timeout(wait, deliver(&shop, &format!("evt_{i}"), body_of(b))).await;
            // Whatever was going on is either over or on its way to being rolled back.
            tokio::time::sleep(Duration::from_millis(40)).await;
            assert_eq!(half_done(&shop.pool).await, 0, "abandoned after {wait:?}");
            let paid = order_state(&shop, b).await == "paid";
            match abandoned {
                Err(_) if !paid => dropped_unpaid += 1,
                Err(_) => {}
                Ok(res) => assert_eq!(outcome(&res), "enrolled"),
            }
        }
        assert!(dropped_unpaid > 0, "no delivery was abandoned before it finished: the sweep tested nothing");

        // The provider sends everything again; each payment ends up applied once, whatever came before.
        for (i, b) in rest.iter().enumerate() {
            let res = deliver_until_accepted(&shop, &Traffic::default(), &format!("evt_{i}"), body_of(b)).await;
            assert!(res.status.is_success(), "{:?}", res.body);
        }
        assert_eq!(n(&shop, "select count(*) from orders where status = 'paid'").await, buyers.len() as i64);
        assert_eq!(n(&shop, "select count(*) from enrollments").await, buyers.len() as i64);
        assert_eq!(n(&shop, "select count(*) from payments where status = 'confirmed'").await, buyers.len() as i64);
        assert_eq!(n(&shop, "select count(*) from ledger_journals where kind = 'sale'").await, buyers.len() as i64);
        the_book_agrees(&shop.pool).await;
    }
}

async fn order_state(shop: &Shop, buyer: &Buyer) -> String {
    text(
        shop,
        &format!(
            "select status::text from orders where id = '{}'",
            buyer.order
        ),
    )
    .await
}

// ---- 3. the database drops its connections while payments arrive --------------------------------------------

/// When a delivery starts: in one of three waves a second and a half apart, a little apart from
/// each other, so that each failover has a crowd to interrupt.
fn wave(rng: &mut StdRng) -> u64 {
    rng.gen_range(0..3u64) * 1_500 + rng.gen_range(0..120)
}

/// How long the database works between failovers. A delivery needs a few fresh connections after
/// each one (about 60 ms apiece on a laptop), so anything shorter than a second or so is not a
/// failover, it is an outage, and no payment could be expected to get through it.
const QUIET: Duration = Duration::from_millis(1_200);

/// A failover: every connection to the database is terminated at once, mid-request, and a moment
/// later the database works again. It waits for there to be traffic, strikes part-way through it,
/// then leaves `QUIET` for the survivors to recover before the next one.
struct Failover {
    stop: Arc<AtomicBool>,
    struck: Arc<AtomicU32>,
    task: tokio::task::JoinHandle<()>,
}

impl Failover {
    fn start(pool: &PgPool, traffic: Arc<Traffic>, strikes: u32) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let struck = Arc::new(AtomicU32::new(0));
        let options = (*pool.connect_options()).clone();
        let (flag, count) = (stop.clone(), struck.clone());
        let task = tokio::spawn(async move {
            let mut conn = PgConnection::connect_with(&options).await.unwrap();
            let mut rng = StdRng::seed_from_u64(7);
            while count.load(Ordering::SeqCst) < strikes && !flag.load(Ordering::Relaxed) {
                while traffic.in_flight.load(Ordering::SeqCst) == 0 {
                    tokio::time::sleep(Duration::from_millis(2)).await;
                }
                // Let a wave build up, so that one strike has several victims.
                for _ in 0..100 {
                    if traffic.in_flight.load(Ordering::SeqCst) >= 4 {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(2)).await;
                }
                tokio::time::sleep(Duration::from_millis(rng.gen_range(15..60))).await;
                let _ = sqlx::query(
                    "select pg_terminate_backend(pid) from pg_stat_activity \
                     where datname = current_database() and pid <> pg_backend_pid()",
                )
                .execute(&mut conn)
                .await;
                count.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(QUIET).await;
            }
        });
        Self { stop, struck, task }
    }

    /// Ends it, returning how many times it struck.
    fn end(self) -> u32 {
        self.stop.store(true, Ordering::Relaxed);
        self.task.abort();
        self.struck.load(Ordering::SeqCst)
    }
}

db_test! {
    async fn a_database_that_keeps_dropping_connections_loses_no_payment_and_counts_none_twice(pool: PgPool) {
        let shop = shop_on_a_production_pool(pool).await;
        let mut buyers = vec![Buyer {
            id: shop.buyer_id,
            order: shop.order,
            remote: shop.remote.clone(),
        }];
        for i in 0..9 {
            buyers.push(another_order(&shop, &format!("storm{i}@chaos.in")).await);
        }

        let traffic = Arc::new(Traffic::default());
        let failover = Failover::start(&shop.pool, traffic.clone(), 3);
        let mut rng = StdRng::seed_from_u64(99);
        // Each payment is announced three ways, two of them the same event twice at once, and the
        // deliveries are spread over several seconds so that every failover has work to interrupt.
        let deliveries = buyers.iter().flat_map(|b| {
            let pay = format!("pay_{}", b.order.simple());
            let id = b.order.simple().to_string();
            let captured_body = captured_with_fee(&pay, &b.remote, PRICE, Some(1_200));
            vec![
                (format!("evt_c_{id}"), captured_body.clone()),
                (format!("evt_c_{id}"), captured_body),
                (format!("evt_o_{id}"), order_paid(&pay, &b.remote, PRICE)),
            ]
        }).map(|(id, body)| (wave(&mut rng), id, body)).collect::<Vec<_>>();
        let results = join_all(deliveries.into_iter().map(|(delay, id, body)| {
            let (shop, traffic) = (&shop, &traffic);
            async move {
                tokio::time::sleep(Duration::from_millis(delay)).await;
                deliver_until_accepted(shop, traffic, &id, body).await
            }
        })).await;
        let strikes = failover.end();

        assert!(strikes >= 2, "the failover struck {strikes} times: it barely ran");
        let interrupted = traffic.interrupted.load(Ordering::SeqCst);
        assert!(interrupted >= 2, "only {interrupted} deliveries were ever interrupted");
        assert!(results.iter().all(|r| r.status.is_success()));
        let total = buyers.len() as i64;
        assert_eq!(n(&shop, "select count(*) from orders where status = 'paid'").await, total);
        assert_eq!(n(&shop, "select count(*) from enrollments").await, total);
        assert_eq!(n(&shop, "select count(*) from payments where status = 'confirmed'").await, total);
        assert_eq!(n(&shop, "select count(*) from ledger_journals where kind = 'sale'").await, total);
        // The provider's fee reached every sale, whichever event got there first.
        assert_eq!(n(&shop, "select count(*) from payments where fee_paise = 1200").await, total);
        assert_eq!(
            n(&shop, "select coalesce(sum(amount_paise), 0)::bigint from ledger_entries where creator_id is not null").await,
            total * (PRICE - 1_200)
        );
        the_book_agrees(&shop.pool).await;
    }
}

// ---- 4. everything at once, against a model -----------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fate {
    Paid,
    PaidThenRefunded,
    PaidThenPartlyRefunded,
    FailedThenPaid,
    PaidThenFailedLate,
    FailedOnly,
    /// The customer paid and no webhook ever came: reconciliation finds it.
    MissedWebhook,
    /// A payment for the wrong amount: nothing granted.
    WrongAmount,
}

const FATES: [Fate; 8] = [
    Fate::Paid,
    Fate::PaidThenRefunded,
    Fate::PaidThenPartlyRefunded,
    Fate::FailedThenPaid,
    Fate::PaidThenFailedLate,
    Fate::FailedOnly,
    Fate::MissedWebhook,
    Fate::WrongAmount,
];

const PARTIAL: i64 = 30_000;

struct Planned {
    buyer: Buyer,
    fate: Fate,
    pay: String,
    fee: i64,
    /// Groups of deliveries; those in a group arrive together, the groups one after another.
    stages: Vec<Vec<(String, Vec<u8>)>>,
}

/// Each delivery is sent one to three times, some of the repeats under a new event id (the
/// provider does both).
fn repeated(rng: &mut StdRng, id: &str, body: Vec<u8>) -> Vec<(String, Vec<u8>)> {
    (0..rng.gen_range(1..=3))
        .map(|k| {
            let id = if k > 0 && rng.gen_bool(0.5) {
                format!("{id}_again{k}")
            } else {
                id.to_owned()
            };
            (id, body.clone())
        })
        .collect()
}

fn plan(rng: &mut StdRng, buyer: Buyer, fate: Fate, fee: i64) -> Planned {
    let key = buyer.order.simple().to_string();
    let pay = format!("pay_{key}");
    let remote = buyer.remote.clone();
    let captured = |rng: &mut StdRng| {
        let mut group = repeated(
            rng,
            &format!("evt_c_{key}"),
            captured_with_fee(&pay, &remote, PRICE, Some(fee)),
        );
        // `order.paid` says the same thing, without the fee.
        group.extend(repeated(
            rng,
            &format!("evt_o_{key}"),
            order_paid(&pay, &remote, PRICE),
        ));
        group
    };
    let failed_event = |rng: &mut StdRng, pay_id: &str, suffix: &str| {
        repeated(
            rng,
            &format!("evt_f_{key}_{suffix}"),
            failed(pay_id, &remote, PRICE),
        )
    };
    let stages = match fate {
        Fate::Paid => vec![captured(rng)],
        Fate::PaidThenRefunded => vec![
            captured(rng),
            repeated(
                rng,
                &format!("evt_r_{key}"),
                refund_processed(&format!("rfnd_{key}"), &pay, PRICE, None),
            ),
        ],
        Fate::PaidThenPartlyRefunded => {
            let mut refunds = repeated(
                rng,
                &format!("evt_ra_{key}"),
                refund_processed(&format!("rfnd_{key}_a"), &pay, PARTIAL, None),
            );
            // A second, different refund of the same size arriving at the same time: only one can fit.
            refunds.extend(repeated(
                rng,
                &format!("evt_rb_{key}"),
                refund_processed(&format!("rfnd_{key}_b"), &pay, PARTIAL, None),
            ));
            vec![captured(rng), refunds]
        }
        Fate::FailedThenPaid => vec![
            failed_event(rng, &format!("payf_{key}"), "first"),
            captured(rng),
        ],
        Fate::PaidThenFailedLate => vec![captured(rng), failed_event(rng, &pay, "late")],
        Fate::FailedOnly => vec![failed_event(rng, &format!("payf_{key}"), "only")],
        Fate::MissedWebhook => vec![],
        Fate::WrongAmount => vec![repeated(
            rng,
            &format!("evt_w_{key}"),
            captured_with_fee(&pay, &remote, PRICE - 1_000, Some(fee)),
        )],
    };
    Planned {
        buyer,
        fate,
        pay,
        fee,
        stages,
    }
}

async fn soak(pool: PgPool, seed: u64, with_failover: bool) {
    let shop = if with_failover {
        shop_on_a_production_pool(pool).await
    } else {
        shop(pool).await
    };
    let mut rng = StdRng::seed_from_u64(seed);
    let mut planned: Vec<Planned> = Vec::new();
    for i in 0..FATES.len() * 2 {
        let buyer = if i == 0 {
            Buyer {
                id: shop.buyer_id,
                order: shop.order,
                remote: shop.remote.clone(),
            }
        } else {
            another_order(&shop, &format!("soak{seed}_{i}@chaos.in")).await
        };
        // Every fate at least once, then whatever the seed says.
        let fate = if let Some(fate) = FATES.get(i) {
            *fate
        } else {
            FATES[rng.gen_range(0..FATES.len())]
        };
        let fee = rng.gen_range(0..=2_500);
        planned.push(plan(&mut rng, buyer, fate, fee));
    }

    // Customers who paid at the provider without the webhook ever arriving.
    for p in planned.iter_mut().filter(|p| p.fate == Fate::MissedWebhook) {
        let payment = shop.provider.capture(&p.buyer.remote).unwrap();
        p.pay = payment.id.clone();
        p.fee = payment.fee_paise.expect("the fake provider reports a fee");
        sqlx::query(
            "update orders set created_at = now() - interval '25 minutes', \
                expires_at = now() + interval '5 minutes' where id = $1",
        )
        .bind(p.buyer.order)
        .execute(&shop.pool)
        .await
        .unwrap();
        // Half of them also get a late webhook, after reconciliation may have been there first.
        if rng.gen_bool(0.5) {
            let key = p.buyer.order.simple().to_string();
            p.stages.push(vec![(
                format!("evt_late_{key}"),
                captured_with_fee(&p.pay, &p.buyer.remote, PRICE, Some(p.fee)),
            )]);
        }
    }

    let traffic = Arc::new(Traffic::default());
    let failover = with_failover.then(|| Failover::start(&shop.pool, traffic.clone(), 3));
    let reconciling = Arc::new(AtomicBool::new(true));
    let reconciler = {
        let state = shop.app.state.clone();
        let running = reconciling.clone();
        tokio::spawn(async move {
            while running.load(Ordering::Relaxed) {
                let _ = run_once(&state).await; // errors are the point: it must be safe to be interrupted
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
    };

    let sent: Arc<Mutex<BTreeSet<String>>> = Arc::default();
    let starts: Vec<u64> = planned
        .iter()
        .map(|_| {
            if with_failover {
                wave(&mut rng)
            } else {
                rng.gen_range(0..50)
            }
        })
        .collect();
    join_all(planned.iter().zip(starts).map(|(p, start)| {
        let (shop, traffic) = (&shop, &traffic);
        let sent = sent.clone();
        async move {
            tokio::time::sleep(Duration::from_millis(start)).await;
            for stage in &p.stages {
                join_all(stage.iter().map(|(id, body)| {
                    let sent = sent.clone();
                    async move {
                        sent.lock().unwrap().insert(id.clone());
                        let res = deliver_until_accepted(shop, traffic, id, body.clone()).await;
                        assert!(res.status.is_success(), "{id}: {:?}", res.body);
                    }
                }))
                .await;
            }
        }
    }))
    .await;

    reconciling.store(false, Ordering::Relaxed);
    reconciler.await.unwrap();
    if let Some(failover) = failover {
        let strikes = failover.end();
        assert!(
            strikes >= 2,
            "the failover struck {strikes} times: it barely ran"
        );
        let interrupted = traffic.interrupted.load(Ordering::SeqCst);
        assert!(
            interrupted >= 2,
            "only {interrupted} deliveries were ever interrupted"
        );
    }
    // Reconciliation, undisturbed, finishes what the storm interrupted.
    for _ in 0..3 {
        sqlx::query("update orders set reconcile_after = null")
            .execute(&shop.pool)
            .await
            .unwrap();
        run_once(&shop.app.state)
            .await
            .expect("a quiet pass completes");
    }

    // ---- the model ---------------------------------------------------------------------------------------
    let mut expected_total = 0;
    for p in &planned {
        let at = format!("seed {seed}, {:?}, order {}", p.fate, p.buyer.order);
        let (status, access_now, net) = match p.fate {
            Fate::Paid | Fate::FailedThenPaid | Fate::PaidThenFailedLate | Fate::MissedWebhook => {
                ("paid", "allowed", PRICE - p.fee)
            }
            Fate::PaidThenRefunded => ("refunded", "access_revoked", -p.fee),
            Fate::PaidThenPartlyRefunded => ("paid", "allowed", PRICE - p.fee - PARTIAL),
            Fate::FailedOnly | Fate::WrongAmount => ("pending", "access_revoked", 0),
        };
        assert_eq!(order_state(&shop, &p.buyer).await, status, "{at}");
        let access_state = text(
            &shop,
            &format!(
                "select video_access_state('{}', '{}')",
                p.buyer.id, shop.video
            ),
        )
        .await;
        assert_eq!(access_state, access_now, "{at}");
        let creator_net = n(
            &shop,
            &format!(
                "select coalesce(sum(e.amount_paise), 0)::bigint from ledger_entries e \
                 join ledger_journals j on j.id = e.journal_id \
                 where j.order_id = '{}' and e.creator_id is not null",
                p.buyer.order
            ),
        )
        .await;
        assert_eq!(creator_net, net, "{at}: what the creator is owed");
        expected_total += net;
        let applied = n(&shop, &format!("select count(*) from payments where order_id = '{}' and status in ('confirmed','refunded')", p.buyer.order)).await;
        assert_eq!(
            applied,
            i64::from(status != "pending"),
            "{at}: payments that took effect"
        );
    }
    assert_eq!(
        n(&shop, "select coalesce(sum(amount_paise), 0)::bigint from ledger_entries where creator_id is not null").await,
        expected_total,
        "seed {seed}: the creator's total"
    );
    // Every event that was sent reached the record, once.
    let sent: Vec<String> = sent.lock().unwrap().iter().cloned().collect();
    let recorded: i64 =
        sqlx::query_scalar("select count(*) from webhook_events where provider_event_id = any($1)")
            .bind(&sent)
            .fetch_one(&shop.pool)
            .await
            .unwrap();
    assert_eq!(
        recorded,
        sent.len() as i64,
        "seed {seed}: events that never reached the record"
    );
    the_book_agrees(&shop.pool).await;
}

db_test! {
    async fn soak_everything_arriving_in_any_order_seed_1(pool: PgPool) {
        tokio::time::timeout(Duration::from_secs(240), soak(pool, 1, false)).await.expect("hung");
    }
}

db_test! {
    async fn soak_everything_arriving_in_any_order_seed_2(pool: PgPool) {
        tokio::time::timeout(Duration::from_secs(240), soak(pool, 2, false)).await.expect("hung");
    }
}

db_test! {
    async fn soak_everything_arriving_in_any_order_while_the_database_keeps_failing_over(pool: PgPool) {
        tokio::time::timeout(Duration::from_secs(300), soak(pool, 3, true)).await.expect("hung");
    }
}
