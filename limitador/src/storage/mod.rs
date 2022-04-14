use crate::counter::Counter;
use crate::limit::{Limit, Namespace};
use async_trait::async_trait;
use std::collections::HashSet;
use thiserror::Error;

pub mod in_memory;
pub mod wasm;

#[cfg(feature = "redis_storage")]
pub mod redis;

#[cfg(feature = "infinispan_storage")]
pub mod infinispan;

#[cfg(any(feature = "redis_storage", feature = "infinispan_storage"))]
mod keys;

pub enum Authorization<'c> {
    Ok,
    Limited(&'c Counter), // First counter found over the limits
}

pub trait Storage: Sync + Send {
    fn get_namespaces(&self) -> Result<HashSet<Namespace>, StorageErr>;
    fn add_limit(&self, limit: &Limit) -> Result<(), StorageErr>;
    fn get_limits(&self, namespace: &Namespace) -> Result<HashSet<Limit>, StorageErr>;
    fn delete_limit(&self, limit: &Limit) -> Result<(), StorageErr>;
    fn delete_limits(&self, namespace: &Namespace) -> Result<(), StorageErr>;
    fn is_within_limits(&self, counter: &Counter, delta: i64) -> Result<bool, StorageErr>;
    fn update_counter(&self, counter: &Counter, delta: i64) -> Result<(), StorageErr>;
    fn check_and_update<'c>(
        &self,
        counters: &HashSet<&'c Counter>,
        delta: i64,
    ) -> Result<Authorization<'c>, StorageErr>;
    fn get_counters(&self, namespace: &Namespace) -> Result<HashSet<Counter>, StorageErr>;
    fn clear(&self) -> Result<(), StorageErr>;
}

#[async_trait]
pub trait AsyncStorage: Sync + Send {
    async fn get_namespaces(&self) -> Result<HashSet<Namespace>, StorageErr>;
    async fn add_limit(&self, limit: &Limit) -> Result<(), StorageErr>;
    async fn get_limits(&self, namespace: &Namespace) -> Result<HashSet<Limit>, StorageErr>;
    async fn delete_limit(&self, limit: &Limit) -> Result<(), StorageErr>;
    async fn delete_limits(&self, namespace: &Namespace) -> Result<(), StorageErr>;
    async fn is_within_limits(&self, counter: &Counter, delta: i64) -> Result<bool, StorageErr>;
    async fn update_counter(&self, counter: &Counter, delta: i64) -> Result<(), StorageErr>;
    async fn check_and_update<'c>(
        &self,
        counters: &HashSet<&'c Counter>,
        delta: i64,
    ) -> Result<Authorization<'c>, StorageErr>;
    async fn get_counters(&self, namespace: &Namespace) -> Result<HashSet<Counter>, StorageErr>;
    async fn clear(&self) -> Result<(), StorageErr>;
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
