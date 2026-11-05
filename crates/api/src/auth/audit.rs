//! Append-only trail of security-relevant account activity, so "what happened on this
//! account?" can be answered from the database instead of grepping logs.

use std::net::IpAddr;

use sha2::{Digest, Sha256};
use sqlx::types::ipnetwork::IpNetwork;
use sqlx::PgPool;
use uuid::Uuid;

use crate::auth::service::ClientContext;
use crate::state::AppState;

const MAX_USER_AGENT_CHARS: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    SignedUp,
    EmailVerified,
    LoginSucceeded,
    LoginFailed,
    LoginBlocked,
    LoginUnverified,
    RefreshReuseDetected,
    LoggedOut,
    PasswordResetRequested,
    PasswordResetCompleted,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::SignedUp => "signed_up",
            Kind::EmailVerified => "email_verified",
            Kind::LoginSucceeded => "login_succeeded",
            Kind::LoginFailed => "login_failed",
            Kind::LoginBlocked => "login_blocked",
            Kind::LoginUnverified => "login_unverified",
            Kind::RefreshReuseDetected => "refresh_reuse_detected",
            Kind::LoggedOut => "logged_out",
            Kind::PasswordResetRequested => "password_reset_requested",
            Kind::PasswordResetCompleted => "password_reset_completed",
        }
    }
}

pub struct Event {
    kind: Kind,
    user_id: Option<Uuid>,
    email_hash: Option<Vec<u8>>,
    family_id: Option<Uuid>,
    ip: Option<IpAddr>,
    user_agent: Option<String>,
}

impl Event {
    pub fn new(kind: Kind, ctx: &ClientContext) -> Self {
        Self {
            kind,
            user_id: None,
            email_hash: None,
            family_id: None,
            ip: ctx.ip,
            user_agent: ctx
                .user_agent
                .as_ref()
                .map(|ua| ua.chars().take(MAX_USER_AGENT_CHARS).collect()),
        }
    }

    pub fn user(mut self, user_id: Uuid) -> Self {
        self.user_id = Some(user_id);
        self
    }

    pub fn maybe_user(mut self, user_id: Option<Uuid>) -> Self {
        self.user_id = user_id;
        self
    }

    /// Only a SHA-256 of the normalised address is kept. That is pseudonymous rather than
    /// anonymous (a known address can be hashed and looked up), which is the point: it lets
    /// failed attempts against one address be correlated without storing the address itself.
    pub fn email(mut self, normalised_email: &str) -> Self {
        self.email_hash = Some(Sha256::digest(normalised_email.as_bytes()).to_vec());
        self
    }

    pub fn family(mut self, family_id: Uuid) -> Self {
        self.family_id = Some(family_id);
        self
    }
}

/// Records in the background so auditing never slows or breaks the request it describes;
/// a failure to write is logged loudly instead.
pub fn record(state: &AppState, event: Event) {
    let db = state.db.clone();
    state.background.spawn(async move {
        if let Err(err) = insert(&db, &event).await {
            tracing::error!(error = ?err, kind = event.kind.as_str(), "could not write audit event");
        }
    });
}

async fn insert(db: &PgPool, event: &Event) -> Result<(), sqlx::Error> {
    sqlx::query(
        "insert into auth_events (user_id, email_hash, kind, family_id, ip, user_agent) \
         values ($1, $2, $3, $4, $5, $6)",
    )
    .bind(event.user_id)
    .bind(event.email_hash.as_deref())
    .bind(event.kind.as_str())
    .bind(event.family_id)
    .bind(event.ip.map(IpNetwork::from))
    .bind(event.user_agent.as_deref())
    .execute(db)
    .await?;
    Ok(())
}

#[derive(Debug, sqlx::FromRow)]
pub struct EventRow {
    pub kind: String,
    pub ip: Option<IpNetwork>,
    pub user_agent: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

pub async fn recent_for_user(
    db: &PgPool,
    user_id: Uuid,
    limit: i64,
) -> Result<Vec<EventRow>, sqlx::Error> {
    sqlx::query_as(
        "select kind, ip, user_agent, created_at from auth_events \
         where user_id = $1 order by created_at desc, id desc limit $2",
    )
    .bind(user_id)
    .bind(limit)
    .fetch_all(db)
    .await
}
