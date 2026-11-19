//! Counting views without a database write per view.
//!
//! A popular recording can be opened thousands of times a minute. Writing each one to Postgres
//! would make the hottest row in the system the one nobody needs to be exact about. Instead:
//!
//! * **Counting** is one cheap Redis operation. A view bumps a field of a pending hash (sharded
//!   16 ways, so one viral recording does not make one key the bottleneck) and adds the viewer to
//!   a HyperLogLog for unique viewers (a few kilobytes per recording-day, ~1% error).
//! * **Repeats do not count.** The same viewer opening the same recording again within
//!   [`REPEAT_WINDOW`] is one view: a refresh is not an audience.
//! * **Flushing** moves a whole shard out of Redis in one atomic step ([`ViewCounter::take`]): the
//!   pending hash is *renamed* to an in-flight batch with its own id. The caller writes that batch
//!   to Postgres in one transaction and then calls [`ViewCounter::done`]. A crash in between
//!   leaves the in-flight batch behind, where [`ViewCounter::stale_batches`] finds it for the next
//!   flush; the batch id is recorded in Postgres in the same transaction as the counts, so
//!   applying one twice is a no-op. Counts are never lost to a crash and never counted twice.
//!
//! Nothing here is money. If Redis is unreachable a view is dropped (with a warning) rather than
//! slowing or failing playback.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, NaiveDate, Utc};
use uuid::Uuid;

/// How many pending hashes views are spread over.
pub const SHARDS: usize = 16;
/// The same viewer opening the same recording again within this long is not a new view.
pub const REPEAT_WINDOW_SECS: u64 = 30 * 60;
const REPEAT_WINDOW: std::time::Duration = std::time::Duration::from_secs(REPEAT_WINDOW_SECS);
/// A unique-viewer sketch only matters until its day has been flushed; keep it a few days.
const SKETCH_TTL_SECS: u64 = 3 * 24 * 3600;
/// An in-flight batch nobody finished is garbage after this long (it was applied or abandoned).
const INFLIGHT_TTL_SECS: u64 = 7 * 24 * 3600;
/// A batch is "stale" (its flusher presumably died) once this old.
pub const STALE_AFTER_SECS: u64 = 120;

#[derive(Debug, thiserror::Error)]
#[error("view counter backend unavailable: {0}")]
pub struct ViewError(pub String);

/// The counts for one recording on one UTC day.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tally {
    pub recording: Uuid,
    pub day: NaiveDate,
    pub views: u64,
    /// Unique viewers that day: exact in memory, an estimate (HyperLogLog) in Redis.
    pub viewers: u64,
}

/// Counts moved out of the counter, to be written somewhere durable and then confirmed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Batch {
    pub id: Uuid,
    pub tallies: Vec<Tally>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recorded {
    /// A new view.
    Counted,
    /// The same viewer, recently: not counted again.
    Repeat,
}

pub fn shard_of(recording: Uuid) -> usize {
    (recording.as_u128() % SHARDS as u128) as usize
}

fn field(recording: Uuid, day: NaiveDate) -> String {
    format!("{recording}:{}", day.format("%Y%m%d"))
}

fn parse_field(raw: &str) -> Option<(Uuid, NaiveDate)> {
    let (recording, day) = raw.split_once(':')?;
    Some((
        Uuid::parse_str(recording).ok()?,
        NaiveDate::parse_from_str(day, "%Y%m%d").ok()?,
    ))
}

type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

fn system_clock() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn day_of(ms: u64) -> NaiveDate {
    DateTime::<Utc>::from_timestamp_millis(i64::try_from(ms).unwrap_or(i64::MAX))
        .unwrap_or_default()
        .date_naive()
}

#[derive(Clone)]
pub struct ViewCounter {
    backend: Arc<Backend>,
    clock: Clock,
}

enum Backend {
    Memory(Mutex<Memory>),
    /// Every call fails, as when Redis is unreachable.
    Failing,
    Redis(Box<RedisBackend>),
}

/// What a taken batch holds: when it was taken, and its (recording, day, views) rows.
type InflightBatch = (u64, Vec<(Uuid, NaiveDate, u64)>);

#[derive(Default)]
struct Memory {
    /// Viewer/recording pairs seen recently, with when the repeat window ends.
    seen: HashMap<(Uuid, String), u64>,
    sketches: HashMap<(Uuid, NaiveDate), HashSet<String>>,
    pending: Vec<HashMap<(Uuid, NaiveDate), u64>>,
    inflight: HashMap<Uuid, InflightBatch>,
}

