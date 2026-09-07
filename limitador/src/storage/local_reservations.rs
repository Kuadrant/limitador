use crate::counter::Counter;
use crate::reservation::{ReservationEntry, ReservationId};
use crate::storage::Authorization;
use moka::sync::{Cache, CacheBuilder};
use moka::Expiry;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime};

/// Used by backends that don't have their own natural "expected number of distinct
/// counters" knob to size their local reservation registry against (currently only
/// `RocksDbStorage`; `InMemoryStorage` and `CachedRedisStorage` reuse their own counter
/// cache size instead).
#[cfg(feature = "disk_storage")]
pub(crate) const DEFAULT_LOCAL_RESERVATIONS_CACHE_SIZE: u64 = 10_000;

/// A counter's outstanding reservations, plus the remaining ttl of its current window as
/// of the last `reserve()` call that touched it - a reservation can never outlive its
/// counter's window (see `LocalReservationRegistry::reserve`'s `expires_at` clamp), so this
/// is also always a safe upper bound for how long to keep the whole entry cached.
struct CounterReservations {
    window_ttl: Duration,
    entries: Vec<ReservationEntry>,
}

type Entry = Arc<RwLock<CounterReservations>>;

/// A local (single-process, in-memory only) reservation registry.
///
/// Used as a pragmatic starting point by backends that don't yet share reservations across
/// replicas/processes (`RocksDbStorage`, `CachedRedisStorage`) - mirroring the RFC's accepted
/// "local-memory-only" scope already granted to disk/distributed storage for counters
/// themselves. Not a substitute for a properly shared implementation where multiple
/// replicas need to see each other's outstanding reservations.
///
/// Backed by a size-bounded Moka cache with a per-entry `Expiry` policy tied to each
/// counter's own window ttl, so a counter's reservations are actively reclaimed once its
/// window closes - even if that counter is never touched again - rather than accumulating
/// until the cache fills up.
pub(crate) struct LocalReservationRegistry {
    entries: Cache<Counter, Entry>,
    // Serializes the whole check-then-write admission decision in `reserve()` across every
    // counter it touches. Without this, two truly concurrent `reserve()` calls (real
    // OS-thread parallelism) can both read the same stale `outstanding` value, both decide
    // to admit, and both write - jointly exceeding the limit. Reservations are a lower-volume
    // path (only token/LLM-style rate limiting), so trading fine-grained per-counter
    // concurrency for one simple, obviously-correct critical section is the right tradeoff -
    // this is the in-process equivalent of the atomicity Redis gets for free from running the
    // whole check-and-write as a single Lua script. `release()` doesn't need this lock: it
    // only ever monotonically decreases outstanding, so racing it against a `reserve()`'s read
    // can only make that read more conservative, never allow over-admission.
    admission_lock: Mutex<()>,
}

pub(crate) struct ReservationRequest<'a> {
    pub(crate) reservation_id: &'a ReservationId,
    pub(crate) amount: u64,
    pub(crate) ttl: Duration,
    pub(crate) load_counters: bool,
    pub(crate) now: SystemTime,
}

struct ReservationExpiry;

impl Expiry<Counter, Entry> for ReservationExpiry {
    fn expire_after_create(
        &self,
        _key: &Counter,
        value: &Entry,
        _created_at: Instant,
    ) -> Option<Duration> {
        Some(value.read().unwrap().window_ttl)
    }

    // Mutating through the `RwLock` alone doesn't notify the cache of anything; `reserve`
    // re-inserts after updating `window_ttl` so this gets re-evaluated against the fresh
    // value, the same way the Redis backend re-issues `PEXPIRE` on every `reserve()` call.
    fn expire_after_update(
        &self,
        _key: &Counter,
        value: &Entry,
        _updated_at: Instant,
        _duration_until_expiry: Option<Duration>,
    ) -> Option<Duration> {
        Some(value.read().unwrap().window_ttl)
    }
}

impl LocalReservationRegistry {
    pub(crate) fn new(max_size: u64) -> Self {
        Self {
            entries: CacheBuilder::new(max_size)
                .expire_after(ReservationExpiry)
                .build(),
            admission_lock: Mutex::new(()),
        }
    }

