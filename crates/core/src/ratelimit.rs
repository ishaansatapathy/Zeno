//! A shared rate limiter for every Zeno service.
//!
//! It is a *sliding-window counter*: each key keeps the request counts of the current and the
//! previous fixed window, and a request is allowed while
//!
//! ```text
//! previous * (1 - elapsed_fraction_of_current_window) + current + 1 <= limit
//! ```
//!
//! That is the accuracy of a true sliding log at the cost of two integers per key, and unlike a
//! plain fixed window it cannot be gamed by sending a full allowance at the end of one window and
//! another at the start of the next. The check and the increment are one atomic Redis script, so
//! any number of API instances share one budget.
//!
//! The arithmetic is integer-only and lives in one place ([`allowed`]); the Lua script repeats
//! exactly that inequality, and tests run both backends through the same scenarios.
//!
//! Callers choose what happens when the backend is unreachable: this crate only reports
//! [`LimitError`], because "let it through" is right for a public catalogue and wrong for a key
//! server.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// At most `limit` requests per `window`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rule {
    pub limit: u32,
    pub window: Duration,
}

impl Rule {
    pub const fn new(limit: u32, window: Duration) -> Self {
        Self { limit, window }
    }

    pub const fn per_minute(limit: u32) -> Self {
        Self::new(limit, Duration::from_secs(60))
    }

    pub const fn per_hour(limit: u32) -> Self {
        Self::new(limit, Duration::from_secs(3600))
    }

    pub const fn per_day(limit: u32) -> Self {
        Self::new(limit, Duration::from_secs(86_400))
    }

