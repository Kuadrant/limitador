use crate::counter::Counter;
use crate::limit::{Context, Limit, Namespace};
use crate::reservation::ReservationId;
use crate::storage::atomic_expiring_value::AtomicExpiringValue;
use crate::storage::local_reservations::{LocalReservationRegistry, ReservationRequest};
use crate::storage::{Authorization, CounterStorage, StorageErr};
use moka::sync::{Cache, CacheBuilder};
use moka::PredicateError;
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Deref;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};

pub struct InMemoryStorage {
    simple_limits: RwLock<BTreeMap<Limit, AtomicExpiringValue>>,
    qualified_counters: Cache<Counter, Arc<AtomicExpiringValue>>,
    reservations: LocalReservationRegistry,
}

impl CounterStorage for InMemoryStorage {
    #[tracing::instrument(skip_all)]
    fn is_within_limits(&self, counter: &Counter, delta: u64) -> Result<bool, StorageErr> {
        let value = if counter.is_qualified() {
            self.qualified_counters
                .get(counter)
                .map(|c| c.value())
                .unwrap_or_default()
        } else {
            let limits_by_namespace = self.simple_limits.read().unwrap();
            limits_by_namespace
                .get(counter.limit())
                .map(|c| c.value())
                .unwrap_or_default()
        };

        Ok(counter.max_value() >= value + delta)
    }

