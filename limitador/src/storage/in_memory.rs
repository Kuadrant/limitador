use crate::counter::Counter;
use crate::limit::{Limit, Namespace};
use crate::storage::{Storage, StorageErr};
use std::collections::{HashMap, HashSet};
use std::iter::FromIterator;
use std::sync::RwLock;
use std::time::Duration;
use ttl_cache::TtlCache;

pub struct InMemoryStorage {
    limits_for_namespace: RwLock<HashMap<Namespace, HashMap<Limit, HashSet<Counter>>>>,
    counters: RwLock<TtlCache<Counter, i64>>,
}

impl Storage for InMemoryStorage {
    fn get_namespaces(&self) -> Result<HashSet<Namespace>, StorageErr> {
        Ok(HashSet::from_iter(
            self.limits_for_namespace.read().unwrap().keys().cloned(),
        ))
    }

    fn add_limit(&self, limit: &Limit) -> Result<(), StorageErr> {
        let namespace = limit.namespace();

        let mut limits_for_namespace = self.limits_for_namespace.write().unwrap();

        match limits_for_namespace.get_mut(&namespace) {
            Some(limits) => {
                limits.insert(limit.clone(), HashSet::new());
            }
            None => {
                let mut limits = HashMap::new();
                limits.insert(limit.clone(), HashSet::new());
                limits_for_namespace.insert(namespace.clone(), limits);
            }
        }

        Ok(())
    }

    fn get_limits(&self, namespace: &Namespace) -> Result<HashSet<Limit>, StorageErr> {
        let limits = match self.limits_for_namespace.read().unwrap().get(namespace) {
            Some(limits) => HashSet::from_iter(limits.keys().cloned()),
            None => HashSet::new(),
        };

        Ok(limits)
    }

    fn delete_limit(&self, limit: &Limit) -> Result<(), StorageErr> {
        self.delete_counters_of_limit(limit);

        let mut limits_for_namespace = self.limits_for_namespace.write().unwrap();

        if let Some(counters_by_limit) = limits_for_namespace.get_mut(limit.namespace()) {
            counters_by_limit.remove(limit);

            if counters_by_limit.is_empty() {
                limits_for_namespace.remove(limit.namespace());
            }
        }

        Ok(())
    }

    fn delete_limits(&self, namespace: &Namespace) -> Result<(), StorageErr> {
        self.delete_counters_in_namespace(namespace);
        self.limits_for_namespace.write().unwrap().remove(namespace);
        Ok(())
    }

    fn is_within_limits(&self, counter: &Counter, delta: i64) -> Result<bool, StorageErr> {
        let stored_counters = self.counters.read().unwrap();

        Ok(Self::counter_is_within_limits(
            counter,
            stored_counters.get(counter),
            delta,
        ))
    }

    fn update_counter(&self, counter: &Counter, delta: i64) -> Result<(), StorageErr> {
        let mut counters = self.counters.write().unwrap();
        self.insert_or_update_counter(&mut counters, counter, delta);
        Ok(())
    }

    fn check_and_update(
        &self,
        counters: &HashSet<&Counter>,
        delta: i64,
    ) -> Result<bool, StorageErr> {
        // This makes the operator of check + update atomic
        let mut stored_counters = self.counters.write().unwrap();

        for counter in counters {
            if !Self::counter_is_within_limits(counter, stored_counters.get(counter), delta) {
                return Ok(false);
            }
        }

        for &counter in counters {
            self.insert_or_update_counter(&mut stored_counters, counter, delta)
        }

        Ok(true)
    }

    fn get_counters(&self, namespace: &Namespace) -> Result<HashSet<Counter>, StorageErr> {
        let mut res = HashSet::new();

        for counter in self.counters_in_namespace(namespace) {
            if let Some(counter_val) = self.counters.read().unwrap().get(&counter) {
                // TODO: return correct TTL
                let mut counter_with_val = counter.clone();
                counter_with_val.set_remaining(*counter_val);
                res.insert(counter_with_val);
            }
        }

        Ok(res)
    }

    fn clear(&self) -> Result<(), StorageErr> {
        self.counters.write().unwrap().clear();
        self.limits_for_namespace.write().unwrap().clear();
        Ok(())
    }
}

impl InMemoryStorage {
    pub fn new(capacity: usize) -> InMemoryStorage {
        InMemoryStorage {
            limits_for_namespace: RwLock::new(HashMap::new()),
            counters: RwLock::new(TtlCache::new(capacity)),
        }
    }

    fn counters_in_namespace(&self, namespace: &Namespace) -> HashSet<Counter> {
        let mut res: HashSet<Counter> = HashSet::new();

        if let Some(counters_by_limit) = self.limits_for_namespace.read().unwrap().get(namespace) {
            for counter in counters_by_limit.values().flatten() {
                res.insert(counter.clone());
            }
        }

        res
    }

    fn delete_counters_in_namespace(&self, namespace: &Namespace) {
        if let Some(counters_by_limit) = self.limits_for_namespace.read().unwrap().get(namespace) {
            let mut counters = self.counters.write().unwrap();
            for counter in counters_by_limit.values().flatten() {
                counters.remove(counter);
            }
        }
    }

    fn delete_counters_of_limit(&self, limit: &Limit) {
        if let Some(counters_by_limit) = self
            .limits_for_namespace
            .read()
            .unwrap()
            .get(limit.namespace())
        {
            if let Some(counters_of_limit) = counters_by_limit.get(limit) {
                let mut counters = self.counters.write().unwrap();
                for counter in counters_of_limit {
                    counters.remove(counter);
                }
            }
        }
    }

    fn add_counter_limit_association(&self, counter: &Counter) {
        let namespace = counter.limit().namespace();

        if let Some(counters_by_limit) = self
            .limits_for_namespace
            .write()
            .unwrap()
            .get_mut(namespace)
        {
            counters_by_limit
                .get_mut(counter.limit())
                .unwrap()
                .insert(counter.clone());
        }
    }

    fn insert_or_update_counter(
        &self,
        counters: &mut TtlCache<Counter, i64>,
        counter: &Counter,
        delta: i64,
    ) {
        match counters.get_mut(counter) {
            Some(value) => {
                *value -= delta;
            }
            None => {
                counters.insert(
                    counter.clone(),
                    counter.max_value() - delta,
                    Duration::from_secs(counter.seconds()),
                );

                self.add_counter_limit_association(counter);
            }
        }
    }

    fn counter_is_within_limits(counter: &Counter, current_val: Option<&i64>, delta: i64) -> bool {
        match current_val {
            Some(current_val) => current_val - delta >= 0,
            None => counter.max_value() - delta >= 0,
        }
    }
}

impl Default for InMemoryStorage {
    fn default() -> Self {
        Self::new(1000)
    }
}