struct RedisBackend {
    conn: redis::aio::ConnectionManager,
    seen: redis::Script,
}

/// KEYS: the repeat marker and the day's unique-viewer sketch (both tagged with the recording, so
/// they share a cluster slot). ARGV: viewer, repeat window, sketch lifetime. Returns 1 for a new
/// view. The sketch always hears about the viewer, repeat or not.
const SEEN_SCRIPT: &str = r#"
redis.call('PFADD', KEYS[2], ARGV[1])
redis.call('EXPIRE', KEYS[2], ARGV[3])
if redis.call('SET', KEYS[1], '1', 'NX', 'EX', ARGV[2]) then
  return 1
end
return 0
"#;

impl ViewCounter {
    /// Per-process counters: for tests and single-instance development.
    pub fn memory() -> Self {
        Self::memory_with_clock(system_clock)
    }

    /// As [`ViewCounter::memory`] with the clock (milliseconds since the Unix epoch) supplied by
    /// the caller, so tests can move time deliberately.
    pub fn memory_with_clock(clock: impl Fn() -> u64 + Send + Sync + 'static) -> Self {
        let memory = Memory {
            pending: (0..SHARDS).map(|_| HashMap::new()).collect(),
            ..Memory::default()
        };
        Self {
            backend: Arc::new(Backend::Memory(Mutex::new(memory))),
            clock: Arc::new(clock),
        }
    }

