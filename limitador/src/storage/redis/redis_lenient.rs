use std::collections::HashSet;
use std::sync::atomic::AtomicBool;
use async_trait::async_trait;
use crate::counter::Counter;
use crate::limit::Limit;
use crate::prometheus_metrics::CounterAccess;
use crate::storage::{AsyncCounterStorage, Authorization, CounterStorage, StorageErr};
use crate::storage::in_memory::InMemoryStorage;
use crate::storage::redis::AsyncRedisStorage;

struct RedisLenient {
    redis: AsyncRedisStorage,
    fallback: InMemoryStorage,
    partitioned: AtomicBool,
}

#[async_trait]
impl AsyncCounterStorage for RedisLenient {
    async fn is_within_limits(&self, counter: &Counter, delta: i64) -> Result<bool, StorageErr> {
        self.redis.is_within_limits(counter, delta).await.or_else(|err| {
            if err.is_transient() {
                self.fallback.is_within_limits(counter, delta)
            } else {
                Err(err)
            }
        })
    }

    async fn update_counter(&self, counter: &Counter, delta: i64) -> Result<(), StorageErr> {
        self.redis.update_counter(counter, delta).await.or_else(|err| {
            if err.is_transient() {
                self.fallback.update_counter(counter, delta)
            } else {
                Err(err)
            }
        })
    }

    async fn check_and_update<'a>(&self, counters: &mut Vec<Counter>, delta: i64, load_counters: bool, counter_access: CounterAccess<'a>) -> Result<Authorization, StorageErr> {
        self.redis.check_and_update(counters, delta, load_counters, counter_access).await.or_else(|err| {
            if err.is_transient() {
                self.fallback.check_and_update(counters, delta, load_counters)
            } else {
                Err(err)
            }
        })
    }

    async fn get_counters(&self, limits: HashSet<Limit>) -> Result<HashSet<Counter>, StorageErr> {
        self.redis.get_counters(limits.clone()).await.or_else(|err| {
            if err.is_transient() {
                self.fallback.get_counters(&limits)
            } else {
                Err(err)
            }
        })
    }

    async fn delete_counters(&self, limits: HashSet<Limit>) -> Result<(), StorageErr> {
        self.redis.delete_counters(limits.clone()).await.or_else(|err| {
            if err.is_transient() {
                self.fallback.delete_counters(limits)
            } else {
                Err(err)
            }
        })
    }

    async fn clear(&self) -> Result<(), StorageErr> {
        self.fallback.clear().expect("This can't fail");
        self.redis.clear().await
    }
}
