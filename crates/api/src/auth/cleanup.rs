use std::time::Duration;

use crate::auth::repo;
use crate::state::AppState;
use crate::videos::service as videos;

const INTERVAL: Duration = Duration::from_secs(60 * 60);

/// Hourly sweep of expired sessions and one-time tokens. Safe to run from every instance at
/// once: the deletes are idempotent.
pub fn spawn_periodic(state: AppState) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(INTERVAL);
        loop {
            ticker.tick().await;
            match videos::abandon_stale_uploads(&state).await {
                Ok(0) => {}
                Ok(cleaned) => tracing::info!(cleaned, "cancelled abandoned video uploads"),
                Err(err) => tracing::error!(error = ?err, "abandoned-upload cleanup failed"),
            }
            match videos::purge_deleted(&state).await {
                Ok(0) => {}
                Ok(purged) => tracing::info!(purged, "removed deleted videos from storage"),
                Err(err) => tracing::error!(error = ?err, "deleted-video purge failed"),
            }
            match videos::purge_old_sources(&state).await {
                Ok(0) => {}
                Ok(purged) => tracing::info!(purged, "removed source uploads past retention"),
                Err(err) => tracing::error!(error = ?err, "source retention sweep failed"),
            }
            match crate::playback::repo::sweep(&state.db, crate::playback::service::KEEP_ENDED_DAYS)
                .await
            {
                Ok((0, 0)) => {}
                Ok((expired, forgotten)) => {
                    tracing::info!(expired, forgotten, "tidied playback sessions")
                }
                Err(err) => tracing::error!(error = ?err, "playback session sweep failed"),
            }
            match repo::purge_expired(&state.db).await {
                Ok(purged) if purged == repo::Purged::default() => {}
                Ok(purged) => {
                    tracing::info!(?purged, "purged expired rows");
                }
                Err(err) => tracing::error!(error = ?err, "auth cleanup failed"),
            }
        }
    });
}
