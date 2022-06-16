use crate::counter::Counter;
use crate::limit::{Limit, Namespace};
use crate::storage::{Authorization, CounterStorage, StorageErr};
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::sync::RwLock;
use std::time::Duration;
use ttl_cache::TtlCache;

pub struct InMemoryStorage {
    limits_for_namespace: RwLock<HashMap<Namespace, HashMap<Limit, HashSet<Counter>>>>,
    counters: RwLock<TtlCache<Counter, i64>>,
}

impl CounterStorage for InMemoryStorage {
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
        counters: HashSet<Counter>,
        delta: i64,
    ) -> Result<Authorization, StorageErr> {
        // This makes the operator of check + update atomic
        let mut stored_counters = self.counters.write().unwrap();

        for counter in counters.iter() {
            if !Self::counter_is_within_limits(counter, stored_counters.get(counter), delta) {
                return Ok(Authorization::Limited(
                    counter.limit().name().map(|n| n.to_owned()),
                ));
            }
        }

        for counter in counters {
            self.insert_or_update_counter(&mut stored_counters, &counter, delta)
        }

        Ok(Authorization::Ok)
    }

    fn get_counters(&self, limits: &HashSet<Limit>) -> Result<HashSet<Counter>, StorageErr> {
        let mut res = HashSet::new();

        let namespaces: HashSet<&Namespace> = limits.iter().map(Limit::namespace).collect();

        for namespace in namespaces {
            for counter in self.counters_in_namespace(namespace) {
                if let Some(counter_val) = self.counters.read().unwrap().get(&counter) {
                    // TODO: return correct TTL
                    let mut counter_with_val = counter.clone();
                    counter_with_val.set_remaining(*counter_val);
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
        self.counters.write().unwrap().clear();
        self.limits_for_namespace.write().unwrap().clear();
        Ok(())
    }
}

impl InMemoryStorage {
    pub fn new(capacity: usize) -> Self {
        Self {
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

        match self
            .limits_for_namespace
            .write()
            .unwrap()
            .entry(namespace.clone())
        {
            Entry::Occupied(mut e) => {
                e.get_mut()
                    .get_mut(counter.limit())
                    .unwrap()
                    .insert(counter.clone());
            }
            Entry::Vacant(e) => {
                let mut counters = HashSet::new();
                counters.insert(counter.clone());
                let mut map = HashMap::new();
                map.insert(counter.limit().clone(), counters);
                e.insert(map);
            }
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