    #[tracing::instrument(skip_all)]
    fn add_counter(&self, limit: &Limit) -> Result<(), StorageErr> {
        if limit.variables().is_empty() {
            let mut limits_by_namespace = self.simple_limits.write().unwrap();
            limits_by_namespace.entry(limit.clone()).or_default();
        }
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    fn update_counter(&self, counter: &Counter, delta: u64) -> Result<(), StorageErr> {
        let mut counters = self.simple_limits.write().unwrap();
        let now = SystemTime::now();
        if counter.is_qualified() {
            let value = match self.qualified_counters.get(counter) {
                None => self.qualified_counters.get_with(counter.clone(), || {
                    Arc::new(AtomicExpiringValue::new(0, now + counter.window()))
                }),
                Some(counter) => counter,
            };
            value.update(delta, counter.window(), now);
        } else {
            match counters.entry(counter.limit().clone()) {
                Entry::Vacant(v) => {
                    v.insert(AtomicExpiringValue::new(delta, now + counter.window()));
                }
                Entry::Occupied(o) => {
                    o.get().update(delta, counter.window(), now);
                }
            }
        }
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    fn check(&self, counters: &mut Vec<Counter>, delta: u64) -> Result<Authorization, StorageErr> {
        Ok(self.evaluate(counters, delta))
    }

    #[tracing::instrument(skip_all)]
    fn check_and_update(
        &self,
        counters: &mut Vec<Counter>,
        delta: u64,
    ) -> Result<Authorization, StorageErr> {
        let auth = self.evaluate(counters, delta);
        if matches!(auth, Authorization::Ok) {
            for counter in counters.iter() {
                self.update_counter(counter, delta)?;
            }
        }
        Ok(auth)
    }

    #[tracing::instrument(skip_all)]
    fn get_counters(&self, limits: &HashSet<Arc<Limit>>) -> Result<HashSet<Counter>, StorageErr> {
        let mut res = HashSet::new();

        for limit in limits {
            for (counter, expiring_value) in self.counters_in_namespace(limit.namespace()) {
                let mut counter_with_val = counter.clone();
                counter_with_val
                    .set_remaining(counter_with_val.max_value() - expiring_value.value());
                counter_with_val.set_expires_in(expiring_value.ttl());
                if counter_with_val.expires_in().unwrap() > Duration::ZERO {
                    res.insert(counter_with_val);
                }
            }
        }

        for (counter, expiring_value) in self.qualified_counters.iter() {
            if limits.contains(counter.limit()) {
                let mut counter_with_val = counter.deref().clone();
                counter_with_val
                    .set_remaining(counter_with_val.max_value() - expiring_value.value());
                counter_with_val.set_expires_in(expiring_value.ttl());
                if counter_with_val.expires_in().unwrap() > Duration::ZERO {
                    res.insert(counter_with_val);
                }
            }
        }

        Ok(res)
    }

    #[tracing::instrument(skip_all)]
    fn delete_counters(&self, limits: &HashSet<Arc<Limit>>) -> Result<(), StorageErr> {
        for limit in limits {
            self.delete_counters_of_limit(limit);
        }
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    fn clear(&self) -> Result<(), StorageErr> {
        self.simple_limits.write().unwrap().clear();
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    fn reserve(
        &self,
        counters: &mut Vec<Counter>,
        reservation_id: &ReservationId,
        check_amount: u64,
        hold_amount: u64,
        ttl: Duration,
    ) -> Result<Authorization, StorageErr> {
        let now = SystemTime::now();
        let values_and_window_ttls: Vec<(u64, Duration)> = counters
            .iter()
            .map(|counter| self.touch(counter, now))
            .collect();

        Ok(self.reservations.reserve(
            counters,
            &values_and_window_ttls,
            ReservationRequest {
                reservation_id,
                check_amount,
                hold_amount,
                ttl,
                now,
            },
        ))
    }

    #[tracing::instrument(skip_all)]
    fn release_reservation(
        &self,
        counters: &[Counter],
        reservation_id: &ReservationId,
    ) -> Result<bool, StorageErr> {
        Ok(self.reservations.release(counters, reservation_id))
    }
}

impl InMemoryStorage {
    pub fn new(cache_size: u64) -> Self {
        Self {
            simple_limits: RwLock::new(BTreeMap::new()),
            qualified_counters: CacheBuilder::new(cache_size)
                .support_invalidation_closures()
                .build(),
            reservations: LocalReservationRegistry::new(cache_size),
        }
    }

    /// Shared by [`CounterStorage::check`] and [`CounterStorage::check_and_update`]: reads
    /// every counter's current value, annotating each with its `remaining`/`expires_in`, and
    /// determines whether `delta` more would keep all of them within their limit. Never
    /// mutates any stored value.
    fn evaluate(&self, counters: &mut [Counter], delta: u64) -> Authorization {
        let limits_by_namespace = self.simple_limits.read().unwrap();
        let mut first_limited = None;

        let mut record = |counter: &mut Counter, value: u64, ttl: Duration| {
            // A zero ttl means the previous window has already elapsed (whether or not a
            // stale entry is still physically present) - report a fresh full window, the same
            // as a counter that never existed at all, rather than a misleading "already reset".
            let ttl = if ttl.is_zero() { counter.window() } else { ttl };
            let remaining = counter.max_value().checked_sub(value + delta);
            counter.set_remaining(remaining.unwrap_or_default());
            counter.set_expires_in(ttl);
            if first_limited.is_none() && remaining.is_none() {
                first_limited = Some(Authorization::Limited(
                    counter.limit().name().map(|n| n.to_owned()),
                ));
            }
        };

        for counter in counters.iter_mut().filter(|c| !c.is_qualified()) {
            let atomic_expiring_value: &AtomicExpiringValue =
                limits_by_namespace.get(counter.limit()).unwrap();
            record(
                counter,
                atomic_expiring_value.value(),
                atomic_expiring_value.ttl(),
            );
        }

        // Never insert into `qualified_counters` here on a miss - `check()` must stay
        // read-only. Persisting (and thus actually starting a counter's window) only happens
        // in `check_and_update`'s subsequent `update_counter` call, once admission is decided.
        for counter in counters.iter_mut().filter(|c| c.is_qualified()) {
            match self.qualified_counters.get(counter) {
                Some(value) => record(counter, value.value(), value.ttl()),
                None => record(counter, 0, counter.window()),
            }
        }

        first_limited.unwrap_or(Authorization::Ok)
    }

    fn counters_in_namespace(
        &self,
        namespace: &Namespace,
    ) -> HashMap<Counter, AtomicExpiringValue> {
        let mut res: HashMap<Counter, AtomicExpiringValue> = HashMap::new();

        for (limit, counter) in self.simple_limits.read().unwrap().iter() {
            if limit.namespace() == namespace {
                res.insert(
                    // todo fixme
                    Counter::new(limit.clone(), &Context::default())
                        .unwrap()
                        .unwrap(),
                    counter.clone(),
                );
            }
        }

        for (counter, value) in self.qualified_counters.iter() {
            if counter.namespace() == namespace {
                res.insert(counter.deref().clone(), value.deref().clone());
            }
        }

        res
    }

    fn delete_counters_of_limit(&self, limit: &Limit) {
        if limit.variables().is_empty() {
            self.simple_limits.write().unwrap().remove(limit);
        } else {
            let l = limit.clone();
            if let Err(PredicateError::InvalidationClosuresDisabled) = self
                .qualified_counters
                .invalidate_entries_if(move |c, _| c.limit() == &l)
            {
                for (c, _) in self.qualified_counters.iter() {
                    if c.limit() == limit {
                        self.qualified_counters.invalidate(c.as_ref());
                    }
                }
            }
        }
    }

    // Reads the counter's current value, lazily starting (or restarting, if expired) its
    // window with a zero-delta update. Returns the (unaffected) value and the ttl of the
    // window it now belongs to, so callers can compute a per-counter reservation expiry
    // without ever touching the counter's real value.
    fn touch(&self, counter: &Counter, now: SystemTime) -> (u64, Duration) {
        if counter.is_qualified() {
            let value = match self.qualified_counters.get(counter) {
                None => self.qualified_counters.get_with(counter.clone(), || {
                    Arc::new(AtomicExpiringValue::new(0, now + counter.window()))
                }),
                Some(value) => value,
            };
            let current = value.update(0, counter.window(), now);
            (current, value.ttl())
        } else {
            let mut counters = self.simple_limits.write().unwrap();
            let value = counters
                .entry(counter.limit().clone())
                .or_insert_with(|| AtomicExpiringValue::new(0, now + counter.window()));
            let current = value.update(0, counter.window(), now);
            (current, value.ttl())
        }
    }
}

impl Default for InMemoryStorage {
    fn default() -> Self {
        Self::new(10_000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_for_multiple_limit_per_ns() {
        let storage = InMemoryStorage::default();
        let namespace = "test_namespace";
        let limit_1 = Limit::new(
            namespace,
            1,
            1,
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            vec!["app_id".try_into().expect("failed parsing!")],
        );
        let limit_2 = Limit::new(
            namespace,
            1,
            10,
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            vec!["app_id".try_into().expect("failed parsing!")],
        );
        let map = HashMap::from([("app_id".to_string(), "foo".to_string())]);
        let ctx = map.into();
        let counter_1 = Counter::new(limit_1, &ctx)
            .expect("counter creation failed!")
            .expect("Should have a counter");
        let counter_2 = Counter::new(limit_2, &ctx)
            .expect("counter creation failed!")
            .expect("Should have a counter");
        storage.update_counter(&counter_1, 1).unwrap();
        storage.update_counter(&counter_2, 1).unwrap();

        assert_eq!(
            storage.counters_in_namespace(counter_1.namespace()).len(),
            2
        );
    }

    fn counter(max_value: u64) -> Counter {
        let limit = Limit::new(
            "reserve_test",
            max_value,
            60,
            vec![],
            Vec::<crate::limit::Expression>::default(),
        );
        Counter::new(limit, &Context::default())
            .unwrap()
            .expect("must have a counter")
    }

    #[test]
    fn reserve_with_hold_amount_zero_creates_no_entry() {
        let storage = InMemoryStorage::default();
        let mut counters = vec![counter(10)];
        let reservation_id = ReservationId::new();

        let auth = storage
            .reserve(
                &mut counters,
                &reservation_id,
                10,
                0,
                Duration::from_secs(60),
            )
            .unwrap();
        assert!(matches!(auth, Authorization::Ok));

        let released = storage
            .release_reservation(&counters, &reservation_id)
            .unwrap();
        assert!(!released, "nothing should have been held to release");
    }

    #[test]
    fn check_does_not_persist_qualified_counters() {
        let storage = InMemoryStorage::default();
        let limit = Limit::new(
            "check_no_persist_test",
            10,
            60,
            vec![],
            vec!["app_id".try_into().expect("failed parsing!")],
        );
        let map = HashMap::from([("app_id".to_string(), "1".to_string())]);
        let counter = Counter::new(limit, &map.into())
            .unwrap()
            .expect("must have a counter");

        let auth = storage.check(&mut vec![counter], 1).unwrap();
        assert!(matches!(auth, Authorization::Ok));

        assert_eq!(
            storage.qualified_counters.iter().count(),
            0,
            "a read-only check must not insert anything into the qualified counters cache - \
             doing so would prematurely start the counter's window and let read-only traffic \
             grow the cache with zero-usage entries"
        );
    }

    #[test]
    fn check_reports_fresh_window_for_a_never_used_counter() {
        let storage = InMemoryStorage::default();
        let limit = Limit::new(
            "check_fresh_window_test",
            10,
            60,
            vec![],
            Vec::<crate::limit::Expression>::default(),
        );
        storage.add_counter(&limit).unwrap();
        let counter = Counter::new(limit, &Context::default())
            .unwrap()
            .expect("must have a counter");

        let mut counters = vec![counter.clone()];
        storage.check(&mut counters, 1).unwrap();

        assert_eq!(
            counters[0].expires_in().unwrap(),
            counter.window(),
            "a never-used counter should report a fresh full window, not a stale/zero ttl"
        );
    }
}
