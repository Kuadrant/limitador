//! Limitador is a generic rate-limiter.
//!
//! # Basic operation
//!
//! Limitador can store the counters in memory or in Redis. Storing them in memory
//! is faster, but the counters cannot be shared between several instances of
//! Limitador. Storing the limits in Redis is slower, but they can be shared
//! between instances.
//!
//! By default, the rate limiter is configured to store the counters in memory.
//! It'll store only a limited amount of "qualified counters", specified as a
//! `u64` value in the constructor.
//! ```
//! use limitador::RateLimiter;
//! let rate_limiter = RateLimiter::new(1000);
//! ```
//!
//! To use Redis:
//! ```no_run
//! #[cfg(feature = "redis_storage")]
//! # {
//! use limitador::RateLimiter;
//! use limitador::storage::redis::RedisStorage;
//!
//! // Default redis URL (redis://localhost:6379).
//! let rate_limiter = RateLimiter::new_with_storage(Box::new(RedisStorage::default()));
//!
//! // Custom redis URL
//! let rate_limiter = RateLimiter::new_with_storage(
//!     Box::new(RedisStorage::new("redis://127.0.0.1:7777").unwrap())
//! );
//! # }
//! ```
//!
//! # Limits
//!
//! The definition of a limit includes:
//! - A namespace that identifies the resource to limit. It could be an API, a
//! Kubernetes service, a proxy ID, etc.
//! - A value.
//! - The length of the period in seconds.
//! - Conditions that define when to apply the limit.
//! - A set of variables. For example, if we need to define the same limit for
//! each "user_id", instead of creating a limit for each hardcoded ID, we just
//! need to define "user_id" as a variable.
//!
//! If we used Limitador in a context where it receives an HTTP request we could
//! define a limit like this to allow 10 requests per minute and per user_id
//! when the HTTP method is "GET".
//!
//! ```
//! use limitador::limit::Limit;
//! let limit = Limit::new(
//!     "my_namespace",
//!      10,
//!      60,
//!      vec!["req.method == 'GET'"],
//!      vec!["user_id"],
//! );
//! ```
//!
//! Notice that the keys and variables are generic, so they do not necessarily
//! have to refer to an HTTP request.
//!
//! # Manage limits
//!
//! ```
//! use limitador::RateLimiter;
//! use limitador::limit::Limit;
//! let limit = Limit::new(
//!     "my_namespace",
//!      10,
//!      60,
//!      vec!["req.method == 'GET'"],
//!      vec!["user_id"],
//! );
//! let mut rate_limiter = RateLimiter::new(1000);
//!
//! // Add a limit
//! rate_limiter.add_limit(limit.clone());
//!
//! // Delete the limit
//! rate_limiter.delete_limit(&limit);
//!
//! // Get all the limits in a namespace
//! let namespace = "my_namespace".into();
//! rate_limiter.get_limits(&namespace);
//!
//! // Delete all the limits in a namespace
//! rate_limiter.delete_limits(&namespace);
//! ```
//!
//! # Apply limits
//!
//! ```
//! use limitador::RateLimiter;
//! use limitador::limit::Limit;
//! use std::collections::HashMap;
//!
//! let mut rate_limiter = RateLimiter::new(1000);
//!
//! let limit = Limit::new(
//!     "my_namespace",
//!      2,
//!      60,
//!      vec!["req.method == 'GET'"],
//!      vec!["user_id"],
//! );
//! rate_limiter.add_limit(limit);
//!
//! // We've defined a limit of 2. So we can report 2 times before being
//! // rate-limited
//! let mut values_to_report: HashMap<String, String> = HashMap::new();
//! values_to_report.insert("req.method".to_string(), "GET".to_string());
//! values_to_report.insert("user_id".to_string(), "1".to_string());
//!
//! // Check if we can report
//! let namespace = "my_namespace".into();
//! assert!(!rate_limiter.is_rate_limited(&namespace, &values_to_report, 1).unwrap());
//!
//! // Report
//! rate_limiter.update_counters(&namespace, &values_to_report, 1).unwrap();
//!
//! // Check and report again
//! assert!(!rate_limiter.is_rate_limited(&namespace, &values_to_report, 1).unwrap());
//! rate_limiter.update_counters(&namespace, &values_to_report, 1).unwrap();
//!
//! // We've already reported 2, so reporting another one should not be allowed
//! assert!(rate_limiter.is_rate_limited(&namespace, &values_to_report, 1).unwrap());
//!
//! // You can also check and report if not limited in a single call. It's useful
//! // for example, when calling Limitador from a proxy. Instead of doing 2
//! // separate calls, we can issue just one:
//! rate_limiter.check_rate_limited_and_update(&namespace, &values_to_report, 1, false).unwrap();
//! ```
//!
//! # Async
//!
//! There are two Redis drivers, a blocking one and an async one. To use the
//! async one, we need to instantiate an "AsyncRateLimiter" with an
//! "AsyncRedisStorage":
//!
//! ```
//! #[cfg(feature = "redis_storage")]
//! # {
//! use limitador::AsyncRateLimiter;
//! use limitador::storage::redis::AsyncRedisStorage;
//!
//! async {
//!     let rate_limiter = AsyncRateLimiter::new_with_storage(
//!         Box::new(AsyncRedisStorage::new("redis://127.0.0.1:7777").await.unwrap())
//!     );
//! };
//! # }
//! ```
//!
//! Both the blocking and the async limiters expose the same functions, so we
//! can use the async limiter as explained above. For example:
//!
//! ```
//! #[cfg(feature = "redis_storage")]
//! # {
//! use limitador::AsyncRateLimiter;
//! use limitador::limit::Limit;
//! use limitador::storage::redis::AsyncRedisStorage;
//! let limit = Limit::new(
//!      "my_namespace",
//!      10,
//!      60,
//!      vec!["req.method == 'GET'"],
//!      vec!["user_id"],
//! );
//!
//! async {
//!     let rate_limiter = AsyncRateLimiter::new_with_storage(
//!         Box::new(AsyncRedisStorage::new("redis://127.0.0.1:7777").await.unwrap())
//!     );
//!     rate_limiter.add_limit(limit);
//! };
//! # }
//! ```
//!
//! # Limits accuracy
//!
//! When storing the counters in memory, Limitador guarantees that we'll never go
//! over the limits defined. However, when using Redis that's not the case. The
//! Redis driver sacrifices a bit of accuracy when applying the limits to be
//! more performant.
//!

