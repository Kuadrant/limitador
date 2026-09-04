use crate::counter::Counter;
use crate::reservation::{ReservationEntry, ReservationId};
use crate::storage::Authorization;
use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, SystemTime};

/// A local (single-process, in-memory only) reservation registry.
///
/// Used as a pragmatic starting point by backends that don't yet share reservations across
/// replicas/processes (`RocksDbStorage`, `CachedRedisStorage`) - mirroring the RFC's accepted
/// "local-memory-only" scope already granted to disk/distributed storage for counters
/// themselves. Not a substitute for a properly shared implementation where multiple
/// replicas need to see each other's outstanding reservations.
pub(crate) struct LocalReservationRegistry {
    entries: RwLock<HashMap<Counter, Vec<ReservationEntry>>>,
}

pub(crate) struct ReservationRequest<'a> {
    pub(crate) reservation_id: &'a ReservationId,
    pub(crate) amount: u64,
    pub(crate) ttl: Duration,
    pub(crate) load_counters: bool,
    pub(crate) now: SystemTime,
}

impl LocalReservationRegistry {
    pub(crate) fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
        }
    }

    fn outstanding(&self, counter: &Counter, now: SystemTime) -> u64 {
        self.entries
            .read()
            .unwrap()
            .get(counter)
            .map(|entries| {
                entries
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

        let mut registry = self.entries.write().unwrap();
        for (counter, expires_at) in counters.iter().zip(expires_at_by_counter) {
            let entries = registry.entry(counter.clone()).or_default();
            entries.retain(|e| e.is_live_at(now));
            entries.push(ReservationEntry::new(
                request.reservation_id.clone(),
                request.amount,
                expires_at,
            ));
        }

        Authorization::Ok
    }

    pub(crate) fn release(&self, counters: &[Counter], reservation_id: &ReservationId) -> bool {
        let mut released = false;
        let mut registry = self.entries.write().unwrap();
        for counter in counters {
            if let Some(entries) = registry.get_mut(counter) {
                let before = entries.len();
                entries.retain(|e| e.reservation_id() != reservation_id);
                released |= entries.len() != before;
                if entries.is_empty() {
                    registry.remove(counter);
                }
            }
        }
        released
    }
}
