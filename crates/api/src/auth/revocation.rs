use std::time::Duration;

use uuid::Uuid;

use crate::auth::counters::CounterStore;
use crate::auth::tokens::ACCESS_TTL_MINUTES;

/// A revoked family only needs remembering until the last access token it issued has
/// expired; after that the signature check alone rejects it.
const MARKER_TTL: Duration = Duration::from_secs((ACCESS_TTL_MINUTES as u64 + 1) * 60);

fn key(family_id: Uuid) -> String {
    format!("revoked:family:{family_id}")
}

/// Makes already-issued access tokens from this session family stop working immediately,
/// instead of lingering until they expire. Best effort: if the store is unreachable the
/// refresh token is still revoked in Postgres, so the damage is bounded to the token TTL.
pub async fn mark_revoked(store: &dyn CounterStore, family_id: Uuid) {
    if let Err(err) = store.incr(&key(family_id), MARKER_TTL).await {
        tracing::error!(error = ?err, "could not record session revocation");
    }
}

pub async fn is_revoked(store: &dyn CounterStore, family_id: Uuid) -> bool {
    match store.get(&key(family_id)).await {
        Ok(count) => count > 0,
        Err(err) => {
            tracing::error!(error = ?err, "revocation check unavailable; failing open");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::counters::MemoryCounters;

    #[tokio::test]
    async fn only_marked_families_are_revoked() {
        let store = MemoryCounters::default();
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        assert!(!is_revoked(&store, a).await);
        mark_revoked(&store, a).await;
        assert!(is_revoked(&store, a).await);
        assert!(!is_revoked(&store, b).await);
    }
}
