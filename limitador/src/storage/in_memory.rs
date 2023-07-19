use crate::counter::Counter;
use crate::limit::{Limit, Namespace};
use crate::storage::atomic_expiring_value::AtomicExpiringValue;
use crate::storage::{Authorization, CounterStorage, StorageErr};
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::RwLock;
use std::time::{Duration, SystemTime};

#[derive(Eq, Clone)]
struct CounterKey {
    set_variables: HashMap<String, String>,
}

impl CounterKey {
    fn to_counter(&self, limit: &Limit) -> Counter {
        Counter::new(limit.clone(), self.set_variables.clone())
    }
}

impl From<&Counter> for CounterKey {
    fn from(counter: &Counter) -> Self {
        CounterKey {
            set_variables: counter.set_variables().clone(),
        }
    }
}

impl From<&mut Counter> for CounterKey {
    fn from(counter: &mut Counter) -> Self {
        CounterKey {
            set_variables: counter.set_variables().clone(),
        }
    }
}

impl Hash for CounterKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.set_variables.iter().for_each(|(k, v)| {
            k.hash(state);
            v.hash(state);
        });
    }
}

impl PartialEq for CounterKey {
    fn eq(&self, other: &Self) -> bool {
        self.set_variables == other.set_variables
    }
}

type NamespacedLimitCounters<T> = HashMap<Namespace, HashMap<Limit, HashMap<CounterKey, T>>>;

pub struct InMemoryStorage {
    limits_for_namespace: RwLock<NamespacedLimitCounters<AtomicExpiringValue>>,
}

impl CounterStorage for InMemoryStorage {
    fn is_within_limits(&self, counter: &Counter, delta: i64) -> Result<bool, StorageErr> {
        let limits_by_namespace = self.limits_for_namespace.read().unwrap();

        let mut value = 0;
        if let Some(limits) = limits_by_namespace.get(counter.limit().namespace()) {
            if let Some(counters) = limits.get(counter.limit()) {
                if let Some(expiring_value) = counters.get(&counter.into()) {
                    value = expiring_value.value();
                }
            }
        }
        Ok(counter.max_value() >= value + delta)
    }

    fn update_counter(&self, counter: &Counter, delta: i64) -> Result<(), StorageErr> {
        let mut limits_by_namespace = self.limits_for_namespace.write().unwrap();
        match limits_by_namespace.entry(counter.limit().namespace().clone()) {
            Entry::Vacant(v) => {
                let mut limits = HashMap::new();
                let mut counters = HashMap::new();
                self.insert_or_update_counter(&mut counters, counter, delta);
                limits.insert(counter.limit().clone(), counters);
                v.insert(limits);
            }
            Entry::Occupied(mut o) => match o.get_mut().entry(counter.limit().clone()) {
                Entry::Vacant(v) => {
                    let mut counters = HashMap::new();
                    self.insert_or_update_counter(&mut counters, counter, delta);
                    v.insert(counters);
                }
                Entry::Occupied(mut o) => {
                    self.insert_or_update_counter(o.get_mut(), counter, delta);
                }
            },
        }
        Ok(())
    }

    fn check_and_update(
        &self,
        counters: &mut Vec<Counter>,
        delta: i64,
        load_counters: bool,
    ) -> Result<Authorization, StorageErr> {
        let mut limits_by_namespace = self.limits_for_namespace.write().unwrap();
        let mut first_limited = None;

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

        for counter in counters.iter_mut() {
            if counter.max_value() < delta {
                if let Some(limited) = process_counter(counter, 0, delta) {
                    if !load_counters {
                        return Ok(limited);
                    }
                }
                continue;
            }

            let value = Some(
                limits_by_namespace
                    .get(counter.limit().namespace())
                    .and_then(|limits| limits.get(counter.limit()))
                    .and_then(|counters| counters.get(&counter.into()))
                    .map(|expiring_value| expiring_value.value())
                    .unwrap_or(0),
            );

            if let Some(limited) = process_counter(counter, value.unwrap(), delta) {
                if !load_counters {
                    return Ok(limited);
                }
            }
        }

        if let Some(limited) = first_limited {
            return Ok(limited);
        }

        for counter in counters.iter_mut() {
            let now = SystemTime::now();
            match limits_by_namespace
                .entry(counter.limit().namespace().clone())
                .or_insert_with(HashMap::new)
                .entry(counter.limit().clone())
                .or_insert_with(HashMap::new)
                .entry(counter.into())
            {
                Entry::Vacant(v) => {
                    v.insert(AtomicExpiringValue::new(
                        delta,
                        now + Duration::from_secs(counter.seconds()),
                    ));
                }
                Entry::Occupied(o) => {
                    o.get().update(delta, counter.seconds(), now);
                }
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
    pub fn new() -> Self {
        Self {
            limits_for_namespace: RwLock::new(HashMap::new()),
        }
    }

    fn counters_in_namespace(
        &self,
        namespace: &Namespace,
    ) -> HashMap<Counter, AtomicExpiringValue> {
        let mut res: HashMap<Counter, AtomicExpiringValue> = HashMap::new();

        if let Some(counters_by_limit) = self.limits_for_namespace.read().unwrap().get(namespace) {
            for (limit, values) in counters_by_limit {
                for (counter_key, expiring_value) in values {
                    res.insert(counter_key.to_counter(limit), expiring_value.clone());
                }
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

    fn insert_or_update_counter(
        &self,
        counters: &mut HashMap<CounterKey, AtomicExpiringValue>,
        counter: &Counter,
        delta: i64,
    ) {
        let now = SystemTime::now();
        match counters.entry(counter.into()) {
            Entry::Vacant(v) => {
                v.insert(AtomicExpiringValue::new(
                    delta,
                    now + Duration::from_secs(counter.seconds()),
                ));
            }
            Entry::Occupied(o) => {
                o.get().update(delta, counter.seconds(), now);
            }
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
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_for_multiple_limit_per_ns() {
        let storage = InMemoryStorage::new();
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
