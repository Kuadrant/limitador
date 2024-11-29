use std::cmp::Ordering;
use crate::counter::Counter;
use crate::limit::{Limit, Namespace};
use crate::{limit, InMemoryStorage};
use async_trait::async_trait;
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use metrics::counter;
use prost::bytes::BufMut;

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

    pub fn is_within_limits(&self, key: &CounterKey, delta: u64) -> Result<bool, StorageErr> {
        self.counters.is_within_limits(key, delta)
    }

    pub fn update_counter(&self, key: &CounterKey, delta: u64) -> Result<(), StorageErr> {
        self.counters.update_counter(key, delta)
    }

    pub fn check_and_update(
        &self,
        counters: &[CounterKey],
        delta: u64,
        load_counters: bool,
    ) -> Result<Authorization, StorageErr> {
        if load_counters {
            let (auth, counters) = self.counters.check_and_update_loading(counters, delta)?;
            // todo deal with these counters!
            Ok(auth)
        } else {
            self.counters.check_and_update(counters.iter().map(|c| c.limit().into()).collect(), delta)
        }
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
        load_counters: bool,
    ) -> Result<Authorization, StorageErr> {
        self.counters
            .check_and_update(counters, delta, load_counters)
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

pub enum CounterKey {
    Identified((Namespace, String)),
    Simple((Namespace, SimpleCounterKey)),
    Qualified((Namespace, SimpleCounterKey, Vec<String>)),
}

impl CounterKey {
    fn namespace(&self) -> &Namespace {
        match self {
            CounterKey::Identified((ns, _)) => ns,
            CounterKey::Simple((ns, _)) => ns,
            CounterKey::Qualified((ns, _, _)) => ns,
        }
    }
}

impl Eq for CounterKey {}

impl PartialEq<Self> for CounterKey {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl PartialOrd<Self> for CounterKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CounterKey {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.namespace().cmp(other.namespace()) {
            Ordering::Equal => {
                match self {
                    CounterKey::Identified((_, id)) => {
                        match other {
                            CounterKey::Identified((_, other)) => id.cmp(other),
                            _ => Ordering::Less,
                        }
                    }
                    CounterKey::Simple((_, key)) => {
                        match other {
                            CounterKey::Identified(_) => Ordering::Greater,
                            CounterKey::Simple((_, other)) => key.cmp(other),
                            CounterKey::Qualified(_) => Ordering::Less,
                        }
                    }
                    CounterKey::Qualified((_, key, qualifiers)) => {
                        match other {
                            CounterKey::Identified(_) => Ordering::Greater,
                            CounterKey::Simple(_) => Ordering::Greater,
                            CounterKey::Qualified((_, other_key, other_qualifiers)) => {
                                match key.cmp(other_key) {
                                    Ordering::Equal => qualifiers.cmp(other_qualifiers),
                                    other => other,
                                }
                            }
                        }
                    }
                }
            },
            other => other,
        }
    }
}

#[derive(Eq, PartialEq, Ord, PartialOrd, Debug)]
pub struct SimpleCounterKey {
    seconds: u64,
    conditions: Vec<String>,
}

impl From<&Limit> for CounterKey {
    fn from(limit: &Limit) -> Self {
        let mut conditions = Vec::from(limit.conditions());
        conditions.sort();
        let mut variables = Vec::from(limit.variables());
        variables.sort();

        Self {
            namespace: limit.namespace().clone(),
            seconds: limit.seconds(),
            conditions,
            variables,
        }
    }
}

pub trait CounterStorage: Sync + Send {
    fn add_counters(&self, keys: &[CounterKey]) -> Result<(), StorageErr>;
    fn get_counters(&self, keys: &[CounterKey]) -> Result<Vec<(u64, Duration)>, StorageErr>;
    fn update_counters(&self, key: &[CounterKey], delta: u64) -> Result<u64, StorageErr>;
    fn delete_counters(&self, keys: &[CounterKey]) -> Result<(), StorageErr>;

    fn check_and_update(
        &self,
        counters: &[CounterKey],
        delta: u64,
    ) -> Result<Authorization, StorageErr>;
    fn check_and_update_loading(
        &self,
        counters: &[CounterKey],
        delta: u64,
    ) -> Result<(Authorization, Vec<Counter>), StorageErr>;
    fn clear(&self) -> Result<(), StorageErr>;
}

#[async_trait]
pub trait AsyncCounterStorage: Sync + Send {
    async fn is_within_limits(&self, counter: &Counter, delta: u64) -> Result<bool, StorageErr>;
    async fn update_counter(&self, counter: &Counter, delta: u64) -> Result<(), StorageErr>;
    async fn check_and_update<'a>(
        &self,
        counters: &mut Vec<Counter>,
        delta: u64,
        load_counters: bool,
    ) -> Result<Authorization, StorageErr>;
    async fn get_counters(
        &self,
        limits: &HashSet<Arc<Limit>>,
    ) -> Result<HashSet<Counter>, StorageErr>;
    async fn delete_counters(&self, limits: &HashSet<Arc<Limit>>) -> Result<(), StorageErr>;
    async fn clear(&self) -> Result<(), StorageErr>;
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
