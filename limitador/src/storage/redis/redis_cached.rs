use crate::counter::Counter;
use crate::limit::{Limit, Namespace};
use crate::storage::redis::batcher::Batcher;
use crate::storage::redis::counters_cache::{
    CountersCache, CountersCacheBuilder, DEFAULT_MAX_CACHED_COUNTERS,
    DEFAULT_MAX_TTL_CACHED_COUNTERS, DEFAULT_TTL_RATIO_CACHED_COUNTERS,
};
use crate::storage::redis::redis_async::AsyncRedisStorage;
use crate::storage::redis::redis_keys::*;
use crate::storage::{AsyncStorage, StorageErr};
use async_trait::async_trait;
use redis::aio::ConnectionManager;
use redis::ConnectionInfo;
use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use ttl_cache::TtlCache;

// This is just a first version.
//
// The idea is to improve throughput and latencies by caching the limits and
// counters in memory to reduce the number of accesses to Redis.
// For now, only the "check_and_update" function uses caching, the rest of
// functions simply delegate the work to another storage implementation.
//
// There might be several instances of Limitador accessing the same Redis server
// at the same time. This means that cached values might not reflect updates
// done by other Limitador instances. That's the trade-off this implementation
// makes. In order to reduce the number of Redis queries, it sacrifices some
// rate-limit accuracy. We can go over limits, but the amount can be configured
// by tuning the constants below.
//
// Future improvements:
// - We shouldn't just cache the remaining of the counter. We should take into
// account that there might be other instances running.
// - Introduce a mechanism to avoid going to Redis to fetch the same counter
// multiple times when it is not cached.

const DEFAULT_FLUSHING_PERIOD: Duration = Duration::from_secs(1);
const DEFAULT_TTL_CACHED_LIMITS: Duration = Duration::from_secs(10);
const DEFAULT_MAX_CACHED_NAMESPACES: usize = 1000;

pub struct CachedRedisStorage {
    cached_limits_by_namespace: Mutex<TtlCache<Namespace, HashSet<Limit>>>,
    ttl_cached_limits: Duration,
    cached_counters: Mutex<CountersCache>,
    batcher_counter_updates: Arc<Mutex<Batcher>>,
    async_redis_storage: AsyncRedisStorage,
    redis_conn_manager: ConnectionManager,
}

#[async_trait]
impl AsyncStorage for CachedRedisStorage {
    async fn get_namespaces(&self) -> Result<HashSet<Namespace>, StorageErr> {
        self.async_redis_storage.get_namespaces().await
    }

    async fn add_limit(&self, limit: &Limit) -> Result<(), StorageErr> {
        self.async_redis_storage.add_limit(limit).await
    }

    async fn get_limits(&self, namespace: &Namespace) -> Result<HashSet<Limit>, StorageErr> {
        let mut cached_limits = self.cached_limits_by_namespace.lock().await;

        match cached_limits.get_mut(namespace) {
            Some(limits) => Ok(limits.clone()),
            None => {
                let limits = self.async_redis_storage.get_limits(namespace).await?;
                cached_limits.insert(namespace.clone(), limits.clone(), self.ttl_cached_limits);
                Ok(limits)
            }
        }
    }

    async fn delete_limit(&self, limit: &Limit) -> Result<(), StorageErr> {
        self.async_redis_storage.delete_limit(limit).await
    }

    async fn delete_limits(&self, namespace: &Namespace) -> Result<(), StorageErr> {
        self.async_redis_storage.delete_limits(namespace).await
    }

    async fn is_within_limits(&self, counter: &Counter, delta: i64) -> Result<bool, StorageErr> {
        self.async_redis_storage
            .is_within_limits(counter, delta)
            .await
    }

    async fn update_counter(&self, counter: &Counter, delta: i64) -> Result<(), StorageErr> {
        self.async_redis_storage
            .update_counter(counter, delta)
            .await
    }

    async fn check_and_update(
        &self,
        counters: &HashSet<&Counter>,
        delta: i64,
    ) -> Result<bool, StorageErr> {
        let mut con = self.redis_conn_manager.clone();

        let mut not_cached: Vec<&Counter> = vec![];

        // Check cached counters
        let mut cached_counters = self.cached_counters.lock().await;
        for counter in counters {
            match cached_counters.get(counter) {
                Some(val) => {
                    if val - delta < 0 {
                        return Ok(false);
                    }
                }
                None => {
                    not_cached.push(counter);
                }
            }
        }

        // Fetch non-cached counters, cache them, and check them
        if !not_cached.is_empty() {
            let (counter_vals, counter_ttls_secs) =
                Self::values_with_ttls(&not_cached, &mut con).await?;

            for (i, &counter) in not_cached.iter().enumerate() {
                cached_counters.insert(counter.clone(), counter_vals[i], counter_ttls_secs[i]);
            }

            for (i, counter) in not_cached.iter().enumerate() {
                match counter_vals[i] {
                    Some(val) => {
                        if val - delta < 0 {
                            return Ok(false);
                        }
                    }
                    None => {
                        if counter.max_value() - delta < 0 {
                            return Ok(false);
                        }
                    }
                }
            }
        }

        for counter in counters {
            cached_counters.decrease_by(counter, delta);

            self.batcher_counter_updates
                .lock()
                .await
                .add_counter(counter, delta)
                .await;
        }

        Ok(true)
    }

    async fn get_counters(&self, namespace: &Namespace) -> Result<HashSet<Counter>, StorageErr> {
        self.async_redis_storage.get_counters(namespace).await
    }

    async fn clear(&self) -> Result<(), StorageErr> {
        self.async_redis_storage.clear().await
    }
}

