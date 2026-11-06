use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct OutgoingEmail {
    pub to: String,
    pub subject: String,
    pub text: String,
}

#[async_trait]
pub trait Mailer: Send + Sync {
    async fn send(&self, email: OutgoingEmail) -> anyhow::Result<()>;
}

/// Development transport: prints the message so verification and reset links can be
/// clicked from the terminal. Emails contain live tokens, so this must never run in
/// production (`main` refuses to start in production until a real transport exists).
pub struct LogMailer;

#[async_trait]
impl Mailer for LogMailer {
    async fn send(&self, email: OutgoingEmail) -> anyhow::Result<()> {
        tracing::info!(
            to = %email.to,
            subject = %email.subject,
            "DEV EMAIL (not delivered)\n{}",
            email.text
        );
        Ok(())
    }
}

/// Collects messages in memory; used by tests.
#[derive(Default, Clone)]
pub struct MemoryMailer {
    sent: Arc<Mutex<Vec<OutgoingEmail>>>,
}

impl MemoryMailer {
    pub fn sent(&self) -> Vec<OutgoingEmail> {
        self.sent.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Emails are sent from background tasks, so tests poll instead of assuming timing.
    pub async fn wait_for(&self, count: usize) -> Vec<OutgoingEmail> {
        for _ in 0..1000 {
            let sent = self.sent();
            if sent.len() >= count {
                return sent;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!(
            "expected {count} email(s) but only {} arrived",
            self.sent().len()
        );
    }
}

#[async_trait]
impl Mailer for MemoryMailer {
    async fn send(&self, email: OutgoingEmail) -> anyhow::Result<()> {
        self.sent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(email);
        Ok(())
    }
}
