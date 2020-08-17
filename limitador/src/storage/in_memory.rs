use crate::counter::Counter;
use crate::limit::Limit;
use crate::storage::{Storage, StorageErr};
use std::collections::{HashMap, HashSet};
use std::iter::FromIterator;
use std::time::Duration;
use ttl_cache::TtlCache;

pub struct InMemoryStorage {
    limits_for_namespace: HashMap<String, HashMap<Limit, HashSet<Counter>>>,
    counters: TtlCache<Counter, i64>,
}

impl Storage for InMemoryStorage {
    fn add_limit(&mut self, limit: &Limit) -> Result<(), StorageErr> {
        let namespace = limit.namespace().to_string();

        match self.limits_for_namespace.get_mut(&namespace) {
            Some(limits) => {
                limits.insert(limit.clone(), HashSet::new());
            }
            None => {
                let mut limits = HashMap::new();
                limits.insert(limit.clone(), HashSet::new());
                self.limits_for_namespace.insert(namespace, limits);
            }
        }

        Ok(())
    }

    fn get_limits(&self, namespace: &str) -> Result<HashSet<Limit>, StorageErr> {
        let limits = match self.limits_for_namespace.get(namespace) {
            Some(limits) => HashSet::from_iter(limits.keys().cloned()),
            None => HashSet::new(),
        };

        Ok(limits)
    }

    fn delete_limit(&mut self, limit: &Limit) -> Result<(), StorageErr> {
        self.delete_counters_of_limit(limit);

        if let Some(counters_by_limit) = self.limits_for_namespace.get_mut(limit.namespace()) {
            counters_by_limit.remove(limit);
        }

        Ok(())
    }

    fn delete_limits(&mut self, namespace: &str) -> Result<(), StorageErr> {
        self.delete_counters_in_namespace(namespace);
        self.limits_for_namespace.remove(namespace);
        Ok(())
    }

    fn is_within_limits(&self, counter: &Counter, delta: i64) -> Result<bool, StorageErr> {
        let within_limits = match self.counters.get(counter) {
            Some(value) => *value - delta >= 0,
            None => true,
        };

        Ok(within_limits)
    }

    fn update_counter(&mut self, counter: &Counter, delta: i64) -> Result<(), StorageErr> {
        match self.counters.get_mut(counter) {
            Some(value) => {
                *value -= delta;
            }
            None => {
                self.counters.insert(
                    counter.clone(),
                    counter.max_value() - delta,
                    Duration::from_secs(counter.seconds()),
                );

                self.add_counter_limit_association(counter);
            }
        };

        Ok(())
    }

    fn get_counters(&mut self, namespace: &str) -> Result<HashSet<Counter>, StorageErr> {
        let mut res = HashSet::new();

        for counter in self.counters_in_namespace(namespace) {
            if let Some(counter_val) = self.counters.get(counter) {
                // TODO: return correct TTL
                let mut counter_with_val = counter.clone();
                counter_with_val.set_remaining(*counter_val);
                res.insert(counter_with_val);
            }
        }

        Ok(res)
    }
}

impl InMemoryStorage {
    pub fn new(capacity: usize) -> InMemoryStorage {
        InMemoryStorage {
            limits_for_namespace: HashMap::new(),
            counters: TtlCache::new(capacity),
        }
    }

    fn counters_in_namespace(&self, namespace: &str) -> HashSet<&Counter> {
        match self.limits_for_namespace.get(namespace) {
            Some(counters_by_limit) => HashSet::from_iter(counters_by_limit.values().flatten()),
            None => HashSet::new(),
        }
    }

    fn delete_counters_in_namespace(&mut self, namespace: &str) {
        if let Some(counters_by_limit) = self.limits_for_namespace.get(namespace) {
            for counter in counters_by_limit.values().flatten() {
                self.counters.remove(counter);
            }
        }
    }

    fn delete_counters_of_limit(&mut self, limit: &Limit) {
        if let Some(counters_by_limit) = self.limits_for_namespace.get(limit.namespace()) {
            if let Some(counters) = counters_by_limit.get(limit) {
                for counter in counters.iter() {
                    self.counters.remove(counter);
                }
            }
        }
    }

    fn add_counter_limit_association(&mut self, counter: &Counter) {
        let namespace = counter.limit().namespace();

        if let Some(counters_by_limit) = self.limits_for_namespace.get_mut(namespace) {
            counters_by_limit
                .get_mut(counter.limit())
                .unwrap()
                .insert(counter.clone());
        }
    }
}

impl Default for InMemoryStorage {
    fn default() -> Self {
        Self::new(1000)
    }
}
