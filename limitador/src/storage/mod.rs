use crate::counter::Counter;
use crate::limit::{Limit, Namespace};
use crate::InMemoryStorage;
use async_trait::async_trait;
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::sync::{Arc, RwLock};


#[cfg(feature = "disk_storage")]
pub mod disk;
#[cfg(feature = "distributed_storage")]
pub mod distributed;
pub mod in_memory;

#[cfg(feature = "distributed_storage")]
pub use crate::storage::distributed::CrInMemoryStorage as DistributedInMemoryStorage;

#[cfg(feature = "redis_storage")]
pub mod redis;

mod atomic_expiring_value;
#[cfg(any(feature = "disk_storage", feature = "redis_storage"))]
mod keys;

pub enum Authorization {
    Ok,
    Limited(Option<String>), // First counter found over the limits
}

pub struct Storage {
    limits: RwLock<HashMap<Namespace, HashSet<Arc<Limit>>>>,
    counters: Box<dyn CounterStorage>,
}

pub struct AsyncStorage {
    limits: RwLock<HashMap<Namespace, HashSet<Arc<Limit>>>>,
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
        limits.entry(namespace).or_default().insert(Arc::new(limit))
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
                limits.insert(Arc::new(update.clone()));
                return true;
            }
        }
        false
    }

    pub fn get_limits(&self, namespace: &Namespace) -> HashSet<Arc<Limit>> {
        match self.limits.read().unwrap().get(namespace) {
            // todo revise typing here?
            Some(limits) => limits.iter().map(Arc::clone).collect(),
            None => HashSet::new(),
        }
    }

    pub fn delete_limit(&self, limit: &Limit) -> Result<(), StorageErr> {
        let arc = match self.limits.read().unwrap().get(limit.namespace()) {
            None => Arc::new(limit.clone()),
            Some(limits) => limits
                .iter()
                .find(|l| ***l == *limit)
                .cloned()
                .unwrap_or_else(|| Arc::new(limit.clone())),
        };
        let mut limits = HashSet::new();
        limits.insert(arc);
        self.counters.delete_counters(&limits)?;

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
            self.counters.delete_counters(&data)?;
        }
        Ok(())
    }

    pub fn is_within_limits(&self, counter: &Counter, delta: u64) -> Result<bool, StorageErr> {
        self.counters.is_within_limits(counter, delta)
    }

    pub fn update_counter(&self, counter: &Counter, delta: u64) -> Result<(), StorageErr> {
        self.counters.update_counter(counter, delta)
    }

    pub fn check_and_update(
        &self,
        counters: &mut Vec<Counter>,
        delta: u64,
        check: bool,
        update: bool,
        load_counters: bool,
    ) -> Result<Authorization, StorageErr> {
        self.counters
            .check_and_update(counters, delta, check, update, load_counters)
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
            Some(limits) => limits.insert(Arc::new(limit)),
            None => {
                let mut limits = HashSet::new();
                limits.insert(Arc::new(limit));
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
                limits.insert(Arc::new(update.clone()));
                return true;
            }
        }
        false
    }

    pub fn get_limits(&self, namespace: &Namespace) -> HashSet<Arc<Limit>> {
        match self.limits.read().unwrap().get(namespace) {
            Some(limits) => limits.iter().map(Arc::clone).collect(),
            None => HashSet::new(),
        }
    }

    pub async fn delete_limit(&self, limit: &Limit) -> Result<(), StorageErr> {
        let arc = match self.limits.read().unwrap().get(limit.namespace()) {
            None => Arc::new(limit.clone()),
            Some(limits) => limits
                .iter()
                .find(|l| ***l == *limit)
                .cloned()
                .unwrap_or_else(|| Arc::new(limit.clone())),
        };
        let mut limits = HashSet::new();
        limits.insert(arc);
        self.counters.delete_counters(&limits).await?;

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
            self.counters.delete_counters(&data).await?;
        }
        Ok(())
    }

    pub async fn is_within_limits(
        &self,
        counter: &Counter,
        delta: u64,
    ) -> Result<bool, StorageErr> {
        self.counters.is_within_limits(counter, delta).await
    }

    pub async fn update_counter(&self, counter: &Counter, delta: u64) -> Result<(), StorageErr> {
        self.counters.update_counter(counter, delta).await
    }

    pub async fn check_and_update(
        &self,
        counters: &mut Vec<Counter>,
        delta: u64,
        check: bool, 
        update: bool,
        load_counters: bool,
    ) -> Result<Authorization, StorageErr> {
        self.counters
            .check_and_update(counters, delta, check, update, load_counters)
            .await
    }

    pub async fn get_counters(
        &self,
        namespace: &Namespace,
    ) -> Result<HashSet<Counter>, StorageErr> {
        let limits = self.get_limits(namespace);
        self.counters.get_counters(&limits).await
    }

    pub async fn clear(&self) -> Result<(), StorageErr> {
        self.limits.write().unwrap().clear();
        self.counters.clear().await
    }
}

pub trait CounterStorage: Sync + Send {
    fn add_counter(&self, limit: &Limit) -> Result<(), StorageErr>;
    fn get_counters(&self, limits: &HashSet<Arc<Limit>>) -> Result<HashSet<Counter>, StorageErr>; // todo revise typing here?
    fn delete_counters(&self, limits: &HashSet<Arc<Limit>>) -> Result<(), StorageErr>; // todo revise typing here?
    fn clear(&self) -> Result<(), StorageErr>;
    fn is_within_limits(
        &self, 
        counter: &Counter, 
        delta: u64, 
    ) -> Result<bool, StorageErr>;   
    fn update_counter(
        &self, 
        counter: &Counter, 
        delta: u64, 
    ) -> Result<(), StorageErr>;
    fn check_and_update(
        &self,
        counters: &mut Vec<Counter>,
        delta: u64,
        check: bool,
        update: bool,
        load_counters: bool,
    ) -> Result<Authorization, StorageErr>;
}

#[async_trait]
pub trait AsyncCounterStorage: Sync + Send {
    async fn get_counters(
        &self,
        limits: &HashSet<Arc<Limit>>,
    ) -> Result<HashSet<Counter>, StorageErr>;
    async fn delete_counters(&self, limits: &HashSet<Arc<Limit>>) -> Result<(), StorageErr>;
    async fn clear(&self) -> Result<(), StorageErr>;
    async fn is_within_limits(
        &self, 
        counter: &Counter, 
        delta: u64, 
    ) -> Result<bool, StorageErr>;
    async fn update_counter(
        &self, 
        counter: &Counter, 
        delta: u64,
    ) -> Result<(), StorageErr>;
    async fn check_and_update<'a>(
        &self,
        counters: &mut Vec<Counter>,
        delta: u64,
        check: bool,
        update: bool,
        load_counters: bool,
    ) -> Result<Authorization, StorageErr>;

}

#[derive(Debug)]
pub struct StorageErr {
    msg: String,
    source: Option<Box<dyn Error + 'static>>,
    transient: bool,
}

impl Display for StorageErr {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "counter storage error: {}", self.msg)
    }
}

impl Error for StorageErr {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source.as_ref().map(|source| source.as_ref())
    }
}

impl StorageErr {
    pub fn msg(&self) -> &str {
        &self.msg
    }

    pub fn is_transient(&self) -> bool {
        self.transient
    }
}
