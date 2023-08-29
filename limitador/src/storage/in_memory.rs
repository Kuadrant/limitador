use crate::counter::Counter;
use crate::limit::{Limit, Namespace};
use crate::storage::atomic_expiring_value::AtomicExpiringValue;
use crate::storage::{Authorization, CounterStorage, StorageErr};
use moka::sync::Cache;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::ops::Deref;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};

struct CounterAuthorizationValues {
    max_value: i64,
    seconds: u64,
    limit_name: Option<String>,
}

impl From<&mut Counter> for CounterAuthorizationValues {
    fn from(counter: &mut Counter) -> Self {
        Self {
            max_value: counter.max_value(),
            seconds: counter.seconds(),
            limit_name: counter.limit().name().map(|n| n.to_owned()),
        }
    }
}

type NamespacedLimitCounters<T> = HashMap<Namespace, HashMap<Limit, T>>;

pub struct InMemoryStorage {
    limits_for_namespace: RwLock<NamespacedLimitCounters<AtomicExpiringValue>>,
    qualified_counters: Cache<Counter, Arc<AtomicExpiringValue>>,
}

impl CounterStorage for InMemoryStorage {
    fn is_within_limits(&self, counter: &Counter, delta: i64) -> Result<bool, StorageErr> {
        let limits_by_namespace = self.limits_for_namespace.read().unwrap();

        let mut value = 0;

        if counter.is_qualified() {
            if let Some(counter) = self.qualified_counters.get(counter) {
                value = counter.value();
            }
        } else if let Some(limits) = limits_by_namespace.get(counter.limit().namespace()) {
            if let Some(counter) = limits.get(counter.limit()) {
                value = counter.value();
            }
        }

        Ok(counter.max_value() >= value + delta)
    }

    fn add_counter(&self, limit: &Limit) -> Result<(), StorageErr> {
        if limit.variables().is_empty() {
            let mut limits_by_namespace = self.limits_for_namespace.write().unwrap();
            limits_by_namespace
                .entry(limit.namespace().clone())
                .or_insert_with(HashMap::new)
                .entry(limit.clone())
                .or_insert_with(AtomicExpiringValue::default);
        }
        Ok(())
    }

    fn update_counter(&self, counter: &Counter, delta: i64) -> Result<(), StorageErr> {
        let mut limits_by_namespace = self.limits_for_namespace.write().unwrap();
        let now = SystemTime::now();
        if counter.is_qualified() {
            let value = match self.qualified_counters.get(counter) {
                None => self.qualified_counters.get_with(counter.clone(), || {
                    Arc::new(AtomicExpiringValue::new(
                        0,
                        now + Duration::from_secs(counter.seconds()),
                    ))
                }),
                Some(counter) => counter,
            };
            value.update(delta, counter.seconds(), now);
        } else {
            match limits_by_namespace.entry(counter.limit().namespace().clone()) {
                Entry::Vacant(v) => {
                    let mut limits = HashMap::new();
                    limits.insert(
                        counter.limit().clone(),
                        AtomicExpiringValue::new(
                            delta,
                            now + Duration::from_secs(counter.seconds()),
                        ),
                    );
                    v.insert(limits);
                }
                Entry::Occupied(mut o) => match o.get_mut().entry(counter.limit().clone()) {
                    Entry::Vacant(v) => {
                        v.insert(AtomicExpiringValue::new(
                            delta,
                            now + Duration::from_secs(counter.seconds()),
                        ));
                    }
                    Entry::Occupied(o) => {
                        o.get().update(delta, counter.seconds(), now);
                    }
                },
            }
        }
        Ok(())
    }

    fn check_and_update(
        &self,
        counters: &mut Vec<Counter>,
        delta: i64,
        load_counters: bool,
    ) -> Result<Authorization, StorageErr> {
        let limits_by_namespace = self.limits_for_namespace.read().unwrap();
        let mut first_limited = None;
        let mut counter_values_to_update: Vec<(&AtomicExpiringValue, CounterAuthorizationValues)> =
            Vec::new();
        let mut qualified_counter_values_to_updated: Vec<(
            Arc<AtomicExpiringValue>,
            CounterAuthorizationValues,
        )> = Vec::new();
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

        let check_post_update = |counter_values: CounterAuthorizationValues,
                                 expiring_value: &AtomicExpiringValue,
                                 delta: i64|
         -> Option<Authorization> {
            let value = expiring_value.update(delta, counter_values.seconds, now);
            if value > counter_values.max_value {
                return Option::from(Authorization::Limited(counter_values.limit_name));
            }
            None
        };
        // Process simple counters
        for counter in counters.iter_mut().filter(|c| !c.is_qualified()) {
            let atomic_expiring_value: &AtomicExpiringValue = limits_by_namespace
                .get(counter.limit().namespace())
                .and_then(|limits| limits.get(counter.limit()))
                .unwrap();

            if let Some(limited) = process_counter(counter, atomic_expiring_value.value(), delta) {
                if !load_counters {
                    return Ok(limited);
                }
            }
            counter_values_to_update.push((atomic_expiring_value, counter.into()));
        }

        // Process qualified counters
        for counter in counters.iter_mut().filter(|c| c.is_qualified()) {
            let value = match self.qualified_counters.get(counter) {
                None => self.qualified_counters.get_with(counter.clone(), || {
                    Arc::new(AtomicExpiringValue::new(
                        0,
                        now + Duration::from_secs(counter.seconds()),
                    ))
                }),
                Some(counter) => counter,
            };

            if let Some(limited) = process_counter(counter, value.value(), delta) {
                if !load_counters {
                    return Ok(limited);
                }
            }

            qualified_counter_values_to_updated.push((value, counter.into()));
        }

        if let Some(limited) = first_limited {
            return Ok(limited);
        }

        // Update and check counters
        counter_values_to_update.sort_by(|a, b| a.0.ttl().cmp(&b.0.ttl()));

        for (expiring_value, counter_values) in counter_values_to_update {
            if let Some(limited) = check_post_update(counter_values, expiring_value, delta) {
                return Ok(limited);
            }
        }

        qualified_counter_values_to_updated.sort_by(|a, b| a.0.ttl().cmp(&b.0.ttl()));

        for (arc_expiring_value, counter_values) in qualified_counter_values_to_updated {
            if let Some(limited) = check_post_update(counter_values, &arc_expiring_value, delta) {
                return Ok(limited);
            }
        }

        Ok(Authorization::Ok)
    }

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
                                counter_with_val.max_value() - expiring_value.value(),
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
                    .set_remaining(counter_with_val.max_value() - expiring_value.value());
                counter_with_val.set_expires_in(expiring_value.ttl());
                if counter_with_val.expires_in().unwrap() > Duration::ZERO {
                    res.insert(counter_with_val);
                }
            }
        }

        Ok(res)
    }

    fn delete_counters(&self, limits: HashSet<Limit>) -> Result<(), StorageErr> {
        for limit in limits {
            self.delete_counters_of_limit(&limit);
        }
        Ok(())
    }

    fn clear(&self) -> Result<(), StorageErr> {
        self.limits_for_namespace.write().unwrap().clear();
        Ok(())
    }
}

impl InMemoryStorage {
    pub fn new(cache_size: u64) -> Self {
        Self {
            limits_for_namespace: RwLock::new(HashMap::new()),
            qualified_counters: Cache::new(cache_size),
        }
    }

    fn counters_in_namespace(
        &self,
        namespace: &Namespace,
    ) -> HashMap<Counter, AtomicExpiringValue> {
        let mut res: HashMap<Counter, AtomicExpiringValue> = HashMap::new();

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