    fn outstanding(&self, counter: &Counter, now: SystemTime) -> u64 {
        self.entries
            .get(counter)
            .map(|entry| {
                entry
                    .read()
                    .unwrap()
                    .entries
                    .iter()
                    .filter(|e| e.is_live_at(now))
                    .map(|e| e.amount())
                    .sum()
            })
            .unwrap_or_default()
    }

    /// `values_and_window_ttls[i]` must be the current value and remaining window ttl of
    /// `counters[i]`, already read (and, if absent, lazily established) by the caller.
    pub(crate) fn reserve(
        &self,
        counters: &mut [Counter],
        values_and_window_ttls: &[(u64, Duration)],
        request: ReservationRequest,
    ) -> Authorization {
        // Held for the whole check-then-write sequence below - see `admission_lock`'s docs.
        let _admission_guard = self.admission_lock.lock().unwrap();

        let now = request.now;
        let mut first_limited = None;
        let mut expires_at_by_counter = Vec::with_capacity(counters.len());

        for (counter, (value, window_ttl)) in counters.iter_mut().zip(values_and_window_ttls) {
            let outstanding = self.outstanding(counter, now);
            let expires_at = std::cmp::min(now + request.ttl, now + *window_ttl);
            expires_at_by_counter.push(expires_at);

            let total = value + outstanding + request.amount;
            if request.load_counters {
                let remaining = counter.max_value().checked_sub(total);
                counter.set_remaining(remaining.unwrap_or_default());
                counter.set_expires_in(*window_ttl);
            }
            if first_limited.is_none() && total > counter.max_value() {
                first_limited = Some(Authorization::Limited(
                    counter.limit().name().map(|n| n.to_owned()),
                ));
            }
        }

        if let Some(limited) = first_limited {
            return limited;
        }

        for ((counter, expires_at), (_, window_ttl)) in counters
            .iter()
            .zip(expires_at_by_counter)
            .zip(values_and_window_ttls)
        {
            let entry = self.entries.get_with(counter.clone(), || {
                Arc::new(RwLock::new(CounterReservations {
                    window_ttl: *window_ttl,
                    entries: Vec::new(),
                }))
            });
            {
                let mut guard = entry.write().unwrap();
                guard.window_ttl = *window_ttl;
                guard.entries.retain(|e| e.is_live_at(now));
                guard.entries.push(ReservationEntry::new(
                    request.reservation_id.clone(),
                    request.amount,
                    expires_at,
                ));
            }
            self.entries.insert(counter.clone(), entry);
        }

        Authorization::Ok
    }

    // Held for the whole read-then-write sequence below, same as `reserve()` - see
    // `admission_lock`'s docs. Without this, a concurrent `reserve()` could replace this
    // counter's cache entry with a fresh `Arc` (e.g. if the one we're about to read had
    // already expired/been evicted) between our `get` and our later `invalidate`; blindly
    // writing back our now-stale captured `Arc` would silently discard whatever
    // concurrently-admitted, still-live reservation that fresh `Arc` holds.
    pub(crate) fn release(&self, counters: &[Counter], reservation_id: &ReservationId) -> bool {
        let _admission_guard = self.admission_lock.lock().unwrap();

        let mut released = false;
        for counter in counters {
            if let Some(entry) = self.entries.get(counter) {
                let now_empty = {
                    let mut guard = entry.write().unwrap();
                    let before = guard.entries.len();
                    guard
                        .entries
                        .retain(|e| e.reservation_id() != reservation_id);
                    released |= guard.entries.len() != before;
                    guard.entries.is_empty()
                };
                if now_empty {
                    self.entries.invalidate(counter);
                }
                // Otherwise: nothing left to do. The mutation above already happened through
                // the same `Arc`/`RwLock` the cache still maps `counter` to (guaranteed by
                // holding `admission_lock` for the whole call), so it's already visible to
                // later reads - reinserting would only risk clobbering a fresher entry with
                // our own stale reference, and would incorrectly push the entry's
                // Moka-tracked expiry out from now, past the counter's actual window
                // boundary, since `release` never touches `window_ttl`.
            }
        }
        released
    }
}
