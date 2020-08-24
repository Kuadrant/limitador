use crate::counter::Counter;
use crate::limit::Limit;
use crate::storage::{Storage, StorageErr};
use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::iter::FromIterator;
use std::sync::RwLock;
use std::time::{Duration, SystemTime};

// This is a storage implementation that can be compiled to WASM. It is very
// similar to the "InMemory" one. The InMemory implementation cannot be used in
// WASM, because it relies on std:time functions. This implementation avoids
// that.

pub trait Clock: Sync + Send {
    fn get_current_time(&self) -> SystemTime;
}

pub struct CacheEntry<V> {
    pub value: V,
    pub expires_at: SystemTime,
}

impl<V: Copy> CacheEntry<V> {
    fn is_expired(&self, current_time: SystemTime) -> bool {
        current_time > self.expires_at
    }
}

pub struct Cache<K: Eq + Hash, V: Copy> {
    pub map: HashMap<K, CacheEntry<V>>,
}

impl<K: Eq + Hash + Clone, V: Copy> Cache<K, V> {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    pub fn get(&self, key: &K) -> Option<&CacheEntry<V>> {
        self.map.get(&key)
    }

    pub fn get_mut(&mut self, key: &K) -> Option<&mut CacheEntry<V>> {
        self.map.get_mut(&key)
    }

    pub fn insert(&mut self, key: &K, value: V, expires_at: SystemTime) {
        self.map
            .insert(key.clone(), CacheEntry { value, expires_at });
    }

    pub fn remove(&mut self, key: &K) {
        self.map.remove(key);
    }

    pub fn get_all(&mut self, current_time: SystemTime) -> Vec<(K, V, SystemTime)> {
        let iterator = self
            .map
            .iter()
            .filter(|(_key, cache_entry)| !cache_entry.is_expired(current_time))
            .map(|(key, cache_entry)| (key.clone(), cache_entry.value, cache_entry.expires_at));

        Vec::from_iter(iterator)
    }
}

impl<K: Eq + Hash + Clone, V: Copy> Default for Cache<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct WasmStorage {
    limits_for_namespace: RwLock<HashMap<String, HashMap<Limit, HashSet<Counter>>>>,
    pub counters: RwLock<Cache<Counter, i64>>,
    pub clock: Box<dyn Clock>,
}

impl Storage for WasmStorage {
    fn add_limit(&self, limit: &Limit) -> Result<(), StorageErr> {
        let namespace = limit.namespace().to_string();

        let mut limits_for_namespace = self.limits_for_namespace.write().unwrap();

        match limits_for_namespace.get_mut(&namespace) {
            Some(limits) => {
                limits.insert(limit.clone(), HashSet::new());
            }
            None => {
                let mut limits = HashMap::new();
                limits.insert(limit.clone(), HashSet::new());
                limits_for_namespace.insert(namespace, limits);
            }
        }

        Ok(())
    }

    fn get_limits(&self, namespace: &str) -> Result<HashSet<Limit>, StorageErr> {
        let limits = match self.limits_for_namespace.read().unwrap().get(namespace) {
            Some(limits) => HashSet::from_iter(limits.keys().cloned()),
            None => HashSet::new(),
        };

        Ok(limits)
    }

    fn delete_limit(&self, limit: &Limit) -> Result<(), StorageErr> {
        self.delete_counters_of_limit(limit);

        if let Some(counters_by_limit) = self
            .limits_for_namespace
            .write()
            .unwrap()
            .get_mut(limit.namespace())
        {
            counters_by_limit.remove(limit);
        }

        Ok(())
    }

    fn delete_limits(&self, namespace: &str) -> Result<(), StorageErr> {
        self.delete_counters_in_namespace(namespace);
        self.limits_for_namespace.write().unwrap().remove(namespace);
        Ok(())
    }

    fn is_within_limits(&self, counter: &Counter, delta: i64) -> Result<bool, StorageErr> {
        let within_limits = match self.counters.read().unwrap().get(counter) {
            Some(entry) => {
                if entry.is_expired(self.clock.get_current_time()) {
                    true
                } else {
                    entry.value - delta >= 0
                }
            }
            None => true,
        };

        Ok(within_limits)
    }

    fn update_counter(&self, counter: &Counter, delta: i64) -> Result<(), StorageErr> {
        let mut counters = self.counters.write().unwrap();

        match counters.get_mut(counter) {
            Some(entry) => {
                if entry.is_expired(self.clock.get_current_time()) {
                    // TODO: remove duplication. "None" branch is identical.
                    counters.insert(
                        counter,
                        counter.max_value() - delta,
                        self.clock.get_current_time() + Duration::from_secs(counter.seconds()),
                    );
                } else {
                    entry.value -= delta;
                }
            }
            None => {
                counters.insert(
                    counter,
                    counter.max_value() - delta,
                    self.clock.get_current_time() + Duration::from_secs(counter.seconds()),
                );

                self.add_counter_limit_association(counter);
            }
        };

        Ok(())
    }

    fn get_counters(&self, namespace: &str) -> Result<HashSet<Counter>, StorageErr> {
        // TODO: optimize to avoid iterating over all of them.

        let counters_with_vals: Vec<Counter> = self
            .counters
            .write()
            .unwrap()
            .get_all(self.clock.get_current_time())
            .iter()
            .filter(|(counter, _, _)| counter.namespace() == namespace)
            .map(|(counter, value, expires_at)| {
                let mut counter_with_val =
                    Counter::new(counter.limit().clone(), counter.set_variables().clone());
                counter_with_val.set_remaining(*value);
                counter_with_val.set_expires_in(
                    (expires_at.duration_since(SystemTime::UNIX_EPOCH).unwrap()
                        - self
                            .clock
                            .get_current_time()
                            .duration_since(SystemTime::UNIX_EPOCH)
                            .unwrap())
                    .as_secs(),
                );
                counter_with_val
            })
            .collect();

        Ok(HashSet::from_iter(counters_with_vals.iter().cloned()))
    }
}

impl WasmStorage {
    pub fn new(clock: Box<impl Clock + 'static>) -> Self {
        Self {
            limits_for_namespace: RwLock::new(HashMap::new()),
            counters: RwLock::new(Cache::default()),
            clock,
        }
    }

    pub fn add_counter(&self, counter: &Counter, value: i64, expires_at: SystemTime) {
        self.counters
            .write()
            .unwrap()
            .insert(counter, value, expires_at);
    }

    fn delete_counters_in_namespace(&self, namespace: &str) {
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
}
