use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;

/// Expiring counters, the storage behind login lockout and rate limits.
#[async_trait]
pub trait CounterStore: Send + Sync {
    /// Adds one and returns the new count. The window starts on the first increment and is
    /// not extended by later ones, so a steady trickle cannot keep a counter alive forever.
    async fn incr(&self, key: &str, window: Duration) -> anyhow::Result<u64>;
    async fn get(&self, key: &str) -> anyhow::Result<u64>;
    async fn ttl(&self, key: &str) -> anyhow::Result<Option<Duration>>;
    async fn clear(&self, key: &str) -> anyhow::Result<()>;
    async fn ping(&self) -> anyhow::Result<()>;
}

#[derive(Default)]
pub struct MemoryCounters {
    entries: Mutex<HashMap<String, (u64, Instant)>>,
}

impl MemoryCounters {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, (u64, Instant)>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[async_trait]
impl CounterStore for MemoryCounters {
    async fn incr(&self, key: &str, window: Duration) -> anyhow::Result<u64> {
        let now = Instant::now();
        let mut entries = self.lock();
        let entry = entries
            .entry(key.to_owned())
            .and_modify(|(count, expires)| {
                if *expires <= now {
                    *count = 0;
                    *expires = now + window;
                }
            })
            .or_insert((0, now + window));
        entry.0 += 1;
        Ok(entry.0)
    }

    async fn get(&self, key: &str) -> anyhow::Result<u64> {
        let now = Instant::now();
        Ok(self
            .lock()
            .get(key)
            .filter(|(_, expires)| *expires > now)
            .map_or(0, |(count, _)| *count))
    }

    async fn ttl(&self, key: &str) -> anyhow::Result<Option<Duration>> {
        let now = Instant::now();
        Ok(self
            .lock()
            .get(key)
            .and_then(|(_, expires)| expires.checked_duration_since(now)))
    }

    async fn clear(&self, key: &str) -> anyhow::Result<()> {
        self.lock().remove(key);
        Ok(())
    }

    async fn ping(&self) -> anyhow::Result<()> {
        Ok(())
    }
}

const INCR_WITH_WINDOW: &str = r#"
local count = redis.call('INCR', KEYS[1])
if count == 1 then
  redis.call('PEXPIRE', KEYS[1], ARGV[1])
end
return count
"#;

pub struct RedisCounters {
    conn: redis::aio::ConnectionManager,
    incr: redis::Script,
}

impl RedisCounters {
    pub fn new(conn: redis::aio::ConnectionManager) -> Self {
        Self {
            conn,
            incr: redis::Script::new(INCR_WITH_WINDOW),
        }
    }

    fn key(key: &str) -> String {
        format!("zeno:ctr:{key}")
    }
}

#[async_trait]
impl CounterStore for RedisCounters {
    async fn incr(&self, key: &str, window: Duration) -> anyhow::Result<u64> {
        let mut conn = self.conn.clone();
        let millis = u64::try_from(window.as_millis()).unwrap_or(u64::MAX);
        let count: u64 = self
            .incr
            .key(Self::key(key))
            .arg(millis)
            .invoke_async(&mut conn)
            .await?;
        Ok(count)
    }

    async fn get(&self, key: &str) -> anyhow::Result<u64> {
        let mut conn = self.conn.clone();
        let count: Option<u64> = redis::cmd("GET")
            .arg(Self::key(key))
            .query_async(&mut conn)
            .await?;
        Ok(count.unwrap_or(0))
    }

    async fn ttl(&self, key: &str) -> anyhow::Result<Option<Duration>> {
        let mut conn = self.conn.clone();
        let millis: i64 = redis::cmd("PTTL")
            .arg(Self::key(key))
            .query_async(&mut conn)
            .await?;
        Ok(u64::try_from(millis)
            .ok()
            .filter(|m| *m > 0)
            .map(Duration::from_millis))
    }

    async fn clear(&self, key: &str) -> anyhow::Result<()> {
        let mut conn = self.conn.clone();
        let _: i64 = redis::cmd("DEL")
            .arg(Self::key(key))
            .query_async(&mut conn)
            .await?;
        Ok(())
    }

    async fn ping(&self) -> anyhow::Result<()> {
        let mut conn = self.conn.clone();
        let _: String = redis::cmd("PING").query_async(&mut conn).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn memory_counts_up_and_clears() {
        let store = MemoryCounters::default();
        let window = Duration::from_secs(60);
        assert_eq!(store.get("k").await.unwrap(), 0);
        assert_eq!(store.incr("k", window).await.unwrap(), 1);
        assert_eq!(store.incr("k", window).await.unwrap(), 2);
        assert_eq!(store.get("k").await.unwrap(), 2);
        assert!(store.ttl("k").await.unwrap().unwrap() <= window);
        store.clear("k").await.unwrap();
        assert_eq!(store.get("k").await.unwrap(), 0);
        assert_eq!(store.ttl("k").await.unwrap(), None);
    }

    #[tokio::test]
    async fn memory_window_expires_and_restarts() {
        let store = MemoryCounters::default();
        let window = Duration::from_millis(40);
        assert_eq!(store.incr("k", window).await.unwrap(), 1);
        assert_eq!(store.incr("k", window).await.unwrap(), 2);
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(store.get("k").await.unwrap(), 0);
        assert_eq!(store.incr("k", window).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn memory_keys_are_independent() {
        let store = MemoryCounters::default();
        let window = Duration::from_secs(60);
        store.incr("a", window).await.unwrap();
        store.incr("a", window).await.unwrap();
        assert_eq!(store.incr("b", window).await.unwrap(), 1);
    }

    /// Runs against a real Redis when TEST_REDIS_URL is set (CI sets it).
    #[tokio::test]
    async fn redis_matches_memory_semantics() {
        let Ok(url) = std::env::var("TEST_REDIS_URL") else {
            eprintln!("skipped: TEST_REDIS_URL not set");
            return;
        };
        let client = redis::Client::open(url).unwrap();
        let conn = redis::aio::ConnectionManager::new(client).await.unwrap();
        let store = RedisCounters::new(conn);
        let key = format!("test:{}", uuid::Uuid::new_v4());
        let window = Duration::from_millis(300);

        store.ping().await.unwrap();
        assert_eq!(store.get(&key).await.unwrap(), 0);
        assert_eq!(store.incr(&key, window).await.unwrap(), 1);
        assert_eq!(store.incr(&key, window).await.unwrap(), 2);
        assert!(store.ttl(&key).await.unwrap().is_some());
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert_eq!(store.get(&key).await.unwrap(), 0);
        assert_eq!(store.incr(&key, window).await.unwrap(), 1);
        store.clear(&key).await.unwrap();
        assert_eq!(store.get(&key).await.unwrap(), 0);
    }
}
