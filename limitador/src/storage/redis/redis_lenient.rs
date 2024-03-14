use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
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
        if self.is_partitioned() {
            self.fallback.is_within_limits(counter, delta)
        } else {
            self.redis.is_within_limits(counter, delta).await.or_else(|err| {
                if err.is_transient() {
                    self.partitioned(true);
                    self.fallback.is_within_limits(counter, delta)
                } else {
                    Err(err)
                }
            })
        }
    }

    async fn update_counter(&self, counter: &Counter, delta: i64) -> Result<(), StorageErr> {
        if self.is_partitioned() {
            self.fallback.update_counter(counter, delta)
        } else {
            self.redis.update_counter(counter, delta).await.or_else(|err| {
                if err.is_transient() {
                    self.partitioned(true);
                    self.fallback.update_counter(counter, delta)
                } else {
                    Err(err)
                }
            })
        }
    }

    async fn check_and_update<'a>(&self, counters: &mut Vec<Counter>, delta: i64, load_counters: bool, counter_access: CounterAccess<'a>) -> Result<Authorization, StorageErr> {
        if self.is_partitioned() {
            self.fallback.check_and_update(counters, delta, load_counters)
        } else {
            self.redis.check_and_update(counters, delta, load_counters, counter_access).await.or_else(|err| {
                if err.is_transient() {
                    self.partitioned(true);
                    self.fallback.check_and_update(counters, delta, load_counters)
                } else {
                    Err(err)
                }
            })
        }
    }

    async fn get_counters(&self, limits: HashSet<Limit>) -> Result<HashSet<Counter>, StorageErr> {
        if self.is_partitioned() {
            self.fallback.get_counters(&limits)
        } else {
            self.redis.get_counters(limits.clone()).await.or_else(|err| {
                if err.is_transient() {
                    self.partitioned(true);
                    self.fallback.get_counters(&limits)
                } else {
                    Err(err)
                }
            })
        }
    }

    async fn delete_counters(&self, limits: HashSet<Limit>) -> Result<(), StorageErr> {
        if self.is_partitioned() {
            self.fallback.delete_counters(limits)
        } else {
            self.redis.delete_counters(limits.clone()).await.or_else(|err| {
                if err.is_transient() {
                    self.partitioned(true);
                    self.fallback.delete_counters(limits)
                } else {
                    Err(err)
                }
            })
        }
    }

    async fn clear(&self) -> Result<(), StorageErr> {
        self.fallback.clear().expect("This can't fail");
        self.redis.clear().await
    }
}

impl RedisLenient {
    fn is_partitioned(&self) -> bool {
        self.partitioned.load(Ordering::Acquire)
    }

    fn partitioned(&self, partition: bool) -> bool {
        self.partitioned.compare_exchange(!partition, partition, Ordering::Release, Ordering::Acquire).is_ok()
    }

    pub async fn sanity_check(self: Arc<Self>, interval: Duration) {
        let mut interval = tokio::time::interval(interval);
        tokio::spawn(async move {
            loop {
                interval.tick().await;
                if self.is_partitioned() {
                    let partitioned = !self.redis.is_alive().await;
                    self.partitioned(partitioned);
                }
            }
        });
    }
}