#![deny(clippy::all, clippy::cargo)]
// TODO this needs review to reduce the bloat pulled in by dependencies
#![allow(clippy::multiple_crate_versions)]

use std::collections::{HashMap, HashSet};

use crate::counter::Counter;
use crate::errors::LimitadorError;
use crate::limit::{Limit, Namespace};
use crate::storage::in_memory::InMemoryStorage;
use crate::storage::{AsyncCounterStorage, AsyncStorage, Authorization, CounterStorage, Storage};

#[macro_use]
extern crate core;

pub mod counter;
pub mod errors;
pub mod limit;
pub mod storage;

pub struct RateLimiter {
    storage: Storage,
}

pub struct AsyncRateLimiter {
    storage: AsyncStorage,
}

pub struct RateLimiterBuilder {
    storage: Storage,
}

pub struct CheckResult {
    pub limited: bool,
    pub counters: Vec<Counter>,
    pub limit_name: Option<String>,
}

impl From<CheckResult> for bool {
    fn from(value: CheckResult) -> Self {
        value.limited
    }
}

impl RateLimiterBuilder {
    pub fn with_storage(storage: Storage) -> Self {
        Self { storage }
    }

    pub fn new(cache_size: u64) -> Self {
        Self {
            storage: Storage::new(cache_size),
        }
    }

    pub fn storage(mut self, storage: Storage) -> Self {
        self.storage = storage;
        self
    }

    pub fn build(self) -> RateLimiter {
        RateLimiter {
            storage: self.storage,
        }
    }
}

pub struct AsyncRateLimiterBuilder {
    storage: AsyncStorage,
}

impl AsyncRateLimiterBuilder {
    pub fn new(storage: AsyncStorage) -> Self {
        Self { storage }
    }

    pub fn build(self) -> AsyncRateLimiter {
        AsyncRateLimiter {
            storage: self.storage,
        }
    }
}

impl RateLimiter {
    pub fn new(cache_size: u64) -> Self {
        Self {
            storage: Storage::new(cache_size),
        }
    }

    pub fn new_with_storage(counters: Box<dyn CounterStorage>) -> Self {
        Self {
            storage: Storage::with_counter_storage(counters),
        }
    }

    pub fn get_namespaces(&self) -> HashSet<Namespace> {
        self.storage.get_namespaces()
    }

    pub fn add_limit(&self, limit: Limit) -> bool {
        self.storage.add_limit(limit)
    }

    pub fn delete_limit(&self, limit: &Limit) -> Result<(), LimitadorError> {
        self.storage.delete_limit(limit)?;
        Ok(())
    }

    pub fn get_limits(&self, namespace: &Namespace) -> HashSet<Limit> {
        self.storage.get_limits(namespace)
    }

