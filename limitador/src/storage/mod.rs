use crate::counter::Counter;
use crate::limit::{Limit, Namespace};
use crate::prometheus_metrics::CounterAccess;
use crate::InMemoryStorage;
use async_trait::async_trait;
use std::collections::{HashMap, HashSet};
use std::sync::RwLock;
use thiserror::Error;

#[cfg(feature = "disk_storage")]
pub mod disk;
pub mod in_memory;
pub mod wasm;

#[cfg(feature = "redis_storage")]
pub mod redis;

#[cfg(feature = "infinispan_storage")]
pub mod infinispan;

mod atomic_expiring_value;
#[cfg(any(
    feature = "disk_storage",
    feature = "infinispan_storage",
    feature = "redis_storage"
))]
mod keys;

pub enum Authorization {
    Ok,
    Limited(Option<String>), // First counter found over the limits
}

pub struct Storage {
    limits: RwLock<HashMap<Namespace, HashSet<Limit>>>,
    counters: Box<dyn CounterStorage>,
}

pub struct AsyncStorage {
    limits: RwLock<HashMap<Namespace, HashSet<Limit>>>,
    counters: Box<dyn AsyncCounterStorage>,
}

impl Storage {
    pub fn new(cache_size: u64) -> Self {
        Self {
            limits: RwLock::new(HashMap::new()),
            counters: Box::new(InMemoryStorage::new(cache_size)),
        }
    }

    pub fn with_counter_storage(counters: Box<dyn CounterStorage>) -> Self {
        Self {
            limits: RwLock::new(HashMap::new()),
            counters,
        }
    }

    pub fn get_namespaces(&self) -> HashSet<Namespace> {
        self.limits.read().unwrap().keys().cloned().collect()
    }

    pub fn add_limit(&self, limit: Limit) -> bool {
        let namespace = limit.namespace().clone();
        let mut limits = self.limits.write().unwrap();
        self.counters.add_counter(&limit).unwrap();
        limits.entry(namespace).or_default().insert(limit)
    }

    pub fn update_limit(&self, update: &Limit) -> bool {
        let mut namespaces = self.limits.write().unwrap();
        let limits = namespaces.get_mut(update.namespace());
        if let Some(limits) = limits {
            let req_update = if let Some(limit) = limits.get(update) {
                limit.max_value() != update.max_value() || limit.name() != update.name()
            } else {
                false
            };
            if req_update {
                limits.remove(update);
                limits.insert(update.clone());
                return true;
            }
        }
        false
    }

    pub fn get_limits(&self, namespace: &Namespace) -> HashSet<Limit> {
        match self.limits.read().unwrap().get(namespace) {
            Some(limits) => limits.clone(),
            None => HashSet::new(),
        }
    }

    pub fn delete_limit(&self, limit: &Limit) -> Result<(), StorageErr> {
        let mut limits = HashSet::new();
        limits.insert(limit.clone());
        self.counters.delete_counters(limits)?;

        let mut limits = self.limits.write().unwrap();

        if let Some(limits_for_ns) = limits.get_mut(limit.namespace()) {
            limits_for_ns.remove(limit);

            if limits_for_ns.is_empty() {
                limits.remove(limit.namespace());
            }
        }
        Ok(())
    }

    pub fn delete_limits(&self, namespace: &Namespace) -> Result<(), StorageErr> {
        if let Some(data) = self.limits.write().unwrap().remove(namespace) {
            self.counters.delete_counters(data)?;
        }
        Ok(())
    }

    pub fn is_within_limits(&self, counter: &Counter, delta: i64) -> Result<bool, StorageErr> {
        self.counters.is_within_limits(counter, delta)
    }

    pub fn update_counter(&self, counter: &Counter, delta: i64) -> Result<(), StorageErr> {
        self.counters.update_counter(counter, delta)
    }

    pub fn check_and_update(
        &self,
        counters: &mut Vec<Counter>,
        delta: i64,
        load_counters: bool,
    ) -> Result<Authorization, StorageErr> {
        self.counters
            .check_and_update(counters, delta, load_counters)
    }

    pub fn get_counters(&self, namespace: &Namespace) -> Result<HashSet<Counter>, StorageErr> {
        match self.limits.read().unwrap().get(namespace) {
            Some(limits) => self.counters.get_counters(limits),
            None => Ok(HashSet::new()),
        }
    }

    pub fn clear(&self) -> Result<(), StorageErr> {
        self.limits.write().unwrap().clear();
        self.counters.clear()
    }
}

impl AsyncStorage {
    pub fn with_counter_storage(counters: Box<dyn AsyncCounterStorage>) -> Self {
        Self {
            limits: RwLock::new(HashMap::new()),
            counters,
        }
    }

    pub fn get_namespaces(&self) -> HashSet<Namespace> {
        self.limits.read().unwrap().keys().cloned().collect()
    }

