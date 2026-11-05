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

/// Counts a hit and returns `Some(retry_after_secs)` once over the limit.
///
/// If the counter store is down this fails open and logs loudly: locking every user out of
/// sign-in because Redis is unavailable is the worse trade, and Argon2 cost still bounds
/// guessing speed.
pub async fn hit(store: &dyn CounterStore, key: &str, limit: u64, window: Duration) -> Option<u64> {
    let count = match store.incr(key, window).await {
        Ok(count) => count,
        Err(err) => {
            tracing::error!(error = ?err, "rate limiter unavailable; failing open");
            return None;
        }
    };
    if count <= limit {
        return None;
    }
    Some(retry_after(store, key, window).await)
}

pub async fn enforce(
    store: &dyn CounterStore,
    key: &str,
    limit: u64,
    window: Duration,
) -> Result<(), ApiError> {
    match hit(store, key, limit, window).await {
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
            Err(err) => tracing::error!(error = ?err, "login guard unavailable; failing open"),
        }
    }
    Ok(())
}

pub async fn record_login_failure(store: &dyn CounterStore, email: &str, ip: Option<IpAddr>) {
    for key in [pair_key(email, ip), ip_key(ip)] {
        if let Err(err) = store.incr(&key, LOGIN_WINDOW).await {
            tracing::error!(error = ?err, "could not record login failure");
        }
    }
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

    fn ip(s: &str) -> Option<IpAddr> {
        Some(s.parse().unwrap())
    }

    #[tokio::test]
    async fn hit_allows_up_to_the_limit_then_blocks() {
        let store = MemoryCounters::default();
        let w = Duration::from_secs(60);
        for _ in 0..3 {
            assert_eq!(hit(&store, "k", 3, w).await, None);
        }
        assert!(hit(&store, "k", 3, w).await.is_some());
    }

    #[tokio::test]
    async fn pair_lockout_hits_after_five_failures_without_affecting_other_emails() {
        let store = MemoryCounters::default();
        let a = ip("1.1.1.1");
        for _ in 0..5 {
            ensure_login_allowed(&store, "victim@x.in", a)
                .await
                .unwrap();
            record_login_failure(&store, "victim@x.in", a).await;
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
            record_login_failure(&store, "victim@x.in", ip("6.6.6.6")).await;
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
            record_login_failure(&store, &format!("user{i}@x.in"), attacker).await;
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
            record_login_failure(&store, "me@x.in", a).await;
        }
        clear_login_failures(&store, "me@x.in", a).await;
        for _ in 0..4 {
            record_login_failure(&store, "me@x.in", a).await;
        }
        ensure_login_allowed(&store, "me@x.in", a).await.unwrap();
    }

    #[tokio::test]
    async fn retry_after_is_reported_in_seconds_and_at_least_one() {
        let store = MemoryCounters::default();
        let w = Duration::from_secs(90);
        hit(&store, "k", 0, w).await;
        let retry = hit(&store, "k", 0, w).await.unwrap();
        assert!((1..=90).contains(&retry));
    }
}
