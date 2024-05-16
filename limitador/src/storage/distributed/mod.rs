mod cr_counter_value;

use crate::counter::Counter;
use crate::limit::{Limit, Namespace};
use crate::storage::distributed::cr_counter_value::CrCounterValue;
use crate::storage::{Authorization, CounterStorage, StorageErr};
use moka::sync::Cache;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::ops::Deref;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};

type NamespacedLimitCounters<T> = HashMap<Namespace, HashMap<Limit, T>>;

pub struct CrInMemoryStorage {
    identifier: String,
    limits_for_namespace: RwLock<NamespacedLimitCounters<CrCounterValue<String>>>,
    qualified_counters: Cache<Counter, Arc<CrCounterValue<String>>>,
}

impl CounterStorage for CrInMemoryStorage {
    #[tracing::instrument(skip_all)]
    fn is_within_limits(&self, counter: &Counter, delta: i64) -> Result<bool, StorageErr> {
        let limits_by_namespace = self.limits_for_namespace.read().unwrap();

        let mut value = 0;

        if counter.is_qualified() {
            if let Some(counter) = self.qualified_counters.get(counter) {
                value = counter.read();
            }
        } else if let Some(limits) = limits_by_namespace.get(counter.limit().namespace()) {
            if let Some(counter) = limits.get(counter.limit()) {
                value = counter.read();
            }
        }

        Ok(counter.max_value() as u64 >= value + (delta as u64))
    }

