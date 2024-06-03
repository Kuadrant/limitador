use crate::counter::Counter;
use crate::limit::{Limit, Namespace};
use crate::storage::atomic_expiring_value::AtomicExpiringValue;
use crate::storage::{Authorization, CounterStorage, StorageErr};
use moka::sync::Cache;
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Deref;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};

pub struct InMemoryStorage {
    simple_limits: RwLock<BTreeMap<Limit, AtomicExpiringValue>>,
    qualified_counters: Cache<Counter, Arc<AtomicExpiringValue>>,
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
    fn check_and_update(
        &self,
        counters: &mut Vec<Counter>,
        delta: u64,
        load_counters: bool,
    ) -> Result<Authorization, StorageErr> {
        let limits_by_namespace = self.simple_limits.read().unwrap();
        let mut first_limited = None;
        let mut counter_values_to_update: Vec<(&AtomicExpiringValue, Duration)> = Vec::new();
        let mut qualified_counter_values_to_updated: Vec<(Arc<AtomicExpiringValue>, Duration)> =
            Vec::new();
        let now = SystemTime::now();

        let mut process_counter =
            |counter: &mut Counter, value: u64, delta: u64| -> Option<Authorization> {
                if load_counters {
                    let remaining = counter.max_value().checked_sub(value + delta);
                    counter.set_remaining(remaining.unwrap_or_default());
                    if first_limited.is_none() && remaining.is_none() {
                        first_limited = Some(Authorization::Limited(
                            counter.limit().name().map(|n| n.to_owned()),
                        ));
                    }
                }
                if !Self::counter_is_within_limits(counter, Some(&value), delta) {
                    return Some(Authorization::Limited(
                        counter.limit().name().map(|n| n.to_owned()),
                    ));
                }
                None
            };

        // Process simple counters
        for counter in counters.iter_mut().filter(|c| !c.is_qualified()) {
            let atomic_expiring_value: &AtomicExpiringValue =
                limits_by_namespace.get(counter.limit()).unwrap();

            if let Some(limited) = process_counter(counter, atomic_expiring_value.value(), delta) {
                if !load_counters {
                    return Ok(limited);
                }
            }
            counter_values_to_update.push((atomic_expiring_value, counter.window()));
        }

        // Process qualified counters
        for counter in counters.iter_mut().filter(|c| c.is_qualified()) {
            let value = match self.qualified_counters.get(counter) {
                None => self.qualified_counters.get_with_by_ref(counter, || {
                    Arc::new(AtomicExpiringValue::new(0, now + counter.window()))
                }),
                Some(counter) => counter,
            };

            if let Some(limited) = process_counter(counter, value.value(), delta) {
                if !load_counters {
                    return Ok(limited);
                }
            }

            qualified_counter_values_to_updated.push((value, counter.window()));
        }

        if let Some(limited) = first_limited {
            return Ok(limited);
        }

        // Update counters
        counter_values_to_update.iter().for_each(|(v, ttl)| {
            v.update(delta, *ttl, now);
        });
        qualified_counter_values_to_updated
            .iter()
            .for_each(|(v, ttl)| {
                v.update(delta, *ttl, now);
            });

        Ok(Authorization::Ok)
    }

    #[tracing::instrument(skip_all)]
    fn get_counters(&self, limits: &HashSet<Limit>) -> Result<HashSet<Counter>, StorageErr> {
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
    fn delete_counters(&self, limits: HashSet<Limit>) -> Result<(), StorageErr> {
        for limit in limits {
            self.delete_counters_of_limit(&limit);
        }
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    fn clear(&self) -> Result<(), StorageErr> {
        self.simple_limits.write().unwrap().clear();
        Ok(())
    }
}

impl InMemoryStorage {
    pub fn new(cache_size: u64) -> Self {
        Self {
            simple_limits: RwLock::new(BTreeMap::new()),
            qualified_counters: Cache::new(cache_size),
        }
    }

    fn counters_in_namespace(
        &self,
        namespace: &Namespace,
    ) -> HashMap<Counter, AtomicExpiringValue> {
        let mut res: HashMap<Counter, AtomicExpiringValue> = HashMap::new();

        for (limit, counter) in self.simple_limits.read().unwrap().iter() {
            if limit.namespace() == namespace {
                res.insert(
                    Counter::new(limit.clone(), HashMap::default()),
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
        self.simple_limits.write().unwrap().remove(limit);
    }

    fn counter_is_within_limits(counter: &Counter, current_val: Option<&u64>, delta: u64) -> bool {
        match current_val {
            Some(current_val) => current_val + delta <= counter.max_value(),
            None => counter.max_value() >= delta,
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
        let limit_1 = Limit::new(namespace, 1, 1, vec!["req.method == 'GET'"], vec!["app_id"]);
        let limit_2 = Limit::new(
            namespace,
            1,
            10,
            vec!["req.method == 'GET'"],
            vec!["app_id"],
        );
        let counter_1 = Counter::new(limit_1, HashMap::default());
        let counter_2 = Counter::new(limit_2, HashMap::default());
        storage.update_counter(&counter_1, 1).unwrap();
        storage.update_counter(&counter_2, 1).unwrap();

        assert_eq!(
            storage.counters_in_namespace(counter_1.namespace()).len(),
            2
        );
    }
}