    /// A counter whose backend is permanently unreachable.
    pub fn failing() -> Self {
        Self {
            backend: Arc::new(Backend::Failing),
            clock: Arc::new(system_clock),
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
                seen: redis::Script::new(SEEN_SCRIPT),
            }))),
            clock: Arc::new(clock),
        }
    }

    /// Counts a view of `recording` by `viewer` (an opaque, stable id for who is watching).
    pub async fn record(&self, recording: Uuid, viewer: &str) -> Result<Recorded, ViewError> {
        let now = (self.clock)();
        let day = day_of(now);
        match self.backend.as_ref() {
            Backend::Memory(memory) => {
                let mut m = memory.lock().unwrap_or_else(|e| e.into_inner());
                if m.seen.len() > 100_000 {
                    m.seen.retain(|_, until| *until > now);
                }
                m.sketches
                    .entry((recording, day))
                    .or_default()
                    .insert(viewer.to_owned());
                let key = (recording, viewer.to_owned());
                if m.seen.get(&key).is_some_and(|until| *until > now) {
                    return Ok(Recorded::Repeat);
                }
                m.seen.insert(key, now + REPEAT_WINDOW.as_millis() as u64);
                *m.pending[shard_of(recording)]
                    .entry((recording, day))
                    .or_insert(0) += 1;
                Ok(Recorded::Counted)
            }
            Backend::Failing => Err(ViewError("backend unreachable".into())),
            Backend::Redis(redis) => {
                let mut conn = redis.conn.clone();
                let counted: u64 = redis
                    .seen
                    .key(format!("zeno:views:seen:{{{recording}}}:{viewer}"))
                    .key(format!(
                        "zeno:views:hll:{{{recording}}}:{}",
                        day.format("%Y%m%d")
                    ))
                    .arg(viewer)
                    .arg(REPEAT_WINDOW_SECS)
                    .arg(SKETCH_TTL_SECS)
                    .invoke_async(&mut conn)
                    .await
                    .map_err(|e| ViewError(e.to_string()))?;
                if counted == 0 {
                    return Ok(Recorded::Repeat);
                }
                let _: i64 = redis::cmd("HINCRBY")
                    .arg(format!("zeno:views:pending:{{{}}}", shard_of(recording)))
                    .arg(field(recording, day))
                    .arg(1)
                    .query_async(&mut conn)
                    .await
                    .map_err(|e| ViewError(e.to_string()))?;
                Ok(Recorded::Counted)
            }
        }
    }

    /// Atomically moves everything pending in `shard` into a new in-flight batch and returns it,
    /// or `None` if nothing was pending. The caller must write the batch durably and then call
    /// [`ViewCounter::done`].
    pub async fn take(&self, shard: usize) -> Result<Option<Batch>, ViewError> {
        let now = (self.clock)();
        match self.backend.as_ref() {
            Backend::Memory(memory) => {
                let mut m = memory.lock().unwrap_or_else(|e| e.into_inner());
                let pending = std::mem::take(&mut m.pending[shard]);
                if pending.is_empty() {
                    return Ok(None);
                }
                let id = Uuid::new_v4();
                let rows: Vec<(Uuid, NaiveDate, u64)> = pending
                    .into_iter()
                    .map(|((recording, day), views)| (recording, day, views))
                    .collect();
                let tallies = rows
                    .iter()
                    .map(|(recording, day, views)| Tally {
                        recording: *recording,
                        day: *day,
                        views: *views,
                        viewers: m
                            .sketches
                            .get(&(*recording, *day))
                            .map_or(0, |s| s.len() as u64),
                    })
                    .collect();
                m.inflight.insert(id, (now, rows));
                Ok(Some(Batch { id, tallies }))
            }
            Backend::Failing => Err(ViewError("backend unreachable".into())),
            Backend::Redis(redis) => {
                let mut conn = redis.conn.clone();
                let id = Uuid::new_v4();
                let pending = format!("zeno:views:pending:{{{shard}}}");
                let inflight = format!("zeno:views:inflight:{{{shard}}}:{id}");
                let renamed: Result<(), redis::RedisError> = redis::cmd("RENAME")
                    .arg(&pending)
                    .arg(&inflight)
                    .query_async(&mut conn)
                    .await;
                match renamed {
                    Ok(()) => {}
                    // Nothing was pending: the key does not exist.
                    Err(e) if e.to_string().contains("no such key") => return Ok(None),
                    Err(e) => return Err(ViewError(e.to_string())),
                }
                let counts: HashMap<String, u64> = redis::cmd("HGETALL")
                    .arg(&inflight)
                    .query_async(&mut conn)
                    .await
                    .map_err(|e| ViewError(e.to_string()))?;
                // Write the unique-viewer estimates into the batch itself, so a batch picked up
                // after a crash is complete without the sketches.
                let mut pipe = redis::pipe();
                let mut order = Vec::new();
                for key in counts.keys() {
                    if let Some((recording, day)) = parse_field(key) {
                        pipe.cmd("PFCOUNT").arg(format!(
                            "zeno:views:hll:{{{recording}}}:{}",
                            day.format("%Y%m%d")
                        ));
                        order.push(key.clone());
                    }
                }
                let estimates: Vec<u64> = if order.is_empty() {
                    Vec::new()
                } else {
                    pipe.query_async(&mut conn)
                        .await
                        .map_err(|e| ViewError(e.to_string()))?
                };
                let mut store = redis::pipe();
                store
                    .cmd("HSET")
                    .arg(&inflight)
                    .arg("t")
                    .arg(now)
                    .ignore()
                    .cmd("EXPIRE")
                    .arg(&inflight)
                    .arg(INFLIGHT_TTL_SECS)
                    .ignore();
                for (key, estimate) in order.iter().zip(&estimates) {
                    store
                        .cmd("HSET")
                        .arg(&inflight)
                        .arg(format!("{key}|u"))
                        .arg(*estimate)
                        .ignore();
                }
                let _: () = store
                    .query_async(&mut conn)
                    .await
                    .map_err(|e| ViewError(e.to_string()))?;
                Ok(Some(Batch {
                    id,
                    tallies: tallies_of(&counts, order.iter().zip(estimates).collect()),
                }))
            }
        }
    }

    /// Forgets a batch that has been written durably.
    pub async fn done(&self, batch: Uuid) -> Result<(), ViewError> {
        match self.backend.as_ref() {
            Backend::Memory(memory) => {
                let mut m = memory.lock().unwrap_or_else(|e| e.into_inner());
                m.inflight.remove(&batch);
                Ok(())
            }
            Backend::Failing => Err(ViewError("backend unreachable".into())),
            Backend::Redis(redis) => {
                let mut conn = redis.conn.clone();
                let key = redis_inflight_key(&mut conn, batch).await?;
                if let Some(key) = key {
                    let _: i64 = redis::cmd("DEL")
                        .arg(key)
                        .query_async(&mut conn)
                        .await
                        .map_err(|e| ViewError(e.to_string()))?;
                }
                Ok(())
            }
        }
    }

    /// In-flight batches whose flusher seems to have died: older than [`STALE_AFTER_SECS`] and
    /// still not confirmed. Applying one again is safe (the durable side records batch ids).
    pub async fn stale_batches(&self) -> Result<Vec<Batch>, ViewError> {
        let now = (self.clock)();
        let cutoff = now.saturating_sub(STALE_AFTER_SECS * 1000);
        match self.backend.as_ref() {
            Backend::Memory(memory) => {
                let m = memory.lock().unwrap_or_else(|e| e.into_inner());
                Ok(m.inflight
                    .iter()
                    .filter(|(_, (taken, _))| *taken <= cutoff)
                    .map(|(id, (_, rows))| Batch {
                        id: *id,
                        tallies: rows
                            .iter()
                            .map(|(recording, day, views)| Tally {
                                recording: *recording,
                                day: *day,
                                views: *views,
                                viewers: m
                                    .sketches
                                    .get(&(*recording, *day))
                                    .map_or(0, |s| s.len() as u64),
                            })
                            .collect(),
                    })
                    .collect())
            }
            Backend::Failing => Err(ViewError("backend unreachable".into())),
            Backend::Redis(redis) => {
                let mut conn = redis.conn.clone();
                let mut out = Vec::new();
                let mut cursor = 0u64;
                loop {
                    let (next, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                        .arg(cursor)
                        .arg("MATCH")
                        .arg("zeno:views:inflight:*")
                        .arg("COUNT")
                        .arg(200)
                        .query_async(&mut conn)
                        .await
                        .map_err(|e| ViewError(e.to_string()))?;
                    for key in keys {
                        let Some(id) = key.rsplit(':').next().and_then(|s| Uuid::parse_str(s).ok())
                        else {
                            continue;
                        };
                        let all: HashMap<String, u64> = redis::cmd("HGETALL")
                            .arg(&key)
                            .query_async(&mut conn)
                            .await
                            .map_err(|e| ViewError(e.to_string()))?;
                        let taken = all.get("t").copied().unwrap_or(0);
                        if taken > cutoff {
                            continue;
                        }
                        let estimates = all
                            .iter()
                            .filter_map(|(k, v)| k.strip_suffix("|u").map(|k| (k.to_owned(), *v)))
                            .collect::<Vec<_>>();
                        out.push(Batch {
                            id,
                            tallies: tallies_of(
                                &all,
                                estimates.iter().map(|(k, v)| (k, *v)).collect(),
                            ),
                        });
                    }
                    if next == 0 {
                        break;
                    }
                    cursor = next;
                }
                Ok(out)
            }
        }
    }
}