    #[tracing::instrument(skip_all)]
    fn add_counter(&self, limit: &Limit) -> Result<(), StorageErr> {
        if limit.variables().is_empty() {
            let mut limits_by_namespace = self.limits_for_namespace.write().unwrap();
            limits_by_namespace
                .entry(limit.namespace().clone())
                .or_default()
                .entry(limit.clone())
                .or_insert(CrCounterValue::new(
                    self.identifier.clone(),
                    Duration::from_secs(limit.seconds()),
                ));
        }
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    fn update_counter(&self, counter: &Counter, delta: i64) -> Result<(), StorageErr> {
        let mut limits_by_namespace = self.limits_for_namespace.write().unwrap();
        let now = SystemTime::now();
        if counter.is_qualified() {
            let value = match self.qualified_counters.get(counter) {
                None => self.qualified_counters.get_with(counter.clone(), || {
                    Arc::new(CrCounterValue::new(
                        self.identifier.clone(),
                        Duration::from_secs(counter.seconds()),
                    ))
                }),
                Some(counter) => counter,
            };
            value.inc_at(delta as u64, Duration::from_secs(counter.seconds()), now);
        } else {
            match limits_by_namespace.entry(counter.limit().namespace().clone()) {
                Entry::Vacant(v) => {
                    let mut limits = HashMap::new();
                    let duration = Duration::from_secs(counter.seconds());
                    let counter_val = CrCounterValue::new(self.identifier.clone(), duration);
                    counter_val.inc_at(delta as u64, duration, now);
                    limits.insert(counter.limit().clone(), counter_val);
                    v.insert(limits);
                }
                Entry::Occupied(mut o) => match o.get_mut().entry(counter.limit().clone()) {
                    Entry::Vacant(v) => {
                        let duration = Duration::from_secs(counter.seconds());
                        let counter_value = CrCounterValue::new(self.identifier.clone(), duration);
                        counter_value.inc_at(delta as u64, duration, now);
                        v.insert(counter_value);
                    }
                    Entry::Occupied(o) => {
                        o.get()
                            .inc_at(delta as u64, Duration::from_secs(counter.seconds()), now);
                    }
                },
            }
        }
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    fn check_and_update(
        &self,
        counters: &mut Vec<Counter>,
        delta: i64,
        load_counters: bool,
    ) -> Result<Authorization, StorageErr> {
        let limits_by_namespace = self.limits_for_namespace.write().unwrap();
        let mut first_limited = None;
        let mut counter_values_to_update: Vec<(&CrCounterValue<String>, u64)> = Vec::new();
        let mut qualified_counter_values_to_updated: Vec<(Arc<CrCounterValue<String>>, u64)> =
            Vec::new();
        let now = SystemTime::now();

        let mut process_counter =
            |counter: &mut Counter, value: i64, delta: i64| -> Option<Authorization> {
                if load_counters {
                    let remaining = counter.max_value() - (value + delta);
                    counter.set_remaining(remaining);
                    if first_limited.is_none() && remaining < 0 {
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
            let atomic_expiring_value: &CrCounterValue<String> = limits_by_namespace
                .get(counter.limit().namespace())
                .and_then(|limits| limits.get(counter.limit()))
                .unwrap();

            if let Some(limited) =
                process_counter(counter, atomic_expiring_value.read() as i64, delta)
            {
                if !load_counters {
                    return Ok(limited);
                }
            }
            counter_values_to_update.push((atomic_expiring_value, counter.seconds()));
        }

        // Process qualified counters
        for counter in counters.iter_mut().filter(|c| c.is_qualified()) {
            let value = match self.qualified_counters.get(counter) {
                None => self.qualified_counters.get_with(counter.clone(), || {
                    Arc::new(CrCounterValue::new(
                        self.identifier.clone(),
                        Duration::from_secs(counter.seconds()),
                    ))
                }),
                Some(counter) => counter,
            };

            if let Some(limited) = process_counter(counter, value.read() as i64, delta) {
                if !load_counters {
                    return Ok(limited);
                }
            }

            qualified_counter_values_to_updated.push((value, counter.seconds()));
        }

        if let Some(limited) = first_limited {
            return Ok(limited);
        }

        // Update counters
        counter_values_to_update.iter().for_each(|(v, ttl)| {
            v.inc_at(delta as u64, Duration::from_secs(*ttl), now);
        });
        qualified_counter_values_to_updated
            .iter()
            .for_each(|(v, ttl)| {
                v.inc_at(delta as u64, Duration::from_secs(*ttl), now);
            });

        Ok(Authorization::Ok)
    }

    #[tracing::instrument(skip_all)]
    fn get_counters(&self, limits: &HashSet<Limit>) -> Result<HashSet<Counter>, StorageErr> {
        let mut res = HashSet::new();

        let namespaces: HashSet<&Namespace> = limits.iter().map(Limit::namespace).collect();
        let limits_by_namespace = self.limits_for_namespace.read().unwrap();

        for namespace in namespaces {
            if let Some(limits) = limits_by_namespace.get(namespace) {
                for limit in limits.keys() {
                    if limits.contains_key(limit) {
                        for (counter, expiring_value) in self.counters_in_namespace(namespace) {
                            let mut counter_with_val = counter.clone();
                            counter_with_val.set_remaining(
                                counter_with_val.max_value() - expiring_value.read() as i64,
                            );
                            counter_with_val.set_expires_in(expiring_value.ttl());
                            if counter_with_val.expires_in().unwrap() > Duration::ZERO {
                                res.insert(counter_with_val);
                            }
                        }
                    }
                }
            }
        }

        for (counter, expiring_value) in self.qualified_counters.iter() {
            if limits.contains(counter.limit()) {
                let mut counter_with_val = counter.deref().clone();
                counter_with_val
                    .set_remaining(counter_with_val.max_value() - expiring_value.read() as i64);
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
        self.limits_for_namespace.write().unwrap().clear();
        Ok(())
    }
}

impl CrInMemoryStorage {
    pub fn new(identifier: String, cache_size: u64) -> Self {
        Self {
            identifier,
            limits_for_namespace: RwLock::new(HashMap::new()),
            qualified_counters: Cache::new(cache_size),
        }
    }

    fn counters_in_namespace(
        &self,
        namespace: &Namespace,
    ) -> HashMap<Counter, CrCounterValue<String>> {
        let mut res: HashMap<Counter, CrCounterValue<String>> = HashMap::new();

        if let Some(counters_by_limit) = self.limits_for_namespace.read().unwrap().get(namespace) {
            for (limit, value) in counters_by_limit {
                res.insert(
                    Counter::new(limit.clone(), HashMap::default()),
                    value.clone(),
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
        if let Some(counters_by_limit) = self
            .limits_for_namespace
            .write()
            .unwrap()
            .get_mut(limit.namespace())
        {
            counters_by_limit.remove(limit);
        }
    }

    fn counter_is_within_limits(counter: &Counter, current_val: Option<&i64>, delta: i64) -> bool {
        match current_val {
            Some(current_val) => current_val + delta <= counter.max_value(),
            None => counter.max_value() >= delta,
        }
    }
}
