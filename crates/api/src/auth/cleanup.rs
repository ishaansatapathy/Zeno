use std::time::Duration;

use sqlx::PgPool;

use crate::auth::repo;

const INTERVAL: Duration = Duration::from_secs(60 * 60);

/// Hourly sweep of expired sessions and one-time tokens. Safe to run from every instance at
/// once: the deletes are idempotent.
pub fn spawn_periodic(db: PgPool) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(INTERVAL);
        loop {
            ticker.tick().await;
            match repo::purge_expired(&db).await {
                Ok((0, 0)) => {}
                Ok((sessions, tokens)) => {
                    tracing::info!(sessions, tokens, "purged expired auth rows");
                }
                Err(err) => tracing::error!(error = ?err, "auth cleanup failed"),
            }
        }
    });
}