impl CachedRedisStorage {
    pub async fn new(redis_url: &str) -> CachedRedisStorage {
        Self::new_with_options(
            redis_url,
            DEFAULT_MAX_CACHED_NAMESPACES,
            DEFAULT_TTL_CACHED_LIMITS,
            DEFAULT_FLUSHING_PERIOD,
            DEFAULT_MAX_CACHED_COUNTERS,
            DEFAULT_MAX_TTL_CACHED_COUNTERS,
            DEFAULT_TTL_RATIO_CACHED_COUNTERS,
        )
        .await
    }

    async fn new_with_options(
        redis_url: &str,
        max_cached_namespaces: usize,
        ttl_cached_limits: Duration,
        flushing_period: Duration,
        max_cached_counters: usize,
        ttl_cached_counters: Duration,
        ttl_ratio_cached_counters: u64,
    ) -> CachedRedisStorage {
        let redis_conn_manager =
            ConnectionManager::new(ConnectionInfo::from_str(redis_url).unwrap())
                .await
                .unwrap();

        let async_redis_storage =
            AsyncRedisStorage::new_with_conn_manager(redis_conn_manager.clone());

        let batcher = Arc::new(Mutex::new(Batcher::new(async_redis_storage.clone())));
        let batcher_flusher = batcher.clone();
        tokio::spawn(async move {
            loop {
                let time_start = Instant::now();
                batcher_flusher.lock().await.flush().await;
                let sleep_time = flushing_period
                    .checked_sub(time_start.elapsed())
                    .unwrap_or_else(|| Duration::from_secs(0));
                tokio::time::delay_for(sleep_time).await;
            }
        });

        let cached_counters = CountersCacheBuilder::new()
            .max_cached_counters(max_cached_counters)
            .max_ttl_cached_counter(ttl_cached_counters)
            .ttl_ratio_cached_counter(ttl_ratio_cached_counters)
            .build();

        CachedRedisStorage {
            cached_limits_by_namespace: Mutex::new(TtlCache::new(max_cached_namespaces)),
            ttl_cached_limits,
            cached_counters: Mutex::new(cached_counters),
            batcher_counter_updates: batcher,
            redis_conn_manager,
            async_redis_storage,
        }
    }

    async fn values_with_ttls(
        counters: &[&Counter],
        redis_con: &mut ConnectionManager,
    ) -> Result<(Vec<Option<i64>>, Vec<i64>), StorageErr> {
        let counter_keys: Vec<String> = counters
            .iter()
            .map(|counter| key_for_counter(counter))
            .collect();

        let counter_vals: Vec<Option<i64>> = redis::cmd("MGET")
            .arg(counter_keys.clone())
            .query_async(&mut redis_con.clone())
            .await?;

        let mut redis_pipeline = redis::pipe();
        redis_pipeline.atomic();

        for counter_key in counter_keys {
            redis_pipeline.cmd("TTL").arg(counter_key);
        }

        let counter_ttls_secs: Vec<i64> =
            redis_pipeline.query_async(&mut redis_con.clone()).await?;

        Ok((counter_vals, counter_ttls_secs))
    }
}

pub struct CachedRedisStorageBuilder {
    redis_url: String,
    max_cached_namespaces: usize,
    ttl_cached_limits: Duration,
    flushing_period: Duration,
    max_cached_counters: usize,
    max_ttl_cached_counters: Duration,
    ttl_ratio_cached_counters: u64,
}

impl CachedRedisStorageBuilder {
    pub fn new(redis_url: &str) -> CachedRedisStorageBuilder {
        CachedRedisStorageBuilder {
            redis_url: redis_url.to_string(),
            max_cached_namespaces: DEFAULT_MAX_CACHED_NAMESPACES,
            ttl_cached_limits: DEFAULT_TTL_CACHED_LIMITS,
            flushing_period: DEFAULT_FLUSHING_PERIOD,
            max_cached_counters: DEFAULT_MAX_CACHED_COUNTERS,
            max_ttl_cached_counters: DEFAULT_MAX_TTL_CACHED_COUNTERS,
            ttl_ratio_cached_counters: DEFAULT_TTL_RATIO_CACHED_COUNTERS,
        }
    }

    pub fn max_cached_namespaces(
        mut self,
        max_cached_namespaces: usize,
    ) -> CachedRedisStorageBuilder {
        self.max_cached_namespaces = max_cached_namespaces;
        self
    }

    pub fn ttl_cached_limits(mut self, ttl_cached_limits: Duration) -> CachedRedisStorageBuilder {
        self.ttl_cached_limits = ttl_cached_limits;
        self
    }

    pub fn flushing_period(mut self, flushing_period: Duration) -> CachedRedisStorageBuilder {
        self.flushing_period = flushing_period;
        self
    }

    pub fn max_cached_counters(mut self, max_cached_counters: usize) -> CachedRedisStorageBuilder {
        self.max_cached_counters = max_cached_counters;
        self
    }

    pub fn max_ttl_cached_counters(
        mut self,
        max_ttl_cached_counters: Duration,
    ) -> CachedRedisStorageBuilder {
        self.max_ttl_cached_counters = max_ttl_cached_counters;
        self
    }

    pub fn ttl_ratio_cached_counters(
        mut self,
        ttl_ratio_cached_counters: u64,
    ) -> CachedRedisStorageBuilder {
        self.ttl_ratio_cached_counters = ttl_ratio_cached_counters;
        self
    }

    pub async fn build(self) -> CachedRedisStorage {
        CachedRedisStorage::new_with_options(
            &self.redis_url,
            self.max_cached_namespaces,
            self.ttl_cached_limits,
            self.flushing_period,
            self.max_cached_counters,
            self.max_ttl_cached_counters,
            self.ttl_ratio_cached_counters,
        )
        .await
    }
}