/// Builds tallies from an in-flight hash: the plain fields are view counts, `estimates` pairs a
/// field with its unique-viewer estimate.
fn tallies_of(counts: &HashMap<String, u64>, estimates: Vec<(&String, u64)>) -> Vec<Tally> {
    let estimates: HashMap<&String, u64> = estimates.into_iter().collect();
    let mut out: Vec<Tally> = counts
        .iter()
        .filter_map(|(key, views)| {
            let (recording, day) = parse_field(key)?;
            Some(Tally {
                recording,
                day,
                views: *views,
                viewers: estimates.get(key).copied().unwrap_or(0),
            })
        })
        .collect();
    out.sort_by_key(|t| (t.recording, t.day));
    out
}

/// The key of an in-flight batch, wherever its shard is.
async fn redis_inflight_key(
    conn: &mut redis::aio::ConnectionManager,
    batch: Uuid,
) -> Result<Option<String>, ViewError> {
    // The shard is part of the key, and `done` is called with only the batch id: look through
    // the shards (16 cheap EXISTS) rather than carrying the shard around.
    for shard in 0..SHARDS {
        let key = format!("zeno:views:inflight:{{{shard}}}:{batch}");
        let exists: bool = redis::cmd("EXISTS")
            .arg(&key)
            .query_async(conn)
            .await
            .map_err(|e| ViewError(e.to_string()))?;
        if exists {
            return Ok(Some(key));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    const T0: u64 = 1_800_000_000_000;

    fn clock() -> (Arc<AtomicU64>, impl Fn() -> u64 + Send + Sync + 'static) {
        let now = Arc::new(AtomicU64::new(T0));
        let handle = now.clone();
        (now, move || handle.load(Ordering::SeqCst))
    }

    fn rec(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    async fn drain(counter: &ViewCounter) -> Vec<Tally> {
        let mut all = Vec::new();
        for shard in 0..SHARDS {
            if let Some(batch) = counter.take(shard).await.unwrap() {
                counter.done(batch.id).await.unwrap();
                all.extend(batch.tallies);
            }
        }
        all.sort_by_key(|t| (t.recording, t.day));
        all
    }

    #[test]
    fn fields_round_trip_and_shards_cover_every_recording() {
        let day = NaiveDate::from_ymd_opt(2026, 11, 19).unwrap();
        let r = Uuid::new_v4();
        assert_eq!(parse_field(&field(r, day)), Some((r, day)));
        assert_eq!(parse_field("nonsense"), None);
        assert_eq!(parse_field(&format!("{r}:2026-11-19")), None);
        for n in 0..1000u128 {
            assert!(shard_of(rec(n)) < SHARDS);
        }
        let hit: HashSet<usize> = (0..64u128).map(|n| shard_of(rec(n))).collect();
        assert_eq!(hit.len(), SHARDS);
    }

    #[tokio::test]
    async fn views_are_counted_and_repeats_within_the_window_are_not() {
        let (now, clock) = clock();
        let counter = ViewCounter::memory_with_clock(clock);
        let r = rec(1);
        assert_eq!(counter.record(r, "ann").await.unwrap(), Recorded::Counted);
        assert_eq!(counter.record(r, "ann").await.unwrap(), Recorded::Repeat);
        assert_eq!(counter.record(r, "bob").await.unwrap(), Recorded::Counted);
        // Another recording is another view, by the same person.
        assert_eq!(
            counter.record(rec(2), "ann").await.unwrap(),
            Recorded::Counted
        );
        // After the window it is a new view.
        now.store(T0 + REPEAT_WINDOW_SECS * 1000 + 1, Ordering::SeqCst);
        assert_eq!(counter.record(r, "ann").await.unwrap(), Recorded::Counted);

        let tallies = drain(&counter).await;
        let of = |id| tallies.iter().find(|t| t.recording == id).unwrap();
        assert_eq!(
            (of(r).views, of(r).viewers),
            (3, 2),
            "three views by two people"
        );
        assert_eq!((of(rec(2)).views, of(rec(2)).viewers), (1, 1));
    }

    #[tokio::test]
    async fn a_day_boundary_starts_a_new_row_and_new_unique_viewers() {
        let (now, clock) = clock();
        let counter = ViewCounter::memory_with_clock(clock);
        counter.record(rec(1), "ann").await.unwrap();
        now.store(T0 + 24 * 3600 * 1000, Ordering::SeqCst);
        counter.record(rec(1), "ann").await.unwrap();
        let tallies = drain(&counter).await;
        assert_eq!(tallies.len(), 2);
        assert_ne!(tallies[0].day, tallies[1].day);
        assert!(tallies.iter().all(|t| t.views == 1 && t.viewers == 1));
    }

    #[tokio::test]
    async fn taking_moves_everything_out_once_and_nothing_is_lost_or_doubled() {
        let counter = ViewCounter::memory();
        for n in 0..50 {
            counter.record(rec(7), &format!("v{n}")).await.unwrap();
        }
        let shard = shard_of(rec(7));
        let batch = counter.take(shard).await.unwrap().unwrap();
        assert_eq!(batch.tallies.len(), 1);
        assert_eq!(batch.tallies[0].views, 50);
        // Nothing is pending any more; views arriving now go into the next batch.
        assert!(counter.take(shard).await.unwrap().is_none());
        counter.record(rec(7), "late").await.unwrap();
        let next = counter.take(shard).await.unwrap().unwrap();
        assert_eq!(next.tallies[0].views, 1);
        assert_ne!(next.id, batch.id);
    }

    #[tokio::test]
    async fn an_unconfirmed_batch_is_found_again_once_stale_and_not_before() {
        let (now, clock) = clock();
        let counter = ViewCounter::memory_with_clock(clock);
        counter.record(rec(3), "ann").await.unwrap();
        let batch = counter.take(shard_of(rec(3))).await.unwrap().unwrap();
        assert!(
            counter.stale_batches().await.unwrap().is_empty(),
            "its flusher may still be working"
        );
        now.store(T0 + STALE_AFTER_SECS * 1000 + 1, Ordering::SeqCst);
        let stale = counter.stale_batches().await.unwrap();
        assert_eq!(stale, vec![batch.clone()]);
        counter.done(batch.id).await.unwrap();
        assert!(counter.stale_batches().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_failing_backend_reports_an_error_rather_than_a_count() {
        let counter = ViewCounter::failing();
        assert!(counter.record(rec(1), "ann").await.is_err());
        assert!(counter.take(0).await.is_err());
        assert!(counter.done(Uuid::new_v4()).await.is_err());
        assert!(counter.stale_batches().await.is_err());
    }

    #[tokio::test]
    async fn concurrent_viewers_are_all_counted_exactly_once() {
        let counter = ViewCounter::memory();
        let tasks: Vec<_> = (0..500)
            .map(|n| {
                let counter = counter.clone();
                tokio::spawn(async move {
                    counter
                        .record(rec(9), &format!("v{}", n % 100))
                        .await
                        .unwrap()
                })
            })
            .collect();
        let mut counted = 0;
        for task in tasks {
            counted += usize::from(task.await.unwrap() == Recorded::Counted);
        }
        assert_eq!(
            counted, 100,
            "100 distinct viewers, however many times each pressed play"
        );
        let tallies = drain(&counter).await;
        assert_eq!((tallies[0].views, tallies[0].viewers), (100, 100));
    }

    /// The same scenario against Redis: the script, the sharded hashes, the rename and the
    /// sketch agree with the in-memory model.
    #[tokio::test]
    async fn redis_matches_memory() {
        let Ok(url) = std::env::var("TEST_REDIS_URL") else {
            eprintln!("skipped: TEST_REDIS_URL not set");
            return;
        };
        let client = redis::Client::open(url).unwrap();
        let conn = redis::aio::ConnectionManager::new(client).await.unwrap();
        // Recordings and a day nobody else uses.
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let day_ms = T0 + 37 * 24 * 3600 * 1000;
        let (rnow, rclock) = clock();
        let (mnow, mclock) = clock();
        rnow.store(day_ms, Ordering::SeqCst);
        mnow.store(day_ms, Ordering::SeqCst);
        let redis = ViewCounter::redis_with_clock(conn.clone(), rclock);
        let memory = ViewCounter::memory_with_clock(mclock);
        for counter in [&redis, &memory] {
            for (who, which) in [
                ("ann", a),
                ("ann", a),
                ("bob", a),
                ("ann", b),
                ("cy", a),
                ("cy", a),
            ] {
                counter.record(which, who).await.unwrap();
            }
        }
        let want = |counter: &ViewCounter| {
            let counter = counter.clone();
            async move {
                let mut mine = Vec::new();
                for shard in [shard_of(a), shard_of(b)] {
                    if let Some(batch) = counter.take(shard).await.unwrap() {
                        mine.extend(
                            batch
                                .tallies
                                .into_iter()
                                .filter(|t| t.recording == a || t.recording == b)
                                .map(|t| (t, batch.id)),
                        );
                    }
                }
                mine
            }
        };
        let got_redis = want(&redis).await;
        let got_memory = want(&memory).await;
        let strip = |v: &Vec<(Tally, Uuid)>| {
            let mut t: Vec<Tally> = v.iter().map(|(t, _)| t.clone()).collect();
            t.sort_by_key(|t| (t.recording, t.day));
            t
        };
        assert_eq!(strip(&got_redis), strip(&got_memory));
        let mine = strip(&got_redis);
        assert_eq!(mine.iter().find(|t| t.recording == a).unwrap().views, 3);
        assert_eq!(mine.iter().find(|t| t.recording == a).unwrap().viewers, 3);

        // Unconfirmed, the batch survives for a later flush; confirmed, it is gone.
        rnow.store(day_ms + STALE_AFTER_SECS * 1000 + 1, Ordering::SeqCst);
        let stale = redis.stale_batches().await.unwrap();
        let ids: HashSet<Uuid> = got_redis.iter().map(|(_, id)| *id).collect();
        for id in &ids {
            let found = stale
                .iter()
                .find(|b| b.id == *id)
                .expect("an unconfirmed batch is found");
            assert!(found
                .tallies
                .iter()
                .any(|t| t.recording == a || t.recording == b));
        }
        for id in &ids {
            redis.done(*id).await.unwrap();
        }
        let after = redis.stale_batches().await.unwrap();
        assert!(after.iter().all(|b| !ids.contains(&b.id)));
    }
}
