mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::http::StatusCode;
use common::*;
use serde_json::json;
use sqlx::PgPool;
use zeno_api::mail::{Mailer, MemoryMailer, OutgoingEmail};
use zeno_api::outbox;

macro_rules! db_test {
    ($(#[$meta:meta])* async fn $name:ident($pool:ident: PgPool) $body:block) => {
        $(#[$meta])*
        #[sqlx::test(migrator = "zeno_core::db::MIGRATOR")]
        async fn $name($pool: PgPool) $body
    };
}

/// Fails the first `failures` sends, then delivers into `inner`.
struct FlakyMailer {
    failures_left: Mutex<usize>,
    inner: MemoryMailer,
}

impl FlakyMailer {
    fn new(failures: usize, inner: MemoryMailer) -> Arc<Self> {
        Arc::new(Self {
            failures_left: Mutex::new(failures),
            inner,
        })
    }
}

#[async_trait]
impl Mailer for FlakyMailer {
    async fn send(&self, email: OutgoingEmail) -> anyhow::Result<()> {
        {
            let mut left = self.failures_left.lock().unwrap();
            if *left > 0 {
                *left -= 1;
                anyhow::bail!("provider is down");
            }
        }
        self.inner.send(email).await
    }
}

/// Counts sends and is slow enough for concurrent workers to overlap.
struct SlowMailer {
    sent: Arc<AtomicUsize>,
}

#[async_trait]
impl Mailer for SlowMailer {
    async fn send(&self, _email: OutgoingEmail) -> anyhow::Result<()> {
        tokio::time::sleep(Duration::from_millis(100)).await;
        self.sent.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

async fn make_all_due(pool: &PgPool) {
    sqlx::query("update email_outbox set next_attempt_at = now() where sent_at is null and failed_at is null")
        .execute(pool)
        .await
        .unwrap();
}

#[derive(sqlx::FromRow, Debug)]
struct Row {
    attempts: i32,
    body: String,
    sent: bool,
    failed: bool,
    last_error: Option<String>,
}

async fn only_row(pool: &PgPool) -> Row {
    sqlx::query_as(
        "select attempts, body, sent_at is not null as sent, failed_at is not null as failed, last_error from email_outbox",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

db_test! {
    async fn an_email_survives_a_provider_outage_and_still_works_when_it_finally_arrives(pool: PgPool) {
        let delivered = MemoryMailer::default();
        let app = build_with_mailer(pool.clone(), FlakyMailer::new(2, delivered.clone()), delivered.clone());
        signup_verified_without_mail(&app).await;

        app.post("/v1/auth/forgot-password").json(json!({"email":"a@x.in"})).send().await;
        app.settle().await;
        let row = only_row(&pool).await;
        assert_eq!((row.attempts, row.sent, row.failed), (1, false, false));
        assert!(row.last_error.unwrap().contains("provider is down"));
        assert!(!row.body.is_empty(), "the link must be kept until it is delivered");
        assert!(delivered.sent().is_empty());

        make_all_due(&pool).await;
        outbox::dispatch_due(&app.state).await;
        assert!(delivered.sent().is_empty(), "second attempt fails too");

        make_all_due(&pool).await;
        outbox::dispatch_due(&app.state).await;
        let mails = delivered.sent();
        assert_eq!(mails.len(), 1);

        let reset = app
            .post("/v1/auth/reset-password")
            .json(json!({"token": token_from(&mails[0]), "new_password": "a-completely-new-passphrase"}))
            .send()
            .await;
        assert_eq!(reset.status, StatusCode::OK, "the delayed link is still good");
    }
}

/// Registers and verifies a user, then forgets the outbox rows those steps created so a test
/// can look at exactly one message.
async fn signup_verified_without_mail(app: &App) {
    signup_verified_via_db(app).await;
    sqlx::query("delete from email_outbox")
        .execute(&app.pool)
        .await
        .unwrap();
}

async fn signup_verified_via_db(app: &App) {
    let hash = app.state.passwords.hash(PASSWORD.to_owned()).await.unwrap();
    sqlx::query("insert into users (name, email, password_hash, email_verified_at) values ('Asha', 'a@x.in', $1, now())")
        .bind(hash)
        .execute(&app.pool)
        .await
        .unwrap();
}

db_test! {
    async fn the_link_is_wiped_from_the_database_once_the_email_is_sent(pool: PgPool) {
        let mailer = MemoryMailer::default();
        let app = build_with_mailer(pool.clone(), Arc::new(mailer.clone()), mailer.clone());
        signup_verified_without_mail(&app).await;

        app.post("/v1/auth/forgot-password").json(json!({"email":"a@x.in"})).send().await;
        mailer.wait_for(1).await;
        app.settle().await;

        let row = only_row(&pool).await;
        assert!(row.sent);
        assert_eq!(row.body, "", "no live reset link may stay in the table");
        assert_eq!(row.last_error, None);
    }
}

db_test! {
    async fn delivery_is_abandoned_after_eight_attempts_and_the_link_is_wiped(pool: PgPool) {
        let mailer = MemoryMailer::default();
        let app = build_with_mailer(pool.clone(), FlakyMailer::new(usize::MAX, mailer.clone()), mailer.clone());
        signup_verified_without_mail(&app).await;
        app.post("/v1/auth/forgot-password").json(json!({"email":"a@x.in"})).send().await;
        app.settle().await;

        for _ in 0..10 {
            make_all_due(&pool).await;
            outbox::dispatch_due(&app.state).await;
        }
        let row = only_row(&pool).await;
        assert!(row.failed && !row.sent);
        assert_eq!(row.attempts, 8, "stops at the cap instead of retrying forever");
        assert_eq!(row.body, "");
        assert!(mailer.sent().is_empty());
    }
}

db_test! {
    async fn a_link_that_expired_while_waiting_is_never_sent(pool: PgPool) {
        let mailer = MemoryMailer::default();
        let app = build_with_mailer(pool.clone(), FlakyMailer::new(1, mailer.clone()), mailer.clone());
        signup_verified_without_mail(&app).await;
        app.post("/v1/auth/forgot-password").json(json!({"email":"a@x.in"})).send().await;
        app.settle().await;

        sqlx::query("update email_outbox set expires_at = now() - interval '1 second'")
            .execute(&pool)
            .await
            .unwrap();
        make_all_due(&pool).await;
        outbox::dispatch_due(&app.state).await;

        let row = only_row(&pool).await;
        assert!(row.failed);
        assert_eq!(row.body, "");
        assert!(row.last_error.unwrap().contains("expired"));
        assert!(mailer.sent().is_empty());
    }
}

db_test! {
    async fn workers_racing_for_one_email_send_it_exactly_once(pool: PgPool) {
        let sent = Arc::new(AtomicUsize::new(0));
        let record = MemoryMailer::default();
        let app = build_with_mailer(pool.clone(), Arc::new(SlowMailer { sent: sent.clone() }), record);
        sqlx::query("insert into email_outbox (to_address, subject, body, expires_at) values ('a@x.in', 's', 'link', now() + interval '1 hour')")
            .execute(&pool)
            .await
            .unwrap();

        let workers: Vec<_> = (0..6)
            .map(|_| {
                let state = app.state.clone();
                tokio::spawn(async move { outbox::dispatch_due(&state).await })
            })
            .collect();
        for worker in workers {
            worker.await.unwrap();
        }
        assert_eq!(sent.load(Ordering::SeqCst), 1);
        assert!(only_row(&pool).await.sent);
    }
}

db_test! {
    async fn a_row_leased_by_a_crashed_worker_is_picked_up_after_the_lease(pool: PgPool) {
        let mailer = MemoryMailer::default();
        let app = build_with_mailer(pool.clone(), Arc::new(mailer.clone()), mailer.clone());
        // A worker claimed this row and died: it is hidden until the lease runs out.
        sqlx::query("insert into email_outbox (to_address, subject, body, expires_at, next_attempt_at) values ('a@x.in', 's', 'link', now() + interval '1 hour', now() + interval '2 minutes')")
            .execute(&pool)
            .await
            .unwrap();

        outbox::dispatch_due(&app.state).await;
        assert!(mailer.sent().is_empty(), "still leased");

        sqlx::query("update email_outbox set next_attempt_at = now() - interval '1 second'")
            .execute(&pool)
            .await
            .unwrap();
        outbox::dispatch_due(&app.state).await;
        assert_eq!(mailer.sent().len(), 1);
    }
}

db_test! {
    async fn signing_up_still_succeeds_while_the_mail_provider_is_down(pool: PgPool) {
        let mailer = MemoryMailer::default();
        let app = build_with_mailer(pool.clone(), FlakyMailer::new(usize::MAX, mailer.clone()), mailer.clone());
        let res = app.post("/v1/auth/register").json(register_body("a@x.in")).send().await;
        assert_eq!(res.status, StatusCode::ACCEPTED);
        app.settle().await;
        assert_eq!(count(&pool, "select count(*) from users").await, 1);
        let row = only_row(&pool).await;
        assert!(!row.sent && !row.failed && row.attempts == 1);
    }
}
