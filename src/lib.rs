use crate::counter::Counter;
use crate::errors::LimitadorError;
use crate::limit::Limit;
use crate::storage::in_memory::InMemoryStorage;
use crate::storage::Storage;
use std::collections::{HashMap, HashSet};

pub mod counter;
pub mod errors;
pub mod limit;
pub mod storage;

pub struct RateLimiter {
    storage: Box<dyn Storage>,
}

impl RateLimiter {
    pub fn new() -> RateLimiter {
        RateLimiter {
            storage: Box::new(InMemoryStorage::default()),
        }
    }

    pub fn new_with_storage(storage: Box<dyn Storage>) -> RateLimiter {
        RateLimiter { storage }
    }

    pub fn add_limit(&mut self, limit: Limit) -> Result<(), LimitadorError> {
        self.storage.add_limit(limit).map_err(|err| err.into())
    }

    pub fn delete_limit(&mut self, limit: &Limit) -> Result<(), LimitadorError> {
        self.storage.delete_limit(limit).map_err(|err| err.into())
    }

    pub fn get_limits(&self, namespace: &str) -> Result<HashSet<Limit>, LimitadorError> {
        self.storage.get_limits(namespace).map_err(|err| err.into())
    }

    pub fn delete_limits(&mut self, namespace: &str) -> Result<(), LimitadorError> {
        self.storage
            .delete_limits(namespace)
            .map_err(|err| err.into())
    }

    pub fn is_rate_limited(
        &self,
        namespace: &str,
        values: &HashMap<String, String>,
        delta: i64,
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
        &mut self,
        namespace: &str,
        values: &HashMap<String, String>,
        delta: i64,
    ) -> Result<(), LimitadorError> {
        let counters = self.counters_that_apply(namespace, values)?;

        counters
            .iter()
            .try_for_each(|counter| self.storage.update_counter(&counter, delta))
            .map_err(|err| err.into())
    }

    pub fn check_rate_limited_and_update(
        &mut self,
        namespace: &str,
        values: &HashMap<String, String>,
        delta: i64,
    ) -> Result<bool, LimitadorError> {
        match self.is_rate_limited(namespace, values, delta) {
            Ok(rate_limited) => {
                if rate_limited {
                    Ok(true)
                } else {
                    match self.update_counters(namespace, values, delta) {
                        Ok(_) => Ok(false),
                        Err(e) => Err(e),
                    }
                }
            }
            Err(e) => Err(e),
        }
    }

    pub fn get_counters(&mut self, namespace: &str) -> Result<HashSet<Counter>, LimitadorError> {
        self.storage
            .get_counters(namespace)
            .map_err(|err| err.into())
    }

    fn counters_that_apply(
        &self,
        namespace: &str,
        values: &HashMap<String, String>,
    ) -> Result<Vec<Counter>, LimitadorError> {
        let limits = self.get_limits(namespace)?;

        let counters = limits
            .iter()
            .filter(|lim| lim.applies(values))
            .map(|lim| Counter::new(lim.clone(), values.clone()))
            .collect();

        Ok(counters)
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}
