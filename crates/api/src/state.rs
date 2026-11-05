use std::sync::Arc;

use sqlx::PgPool;

use crate::auth::counters::CounterStore;
use crate::auth::password::Passwords;
use crate::auth::tokens::TokenKeys;
use crate::config::ApiConfig;
use crate::mail::{Mailer, OutgoingEmail};

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: Arc<ApiConfig>,
    pub counters: Arc<dyn CounterStore>,
    pub mailer: Arc<dyn Mailer>,
    pub passwords: Passwords,
    pub tokens: Arc<TokenKeys>,
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
        }
    }

    /// Sends in the background so response time doesn't reveal whether an email was sent
    /// (which would leak whether an account exists). Failures are logged, not surfaced.
    pub fn send_email(&self, email: OutgoingEmail) {
        let mailer = self.mailer.clone();
        tokio::spawn(async move {
            if let Err(err) = mailer.send(email).await {
                tracing::error!(error = ?err, "failed to send email");
            }
        });
    }
}