    fn window_ms(&self) -> u64 {
        u64::try_from(self.window.as_millis())
            .unwrap_or(u64::MAX / 4)
            .max(1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    pub allowed: bool,
    /// Requests still available right now (0 when refused).
    pub remaining: u32,
    /// How long until a refused request would be allowed, assuming no other traffic. Zero when
    /// allowed.
    pub retry_after: Duration,
}

#[derive(Debug, thiserror::Error)]
#[error("rate limit backend unavailable: {0}")]
pub struct LimitError(pub String);

/// The one inequality. `elapsed` is how far into the current window we are, in the same unit as
/// `window`.
fn allowed(limit: u64, window: u64, elapsed: u64, current: u64, previous: u64) -> bool {
    previous * (window - elapsed) + (current + 1) * window <= limit * window
}

/// Milliseconds until [`allowed`] would hold, with no further requests. Never zero for a request
/// that was refused.
fn retry_after_ms(limit: u64, window: u64, elapsed: u64, current: u64, previous: u64) -> u64 {
    if limit == 0 {
        return window;
    }
    // Still inside this window: the previous window's weight shrinks linearly. We need
    // previous * (window - x) / window <= limit - current - 1, with x the elapsed time.
    if previous > 0 && current < limit {
        let spare = limit - current - 1;
        // x >= window - spare * window / previous (rounded so the result is on the safe side)
        let needed = window.saturating_sub(spare * window / previous);
        if needed <= window {
            return needed.saturating_sub(elapsed).max(1);
        }
    }
    // Otherwise wait for the next window, whose "previous" is this window's count.
    let into_next = window.saturating_sub(
        ((limit - 1) * window)
            .checked_div(current)
            .unwrap_or(window),
    );
    (window - elapsed + into_next).clamp(1, window * 2)
}

fn decide(limit: u64, window: u64, elapsed: u64, current: u64, previous: u64) -> Decision {
    // `current` is the count after this request when it was allowed.
    let weighted = previous * (window - elapsed) + current * window;
    Decision {
        allowed: true,
        remaining: u32::try_from(limit.saturating_sub(weighted.div_ceil(window))).unwrap_or(0),
        retry_after: Duration::ZERO,
    }
}

const SCRIPT: &str = r#"
local window = tonumber(ARGV[1])
local limit = tonumber(ARGV[2])
local elapsed = tonumber(ARGV[3])
local current = tonumber(redis.call('GET', KEYS[1]) or '0')
local previous = tonumber(redis.call('GET', KEYS[2]) or '0')
if previous * (window - elapsed) + (current + 1) * window <= limit * window then
  current = redis.call('INCR', KEYS[1])
  if current == 1 then
    redis.call('PEXPIRE', KEYS[1], window * 2)
  end
  return {1, current, previous}
end
return {0, current, previous}
"#;

type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

#[derive(Clone)]
pub struct RateLimiter {
    backend: Arc<Backend>,
    clock: Clock,
}

enum Backend {
    Memory(Mutex<HashMap<String, Windows>>),
    Redis(Box<RedisBackend>),
}

struct RedisBackend {
    conn: redis::aio::ConnectionManager,
    script: redis::Script,
}

/// The two counters of one key, valid for the window index `index`.
struct Windows {
    index: u64,
    window: u64,
    current: u64,
    previous: u64,
}

fn system_clock() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

impl RateLimiter {
    /// Per-process counters: for tests and single-instance development.
    pub fn memory() -> Self {
        Self::memory_with_clock(system_clock)
    }

    /// As [`RateLimiter::memory`] with the clock (milliseconds since the Unix epoch) supplied by
    /// the caller, so tests can move time deliberately.
    pub fn memory_with_clock(clock: impl Fn() -> u64 + Send + Sync + 'static) -> Self {
        Self {
            backend: Arc::new(Backend::Memory(Mutex::new(HashMap::new()))),
            clock: Arc::new(clock),
        }
    }

    pub fn redis(conn: redis::aio::ConnectionManager) -> Self {
        Self::redis_with_clock(conn, system_clock)
    }

    pub fn redis_with_clock(
        conn: redis::aio::ConnectionManager,
        clock: impl Fn() -> u64 + Send + Sync + 'static,
    ) -> Self {
        Self {
            backend: Arc::new(Backend::Redis(Box::new(RedisBackend {
                conn,
                script: redis::Script::new(SCRIPT),
            }))),
            clock: Arc::new(clock),
        }
    }

    /// Counts one request against `key` (include the policy name in it) and says whether it fits.
    pub async fn check(&self, key: &str, rule: Rule) -> Result<Decision, LimitError> {
        let window = rule.window_ms();
        let limit = u64::from(rule.limit);
        let now = (self.clock)();
        let index = now / window;
        let elapsed = now % window;

        let (granted, current, previous) = match self.backend.as_ref() {
            Backend::Memory(map) => {
                let mut map = map.lock().unwrap_or_else(|e| e.into_inner());
                if map.len() > 50_000 {
                    map.retain(|_, w| (w.index + 2) * w.window > now);
                }
                let entry = map.entry(key.to_owned()).or_insert(Windows {
                    index,
                    window,
                    current: 0,
                    previous: 0,
                });
                match index.saturating_sub(entry.index) {
                    0 => {}
                    1 => {
                        entry.previous = entry.current;
                        entry.current = 0;
                        entry.index = index;
                    }
                    _ => {
                        entry.previous = 0;
                        entry.current = 0;
                        entry.index = index;
                    }
                }
                entry.window = window;
                if allowed(limit, window, elapsed, entry.current, entry.previous) {
                    entry.current += 1;
                    (true, entry.current, entry.previous)
                } else {
                    (false, entry.current, entry.previous)
                }
            }
            Backend::Redis(redis) => {
                let mut conn = redis.conn.clone();
                let result: Vec<u64> = redis
                    .script
                    .key(format!("zeno:rl:{{{key}}}:{index}"))
                    .key(format!("zeno:rl:{{{key}}}:{}", index.wrapping_sub(1)))
                    .arg(window)
                    .arg(limit)
                    .arg(elapsed)
                    .invoke_async(&mut conn)
                    .await
                    .map_err(|e| LimitError(e.to_string()))?;
                match result.as_slice() {
                    [granted, current, previous] => (*granted == 1, *current, *previous),
                    other => return Err(LimitError(format!("unexpected script reply {other:?}"))),
                }
            }
        };

        Ok(if granted {
            decide(limit, window, elapsed, current, previous)
        } else {
            Decision {
                allowed: false,
                remaining: 0,
                retry_after: Duration::from_millis(retry_after_ms(
                    limit, window, elapsed, current, previous,
                )),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Time(Arc<AtomicU64>);

    impl Time {
        fn new(start: u64) -> (Self, impl Fn() -> u64 + Send + Sync + 'static) {
            let cell = Arc::new(AtomicU64::new(start));
            let reader = cell.clone();
            (Self(cell), move || reader.load(Ordering::SeqCst))
        }

        fn set(&self, ms: u64) {
            self.0.store(ms, Ordering::SeqCst);
        }
    }

    const RULE: Rule = Rule::new(5, Duration::from_secs(10));
    /// A window boundary (a multiple of ten seconds), so tests can say "0.5 s into a window".
    const T0: u64 = 1_000_000_000_000;

    async fn hits(limiter: &RateLimiter, key: &str, n: usize) -> Vec<bool> {
        let mut out = Vec::new();
        for _ in 0..n {
            out.push(limiter.check(key, RULE).await.unwrap().allowed);
        }
        out
    }

    #[test]
    fn the_inequality_matches_the_documented_formula() {
        // Half way through a window, 4 in the previous one: 4 * 0.5 + 2 + 1 = 5 <= 5.
        assert!(allowed(5, 10, 5, 2, 4));
        assert!(!allowed(5, 10, 5, 3, 4));
        // Nothing before: a plain counter.
        assert!(allowed(5, 10, 0, 4, 0));
        assert!(!allowed(5, 10, 0, 5, 0));
        assert!(!allowed(0, 10, 0, 0, 0), "a zero limit refuses everything");
    }

    #[tokio::test]
    async fn the_limit_holds_and_remaining_counts_down() {
        let (time, clock) = Time::new(T0);
        let limiter = RateLimiter::memory_with_clock(clock);
        let mut remaining = Vec::new();
        for _ in 0..5 {
            let d = limiter.check("k", RULE).await.unwrap();
            assert!(d.allowed);
            assert_eq!(d.retry_after, Duration::ZERO);
            remaining.push(d.remaining);
        }
        assert_eq!(remaining, [4, 3, 2, 1, 0]);
        let refused = limiter.check("k", RULE).await.unwrap();
        assert!(!refused.allowed && refused.remaining == 0);
        assert!(refused.retry_after > Duration::ZERO);
        time.set(T0 + 9_999);
        assert!(!limiter.check("k", RULE).await.unwrap().allowed);
    }

    #[tokio::test]
    async fn keys_are_independent() {
        let (_time, clock) = Time::new(T0);
        let limiter = RateLimiter::memory_with_clock(clock);
        assert_eq!(
            hits(&limiter, "a", 6).await,
            [true, true, true, true, true, false]
        );
        assert_eq!(hits(&limiter, "b", 2).await, [true, true]);
    }

    #[tokio::test]
    async fn a_burst_at_the_end_of_a_window_cannot_be_repeated_at_the_start_of_the_next() {
        let (time, clock) = Time::new(T0 + 9_900);
        let limiter = RateLimiter::memory_with_clock(clock);
        assert_eq!(hits(&limiter, "k", 5).await, [true; 5]);
        // 0.2 s later a fixed-window limiter would hand out another five. This one still
        // remembers: 5 * (1 - 0.1) + 1 > 5.
        time.set(T0 + 10_100);
        assert!(!limiter.check("k", RULE).await.unwrap().allowed);
        // The earlier burst fades linearly: by 20% into the window one more fits...
        time.set(T0 + 12_100);
        assert!(limiter.check("k", RULE).await.unwrap().allowed);
        assert!(!limiter.check("k", RULE).await.unwrap().allowed);
        // ...and a long quiet spell forgets everything.
        time.set(T0 + 60_000);
        assert_eq!(hits(&limiter, "k", 5).await, [true; 5]);
    }

    #[tokio::test]
    async fn retry_after_is_honest() {
        let (time, clock) = Time::new(T0 + 3_000);
        let limiter = RateLimiter::memory_with_clock(clock);
        assert_eq!(hits(&limiter, "k", 5).await, [true; 5]);
        let refused = limiter.check("k", RULE).await.unwrap();
        assert!(!refused.allowed);
        let wait = u64::try_from(refused.retry_after.as_millis()).unwrap();
        // Waiting almost that long is still too early; waiting that long is enough.
        time.set(T0 + 3_000 + wait - 1);
        assert!(
            !limiter.check("k", RULE).await.unwrap().allowed,
            "{wait} ms was promised"
        );
        time.set(T0 + 3_000 + wait + 1);
        assert!(
            limiter.check("k", RULE).await.unwrap().allowed,
            "{wait} ms was promised"
        );
    }

    #[tokio::test]
    async fn retry_after_is_honest_when_the_previous_window_is_still_counting() {
        let (time, clock) = Time::new(T0 + 8_000);
        let limiter = RateLimiter::memory_with_clock(clock);
        assert_eq!(hits(&limiter, "k", 5).await, [true; 5]);
        time.set(T0 + 10_500);
        let refused = limiter.check("k", RULE).await.unwrap();
        assert!(!refused.allowed);
        let wait = u64::try_from(refused.retry_after.as_millis()).unwrap();
        assert!(
            wait > 1_000,
            "the burst fades over seconds, not milliseconds: {wait}"
        );
        time.set(T0 + 10_500 + wait - 5);
        assert!(!limiter.check("k", RULE).await.unwrap().allowed);
        time.set(T0 + 10_500 + wait + 5);
        assert!(limiter.check("k", RULE).await.unwrap().allowed);
    }

    #[tokio::test]
    async fn zero_and_one_limits_behave() {
        let (_time, clock) = Time::new(T0);
        let limiter = RateLimiter::memory_with_clock(clock);
        let none = limiter
            .check("z", Rule::new(0, Duration::from_secs(5)))
            .await
            .unwrap();
        assert!(!none.allowed && none.retry_after > Duration::ZERO);
        let one = Rule::new(1, Duration::from_secs(5));
        assert!(limiter.check("o", one).await.unwrap().allowed);
        assert!(!limiter.check("o", one).await.unwrap().allowed);
    }

    #[tokio::test]
    async fn concurrent_callers_never_overshoot() {
        let limiter = RateLimiter::memory();
        let rule = Rule::per_minute(7);
        let tasks: Vec<_> = (0..40)
            .map(|_| {
                let limiter = limiter.clone();
                tokio::spawn(async move { limiter.check("shared", rule).await.unwrap().allowed })
            })
            .collect();
        let mut granted = 0;
        for task in tasks {
            granted += usize::from(task.await.unwrap());
        }
        assert_eq!(granted, 7);
    }

    /// The same scenario against Redis: proves the Lua script and the Rust inequality agree.
    #[tokio::test]
    async fn redis_matches_memory() {
        let Ok(url) = std::env::var("TEST_REDIS_URL") else {
            eprintln!("skipped: TEST_REDIS_URL not set");
            return;
        };
        let client = redis::Client::open(url).unwrap();
        let conn = redis::aio::ConnectionManager::new(client).await.unwrap();
        // Unique keys and a time far from any other test's windows.
        let key = format!("test:{}", uuid_like());
        let start = T0 + 9_900;
        let (rtime, rclock) = Time::new(start);
        let (mtime, mclock) = Time::new(start);
        let redis = RateLimiter::redis_with_clock(conn, rclock);
        let memory = RateLimiter::memory_with_clock(mclock);
        let script = [
            (0, 7),
            (50, 2),
            (200, 2),
            (2_200, 3),
            (2_300, 2),
            (5_000, 1),
            (20_000, 7),
        ];
        let mut at = start;
        for (advance, count) in script {
            at += advance;
            rtime.set(at);
            mtime.set(at);
            for i in 0..count {
                let r = redis.check(&key, RULE).await.unwrap();
                let m = memory.check(&key, RULE).await.unwrap();
                assert_eq!(r, m, "at +{at} request {i}");
            }
        }
    }

    fn uuid_like() -> String {
        format!("{:x}{:x}", system_clock(), std::process::id())
    }
}
