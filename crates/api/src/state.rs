use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use sqlx::PgPool;
use tokio::sync::Notify;

use crate::auth::counters::CounterStore;
use crate::auth::password::Passwords;
use crate::auth::tokens::TokenKeys;
use crate::config::ApiConfig;
use crate::mail::{Mailer, OutgoingEmail};

/// Fire-and-forget tasks that can still be waited for: shutdown drains them so queued
/// emails aren't dropped, and tests settle on them instead of sleeping.
#[derive(Clone, Default)]
pub struct Background {
    inflight: Arc<AtomicUsize>,
    idle: Arc<Notify>,
}

struct InflightGuard(Background);

impl Drop for InflightGuard {
    fn drop(&mut self) {
        if self.0.inflight.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.0.idle.notify_waiters();
        }
    }
}

impl Background {
    pub fn spawn<F>(&self, task: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.inflight.fetch_add(1, Ordering::SeqCst);
        let guard = InflightGuard(self.clone());
        tokio::spawn(async move {
            let _guard = guard;
            task.await;
        });
    }

    pub async fn idle(&self) {
        loop {
            let notified = self.idle.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.inflight.load(Ordering::SeqCst) == 0 {
                return;
            }
            notified.await;
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: Arc<ApiConfig>,
    pub counters: Arc<dyn CounterStore>,
    pub mailer: Arc<dyn Mailer>,
    pub passwords: Passwords,
    pub tokens: Arc<TokenKeys>,
    pub background: Background,
}

impl AppState {
    pub fn new(
        db: PgPool,
        config: ApiConfig,
        counters: Arc<dyn CounterStore>,
        mailer: Arc<dyn Mailer>,
        passwords: Passwords,
    ) -> Self {
        let tokens = Arc::new(TokenKeys::new(config.jwt_access_secret.as_bytes()));
        Self {
            db,
            config: Arc::new(config),
            counters,
            mailer,
            passwords,
            tokens,
            background: Background::default(),
        }
    }

    /// Sends in the background so response time doesn't reveal whether an email was sent
    /// (which would leak whether an account exists). Failures are logged, not surfaced.
    pub fn send_email(&self, email: OutgoingEmail) {
        let mailer = self.mailer.clone();
        self.background.spawn(async move {
            if let Err(err) = mailer.send(email).await {
                tracing::error!(error = ?err, "failed to send email");
            }
        });
    }
}
