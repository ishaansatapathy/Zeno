use std::net::IpAddr;
use std::time::Duration;

use crate::auth::counters::CounterStore;
use crate::error::ApiError;
use crate::net::limit_bucket;

pub const HOUR: Duration = Duration::from_secs(60 * 60);
const LOGIN_WINDOW: Duration = Duration::from_secs(15 * 60);
/// Failures allowed for one (email, address) pair. Keying on the pair means a stranger
/// cannot lock a victim out of their own account just by guessing at it.
const LOGIN_PAIR_LIMIT: u64 = 5;
/// Failures allowed from one address across all accounts, which stops password spraying.
const LOGIN_IP_LIMIT: u64 = 30;

/// Counts a hit and returns `Some(retry_after_secs)` once over the limit. Authentication and
/// upload write budgets fail closed when Redis is unavailable.
pub async fn hit(
    store: &dyn CounterStore,
    key: &str,
    limit: u64,
    window: Duration,
) -> Result<Option<u64>, ApiError> {
    let count = store.incr(key, window).await.map_err(|err| {
        tracing::error!(error = ?err, "auth counter unavailable; failing closed");
        ApiError::Unavailable("rate limiting is unavailable")
    })?;
    if count <= limit {
        return Ok(None);
    }
    Ok(Some(retry_after(store, key, window).await))
}

pub async fn enforce(
    store: &dyn CounterStore,
    key: &str,
    limit: u64,
    window: Duration,
) -> Result<(), ApiError> {
    match hit(store, key, limit, window).await? {
        None => Ok(()),
        Some(retry_after_secs) => Err(ApiError::TooManyRequests { retry_after_secs }),
    }
}

async fn retry_after(store: &dyn CounterStore, key: &str, window: Duration) -> u64 {
    store
        .ttl(key)
        .await
        .ok()
        .flatten()
        .unwrap_or(window)
        .as_secs()
        .max(1)
}

fn pair_key(email: &str, ip: Option<IpAddr>) -> String {
    format!("login:pair:{email}|{}", limit_bucket(ip))
}

fn ip_key(ip: Option<IpAddr>) -> String {
    format!("login:ip:{}", limit_bucket(ip))
}

pub async fn ensure_login_allowed(
    store: &dyn CounterStore,
    email: &str,
    ip: Option<IpAddr>,
) -> Result<(), ApiError> {
    for (key, limit) in [
        (pair_key(email, ip), LOGIN_PAIR_LIMIT),
        (ip_key(ip), LOGIN_IP_LIMIT),
    ] {
        match store.get(&key).await {
            Ok(count) if count >= limit => {
                return Err(ApiError::TooManyRequests {
                    retry_after_secs: retry_after(store, &key, LOGIN_WINDOW).await,
                });
            }
            Ok(_) => {}
            Err(err) => {
                tracing::error!(error = ?err, "login guard unavailable; failing closed");
                return Err(ApiError::Unavailable("rate limiting is unavailable"));
            }
        }
    }
    Ok(())
}

pub async fn record_login_failure(
    store: &dyn CounterStore,
    email: &str,
    ip: Option<IpAddr>,
) -> Result<(), ApiError> {
    for key in [pair_key(email, ip), ip_key(ip)] {
        store.incr(&key, LOGIN_WINDOW).await.map_err(|err| {
            tracing::error!(error = ?err, "could not record login failure");
            ApiError::Unavailable("rate limiting is unavailable")
        })?;
    }
    Ok(())
}

pub async fn clear_login_failures(store: &dyn CounterStore, email: &str, ip: Option<IpAddr>) {
    if let Err(err) = store.clear(&pair_key(email, ip)).await {
        tracing::error!(error = ?err, "could not clear login failures");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::counters::MemoryCounters;

    struct FailedCounters;

    #[async_trait::async_trait]
    impl CounterStore for FailedCounters {
        async fn incr(&self, _: &str, _: Duration) -> anyhow::Result<u64> {
            anyhow::bail!("redis down")
        }
        async fn get(&self, _: &str) -> anyhow::Result<u64> {
            anyhow::bail!("redis down")
        }
        async fn ttl(&self, _: &str) -> anyhow::Result<Option<Duration>> {
            anyhow::bail!("redis down")
        }
        async fn clear(&self, _: &str) -> anyhow::Result<()> {
            anyhow::bail!("redis down")
        }
        async fn ping(&self) -> anyhow::Result<()> {
            anyhow::bail!("redis down")
        }
    }

    #[tokio::test]
    async fn auth_budgets_fail_closed_when_redis_is_down() {
        let store = FailedCounters;
        assert!(matches!(
            hit(&store, "register", 10, HOUR).await,
            Err(ApiError::Unavailable(_))
        ));
        assert!(matches!(
            ensure_login_allowed(&store, "x@example.com", None).await,
            Err(ApiError::Unavailable(_))
        ));
        assert!(matches!(
            record_login_failure(&store, "x@example.com", None).await,
            Err(ApiError::Unavailable(_))
        ));
    }

    fn ip(s: &str) -> Option<IpAddr> {
        Some(s.parse().unwrap())
    }

    #[tokio::test]
    async fn hit_allows_up_to_the_limit_then_blocks() {
        let store = MemoryCounters::default();
        let w = Duration::from_secs(60);
        for _ in 0..3 {
            assert_eq!(hit(&store, "k", 3, w).await.unwrap(), None);
        }
        assert!(hit(&store, "k", 3, w).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn pair_lockout_hits_after_five_failures_without_affecting_other_emails() {
        let store = MemoryCounters::default();
        let a = ip("1.1.1.1");
        for _ in 0..5 {
            ensure_login_allowed(&store, "victim@x.in", a)
                .await
                .unwrap();
            record_login_failure(&store, "victim@x.in", a)
                .await
                .unwrap();
        }
        assert!(matches!(
            ensure_login_allowed(&store, "victim@x.in", a).await,
            Err(ApiError::TooManyRequests { .. })
        ));
        ensure_login_allowed(&store, "other@x.in", a).await.unwrap();
    }

    #[tokio::test]
    async fn attacker_cannot_lock_the_victim_out_from_a_different_address() {
        let store = MemoryCounters::default();
        for _ in 0..5 {
            record_login_failure(&store, "victim@x.in", ip("6.6.6.6"))
                .await
                .unwrap();
        }
        ensure_login_allowed(&store, "victim@x.in", ip("1.1.1.1"))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn spraying_many_accounts_from_one_address_is_blocked() {
        let store = MemoryCounters::default();
        let attacker = ip("6.6.6.6");
        for i in 0..30 {
            record_login_failure(&store, &format!("user{i}@x.in"), attacker)
                .await
                .unwrap();
        }
        assert!(ensure_login_allowed(&store, "fresh@x.in", attacker)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn success_clears_the_pair_counter() {
        let store = MemoryCounters::default();
        let a = ip("1.1.1.1");
        for _ in 0..4 {
            record_login_failure(&store, "me@x.in", a).await.unwrap();
        }
        clear_login_failures(&store, "me@x.in", a).await;
        for _ in 0..4 {
            record_login_failure(&store, "me@x.in", a).await.unwrap();
        }
        ensure_login_allowed(&store, "me@x.in", a).await.unwrap();
    }

    #[tokio::test]
    async fn retry_after_is_reported_in_seconds_and_at_least_one() {
        let store = MemoryCounters::default();
        let w = Duration::from_secs(90);
        hit(&store, "k", 0, w).await.unwrap();
        let retry = hit(&store, "k", 0, w).await.unwrap().unwrap();
        assert!((1..=90).contains(&retry));
    }
}