    pub fn delete_limits(&self, namespace: &Namespace) -> Result<(), LimitadorError> {
        self.storage.delete_limits(namespace)?;
        Ok(())
    }

    pub fn is_rate_limited(
        &self,
        namespace: &Namespace,
        values: &HashMap<String, String>,
        delta: u64,
    ) -> Result<bool, LimitadorError> {
        let counters = self.counters_that_apply(namespace, values)?;

        for counter in counters {
            match self.storage.is_within_limits(&counter, delta) {
                Ok(within_limits) => {
                    if !within_limits {
                        return Ok(true);
                    }
                }
                Err(e) => return Err(e.into()),
            }
        }

        Ok(false)
    }

    pub fn update_counters(
        &self,
        namespace: &Namespace,
        values: &HashMap<String, String>,
        delta: u64,
    ) -> Result<(), LimitadorError> {
        let counters = self.counters_that_apply(namespace, values)?;

        counters
            .iter()
            .try_for_each(|counter| self.storage.update_counter(counter, delta))
            .map_err(|err| err.into())
    }

    pub fn check_rate_limited_and_update(
        &self,
        namespace: &Namespace,
        values: &HashMap<String, String>,
        delta: u64,
        load_counters: bool,
    ) -> Result<CheckResult, LimitadorError> {
        let mut counters = self.counters_that_apply(namespace, values)?;

        if counters.is_empty() {
            return Ok(CheckResult {
                limited: false,
                counters,
                limit_name: None,
            });
        }

        let check_result = self
            .storage
            .check_and_update(&mut counters, delta, load_counters)?;

        let counters = if load_counters {
            counters
        } else {
            Vec::default()
        };

        match check_result {
            Authorization::Ok => Ok(CheckResult {
                limited: false,
                counters,
                limit_name: None,
            }),
            Authorization::Limited(name) => Ok(CheckResult {
                limited: true,
                counters,
                limit_name: name,
            }),
        }
    }

    pub fn get_counters(&self, namespace: &Namespace) -> Result<HashSet<Counter>, LimitadorError> {
        self.storage
            .get_counters(namespace)
            .map_err(|err| err.into())
    }

    // Deletes all the limits stored except the ones received in the params. For
    // every limit received, if it does not exist, it is created. If it already
    // exists, its associated counters are not reset.
    pub fn configure_with(
        &self,
        limits: impl IntoIterator<Item = Limit>,
    ) -> Result<(), LimitadorError> {
        let limits_to_keep_or_create = classify_limits_by_namespace(limits);

        let namespaces_limits_to_keep_or_create: HashSet<Namespace> =
            limits_to_keep_or_create.keys().cloned().collect();

        for namespace in self
            .get_namespaces()
            .union(&namespaces_limits_to_keep_or_create)
        {
            let limits_in_namespace = self.get_limits(namespace);
            let limits_to_keep_in_ns: HashSet<Limit> = limits_to_keep_or_create
                .get(namespace)
                .cloned()
                .unwrap_or_default();

            for limit in limits_in_namespace.difference(&limits_to_keep_in_ns) {
                self.delete_limit(limit)?;
            }

            for limit in limits_to_keep_in_ns.difference(&limits_in_namespace) {
                self.add_limit(limit.clone());
            }

            for limit in limits_to_keep_in_ns.union(&limits_in_namespace) {
                self.storage.update_limit(limit);
            }
        }

        Ok(())
    }

    fn counters_that_apply(
        &self,
        namespace: &Namespace,
        values: &HashMap<String, String>,
    ) -> Result<Vec<Counter>, LimitadorError> {
        let limits = self.get_limits(namespace);

        let counters = limits
            .iter()
            .filter(|lim| lim.applies(values))
            .map(|lim| Counter::new(lim.clone(), values.clone()))
            .collect();

        Ok(counters)
    }
}

// TODO: the code of this implementation is almost identical to the blocking
// one. The only exception is that the functions defined are "async" and all the
// calls to the storage need to include ".await". We'll need to think about how
// to remove this duplication.

impl AsyncRateLimiter {
    pub fn new_with_storage(storage: Box<dyn AsyncCounterStorage>) -> Self {
        Self {
            storage: AsyncStorage::with_counter_storage(storage),
        }
    }

    pub fn get_namespaces(&self) -> HashSet<Namespace> {
        self.storage.get_namespaces()
    }

    pub fn add_limit(&self, limit: Limit) -> bool {
        self.storage.add_limit(limit)
    }

    pub async fn delete_limit(&self, limit: &Limit) -> Result<(), LimitadorError> {
        self.storage.delete_limit(limit).await?;
        Ok(())
    }

