use crate::counter::Counter;
use crate::limit::{Context, Limit, Namespace};
use crate::storage::atomic_expiring_value::AtomicExpiringValue;
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
        check: bool,
        update: bool,
        load_counters: bool,
    ) -> Result<Authorization, StorageErr> {
        let limits_by_namespace = self.simple_limits.read().unwrap();
        let mut first_limited = None;
        let mut counter_values_to_update: Vec<(&AtomicExpiringValue, Duration)> = Vec::new();
        let mut qualified_counter_values_to_update: Vec<(Arc<AtomicExpiringValue>, Duration)> =
            Vec::new();
        let now = SystemTime::now();

        let mut process_counter =
            |counter: &mut Counter, value: u64, delta: u64| -> () {
              
                let current_remaining = counter.max_value().checked_sub(value);
                let remaining = counter.max_value().checked_sub(value + delta);
                if load_counters {   
                    if update {
                        counter.set_remaining(remaining.unwrap_or_default());
                    } else {
                        counter.set_remaining(current_remaining.unwrap_or_default());
                    }
                }

                if first_limited.is_none() && remaining.is_none() {
                    first_limited = Some(Authorization::Limited(
                        counter.limit().name().map(|n| n.to_owned()),
                    ));
                }                
            };

        // Process simple counters
        for counter in counters.iter_mut().filter(|c| !c.is_qualified()) {
            let atomic_expiring_value: &AtomicExpiringValue =
                limits_by_namespace.get(counter.limit()).unwrap();
            
            process_counter( counter, atomic_expiring_value.value(), delta);

            if update {
                counter_values_to_update.push((atomic_expiring_value, counter.window()));
            }
        }

        // Process qualified counters
        for counter in counters.iter_mut().filter(|c| c.is_qualified()) {
            let value = match self.qualified_counters.get(counter) {
                None => self.qualified_counters.get_with_by_ref(counter, || {
                    Arc::new(AtomicExpiringValue::new(0, now + counter.window()))
                }),
                Some(counter) => counter,
            };

            process_counter(counter, value.value(), delta);

            if update {
                qualified_counter_values_to_update.push((value, counter.window()));
            }
        }

        if update {
            // Update counters
            counter_values_to_update.iter().for_each(|(v, ttl)| {
                v.update(delta, *ttl, now);
            });
            qualified_counter_values_to_update
                .iter()
                .for_each(|(v, ttl)| {
                    v.update(delta, *ttl, now);
            });
        }

        if check {
            if let Some(limited) = first_limited {
                return Ok(limited);
            }
        }
        Ok(Authorization::Ok)
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
}

impl InMemoryStorage {
    pub fn new(cache_size: u64) -> Self {
        Self {
            simple_limits: RwLock::new(BTreeMap::new()),
            qualified_counters: CacheBuilder::new(cache_size)
                .support_invalidation_closures()
                .build(),
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
                        self.qualified_counters.invalidate(&c);
                    }
                }
            }
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
}
