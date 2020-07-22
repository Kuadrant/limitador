#[cfg(feature = "redis_storage")]
use ::redis::RedisError;

use crate::counter::Counter;
use crate::limit::Limit;
use std::collections::HashSet;
use std::time::Duration;
use thiserror::Error;

pub mod in_memory;
pub mod wasm;

#[cfg(feature = "redis_storage")]
pub mod redis;

pub trait Storage: Sync + Send {
    fn add_limit(&mut self, limit: Limit) -> Result<(), StorageErr>;
    fn get_limits(&self, namespace: &str) -> Result<HashSet<Limit>, StorageErr>;
    fn delete_limit(&mut self, limit: &Limit) -> Result<(), StorageErr>;
    fn delete_limits(&mut self, namespace: &str) -> Result<(), StorageErr>;
    fn is_within_limits(&self, counter: &Counter, delta: i64) -> Result<bool, StorageErr>;
    fn update_counter(&mut self, counter: &Counter, delta: i64) -> Result<(), StorageErr>;
    fn get_counters(
        &mut self,
        namespace: &str,
    ) -> Result<Vec<(Counter, i64, Duration)>, StorageErr>;
}

#[derive(Error, Debug)]
#[error("error while accessing the limits storage: {msg}")]
pub struct StorageErr {
    msg: String,
}

impl StorageErr {
    pub fn msg(&self) -> &str {
        &self.msg
    }
}

#[cfg(feature = "redis_storage")]
impl From<RedisError> for StorageErr {
    fn from(e: RedisError) -> Self {
        StorageErr { msg: e.to_string() }
    }
}