    pub fn add_limit(&self, limit: Limit) -> bool {
        let namespace = limit.namespace().clone();

        let mut limits_for_namespace = self.limits.write().unwrap();

        match limits_for_namespace.get_mut(&namespace) {
            Some(limits) => limits.insert(limit),
            None => {
                let mut limits = HashSet::new();
                limits.insert(limit);
                limits_for_namespace.insert(namespace, limits);
                true
            }
        }
    }

    pub fn update_limit(&self, update: &Limit) -> bool {
        let mut namespaces = self.limits.write().unwrap();
        let limits = namespaces.get_mut(update.namespace());
        if let Some(limits) = limits {
            let req_update = if let Some(limit) = limits.get(update) {
                limit.max_value() != update.max_value() || limit.name() != update.name()
            } else {
                false
            };
            if req_update {
                limits.remove(update);
                limits.insert(update.clone());
                return true;
            }
        }
        false
    }

    pub fn get_limits(&self, namespace: &Namespace) -> HashSet<Limit> {
        match self.limits.read().unwrap().get(namespace) {
            Some(limits) => limits.iter().cloned().collect(),
            None => HashSet::new(),
        }
    }

    pub async fn delete_limit(&self, limit: &Limit) -> Result<(), StorageErr> {
        let mut limits = HashSet::new();
        limits.insert(limit.clone());
        self.counters.delete_counters(limits).await?;

        let mut limits_for_namespace = self.limits.write().unwrap();

        if let Some(counters_by_limit) = limits_for_namespace.get_mut(limit.namespace()) {
            counters_by_limit.remove(limit);

            if counters_by_limit.is_empty() {
                limits_for_namespace.remove(limit.namespace());
            }
        }
        Ok(())
    }

    pub async fn delete_limits(&self, namespace: &Namespace) -> Result<(), StorageErr> {
        let option = { self.limits.write().unwrap().remove(namespace) };
        if let Some(data) = option {
            let limits = data.iter().cloned().collect();
            self.counters.delete_counters(limits).await?;
        }
        Ok(())
    }

    pub async fn is_within_limits(
        &self,
        counter: &Counter,
        delta: i64,
    ) -> Result<bool, StorageErr> {
        self.counters.is_within_limits(counter, delta).await
    }

    pub async fn update_counter(&self, counter: &Counter, delta: i64) -> Result<(), StorageErr> {
        self.counters.update_counter(counter, delta).await
    }

    pub async fn check_and_update<'a>(
        &self,
        counters: &mut Vec<Counter>,
        delta: i64,
        load_counters: bool,
        counter_access: CounterAccess<'a>,
    ) -> Result<Authorization, StorageErr> {
        self.counters
            .check_and_update(counters, delta, load_counters, counter_access)
            .await
    }

    pub async fn get_counters(
        &self,
        namespace: &Namespace,
    ) -> Result<HashSet<Counter>, StorageErr> {
        let limits = self.get_limits(namespace);
        self.counters.get_counters(limits).await
    }

    pub async fn clear(&self) -> Result<(), StorageErr> {
        self.limits.write().unwrap().clear();
        self.counters.clear().await
    }
}

pub trait CounterStorage: Sync + Send {
    fn is_within_limits(&self, counter: &Counter, delta: i64) -> Result<bool, StorageErr>;
    fn add_counter(&self, limit: &Limit) -> Result<(), StorageErr>;
    fn update_counter(&self, counter: &Counter, delta: i64) -> Result<(), StorageErr>;
    fn check_and_update(
        &self,
        counters: &mut Vec<Counter>,
        delta: i64,
        load_counters: bool,
    ) -> Result<Authorization, StorageErr>;
    fn get_counters(&self, limits: &HashSet<Limit>) -> Result<HashSet<Counter>, StorageErr>;
    fn delete_counters(&self, limits: HashSet<Limit>) -> Result<(), StorageErr>;
    fn clear(&self) -> Result<(), StorageErr>;
}

#[async_trait]
pub trait AsyncCounterStorage: Sync + Send {
    async fn is_within_limits(&self, counter: &Counter, delta: i64) -> Result<bool, StorageErr>;
    async fn update_counter(&self, counter: &Counter, delta: i64) -> Result<(), StorageErr>;
    async fn check_and_update<'a>(
        &self,
        counters: &mut Vec<Counter>,
        delta: i64,
        load_counters: bool,
        counter_access: CounterAccess<'a>,
    ) -> Result<Authorization, StorageErr>;
    async fn get_counters(&self, limits: HashSet<Limit>) -> Result<HashSet<Counter>, StorageErr>;
    async fn delete_counters(&self, limits: HashSet<Limit>) -> Result<(), StorageErr>;
    async fn clear(&self) -> Result<(), StorageErr>;
}

#[derive(Error, Debug)]
#[error("error while accessing the limits storage: {msg}")]
pub struct StorageErr {
    msg: String,
    transient: bool,
}

impl StorageErr {
    pub fn msg(&self) -> &str {
        &self.msg
    }

    pub fn is_transient(&self) -> bool {
        self.transient
    }
}
