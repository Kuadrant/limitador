use crate::counter::Counter;
use crate::limit::Limit;
use crate::storage::{Storage, StorageErr};
use std::collections::hash_map::RandomState;
use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::iter::FromIterator;
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
    pub limits_for_namespace: HashMap<String, HashSet<Limit>>,
    pub counters: Cache<Counter, i64>,
    pub clock: Box<dyn Clock>,
}

impl Storage for WasmStorage {
    fn add_limit(&mut self, limit: Limit) -> Result<(), StorageErr> {
        let namespace = limit.namespace().to_string();

        match self.limits_for_namespace.get_mut(&namespace) {
            Some(value) => {
                value.insert(limit);
            }
            None => {
                let mut limits = HashSet::new();
                limits.insert(limit);
                self.limits_for_namespace.insert(namespace, limits);
            }
        }

        Ok(())
    }

    fn get_limits(&self, namespace: &str) -> Result<HashSet<Limit, RandomState>, StorageErr> {
        let limits = match self.limits_for_namespace.get(namespace) {
            Some(limits) => limits.clone(),
            None => HashSet::new(),
        };

        Ok(limits)
    }

    fn delete_limit(&mut self, limit: &Limit) -> Result<(), StorageErr> {
        // TODO: delete counters associated with the limit too.

        let namespace = limit.namespace().to_string();

        if let Some(value) = self.limits_for_namespace.get_mut(&namespace) {
            value.remove(limit);
        }

        Ok(())
    }

    fn delete_limits(&mut self, namespace: &str) -> Result<(), StorageErr> {
        // TODO: delete counters associated with the limits.

        self.limits_for_namespace.remove(namespace);
        Ok(())
    }

    fn is_within_limits(&self, counter: &Counter, delta: i64) -> Result<bool, StorageErr> {
        let within_limits = match self.counters.get(counter) {
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

    fn update_counter(&mut self, counter: &Counter, delta: i64) -> Result<(), StorageErr> {
        match self.counters.get_mut(counter) {
            Some(entry) => {
                if entry.is_expired(self.clock.get_current_time()) {
                    // TODO: remove duplication. "None" branch is identical.
                    self.counters.insert(
                        counter,
                        counter.max_value() - 1,
                        self.clock.get_current_time() + Duration::from_secs(counter.seconds()),
                    );
                } else {
                    entry.value -= delta;
                }
            }
            None => {
                self.counters.insert(
                    counter,
                    counter.max_value() - 1,
                    self.clock.get_current_time() + Duration::from_secs(counter.seconds()),
                );
            }
        };

        Ok(())
    }

    fn get_counters(&mut self, namespace: &str) -> Vec<(Counter, i64, Duration)> {
        self.counters
            .get_all(self.clock.get_current_time())
            .iter()
            .filter(|(counter, _, _)| counter.namespace() == namespace)
            .map(|(counter, value, expires_at)| {
                (
                    counter.clone(),
                    *value,
                    expires_at.duration_since(SystemTime::UNIX_EPOCH).unwrap()
                        - self
                            .clock
                            .get_current_time()
                            .duration_since(SystemTime::UNIX_EPOCH)
                            .unwrap(),
                )
            })
            .collect()
    }
}

impl WasmStorage {
    pub fn new(clock: Box<impl Clock + 'static>) -> Self {
        Self {
            limits_for_namespace: HashMap::new(),
            counters: Cache::default(),
            clock,
        }
    }

    pub fn add_counter(&mut self, counter: &Counter, value: i64, expires_at: SystemTime) {
        self.counters.insert(counter, value, expires_at);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub struct TestClock {}
    impl Clock for TestClock {
        fn get_current_time(&self) -> SystemTime {
            SystemTime::now()
        }
    }

    #[test]
    fn apply_limit() {
        let max_hits = 3;
        let namespace = "test_namespace";

        let limit = Limit::new(
            namespace,
            max_hits,
            60,
            vec!["req.method == GET"],
            vec!["req.method", "app_id"],
        );

        let mut storage = WasmStorage::new(Box::new(TestClock {}));

        storage.add_limit(limit.clone()).unwrap();

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("namespace".to_string(), namespace.to_string());
        values.insert("req.method".to_string(), "GET".to_string());
        values.insert("app_id".to_string(), "test_app_id".to_string());

        let counter = Counter::new(limit, values);

        for _ in 0..max_hits {
            assert!(storage.is_within_limits(&counter, 1).unwrap());
            storage.update_counter(&counter, 1).unwrap();
        }
        assert_eq!(false, storage.is_within_limits(&counter, 1).unwrap());
    }
}