    pub fn get_limits(&self, namespace: &Namespace) -> HashSet<Limit> {
        self.storage.get_limits(namespace)
    }

    pub async fn delete_limits(&self, namespace: &Namespace) -> Result<(), LimitadorError> {
        self.storage.delete_limits(namespace).await?;
        Ok(())
    }

    pub async fn is_rate_limited(
        &self,
        namespace: &Namespace,
        values: &HashMap<String, String>,
        delta: u64,
    ) -> Result<bool, LimitadorError> {
        let counters = self.counters_that_apply(namespace, values).await?;

        for counter in counters {
            match self.storage.is_within_limits(&counter, delta).await {
                Ok(within_limits) => {
                    if !within_limits {
                        return Ok(true);
                    }
                }
                Err(e) => return Err(e.into()),
            }
        }
        Ok(false)
    }

    pub async fn update_counters(
        &self,
        namespace: &Namespace,
        values: &HashMap<String, String>,
        delta: u64,
    ) -> Result<(), LimitadorError> {
        let counters = self.counters_that_apply(namespace, values).await?;

        for counter in counters {
            self.storage.update_counter(&counter, delta).await?
        }

        Ok(())
    }

    pub async fn check_rate_limited_and_update(
        &self,
        namespace: &Namespace,
        values: &HashMap<String, String>,
        delta: u64,
        load_counters: bool,
    ) -> Result<CheckResult, LimitadorError> {
        // the above where-clause is needed in order to call unwrap().
        let mut counters = self.counters_that_apply(namespace, values).await?;

        if counters.is_empty() {
            return Ok(CheckResult {
                limited: false,
                counters,
                limit_name: None,
            });
        }

        let check_result = self
            .storage
            .check_and_update(&mut counters, delta, load_counters)
            .await?;

        let counters = if load_counters {
            counters
        } else {
            Vec::default()
        };

        match check_result {
            Authorization::Ok => Ok(CheckResult {
                limited: false,
                counters,
                limit_name: None,
            }),
            Authorization::Limited(name) => Ok(CheckResult {
                limited: true,
                counters,
                limit_name: name,
            }),
        }
    }

    pub async fn get_counters(
        &self,
        namespace: &Namespace,
    ) -> Result<HashSet<Counter>, LimitadorError> {
        self.storage
            .get_counters(namespace)
            .await
            .map_err(|err| err.into())
    }

    // Deletes all the limits stored except the ones received in the params. For
    // every limit received, if it does not exist, it is created. If it already
    // exists, its associated counters are not reset.
    pub async fn configure_with(
        &self,
        limits: impl IntoIterator<Item = Limit>,
    ) -> Result<(), LimitadorError> {
        let limits_to_keep_or_create = classify_limits_by_namespace(limits);

        let namespaces_limits_to_keep_or_create: HashSet<Namespace> =
            limits_to_keep_or_create.keys().cloned().collect();

        for namespace in self
            .get_namespaces()
            .union(&namespaces_limits_to_keep_or_create)
        {
            let limits_in_namespace = self.get_limits(namespace);
            let limits_to_keep_in_ns: HashSet<Limit> = limits_to_keep_or_create
                .get(namespace)
                .cloned()
                .unwrap_or_default();

            for limit in limits_in_namespace.difference(&limits_to_keep_in_ns) {
                self.delete_limit(limit).await?;
            }

            for limit in limits_to_keep_in_ns.difference(&limits_in_namespace) {
                self.add_limit(limit.clone());
            }

            for limit in limits_to_keep_in_ns.union(&limits_in_namespace) {
                self.storage.update_limit(limit);
            }
        }

        Ok(())
    }

    async fn counters_that_apply(
        &self,
        namespace: &Namespace,
        values: &HashMap<String, String>,
    ) -> Result<Vec<Counter>, LimitadorError> {
        let limits = self.get_limits(namespace);

        let counters = limits
            .iter()
            .filter(|lim| lim.applies(values))
            .map(|lim| Counter::new(lim.clone(), values.clone()))
            .collect();

        Ok(counters)
    }
}

fn classify_limits_by_namespace(
    limits: impl IntoIterator<Item = Limit>,
) -> HashMap<Namespace, HashSet<Limit>> {
    let mut res: HashMap<Namespace, HashSet<Limit>> = HashMap::new();

    for limit in limits {
        match res.get_mut(limit.namespace()) {
            Some(limits) => {
                limits.insert(limit);
            }
            None => {
                let mut set = HashSet::new();
                set.insert(limit.clone());
                res.insert(limit.namespace().clone(), set);
            }
        }
    }

    res
}
