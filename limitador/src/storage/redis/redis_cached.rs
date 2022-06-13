use crate::counter::Counter;
use crate::limit::Limit;
use crate::storage::keys::*;
use crate::storage::redis::batcher::Batcher;
use crate::storage::redis::counters_cache::{
    CountersCache, CountersCacheBuilder, DEFAULT_MAX_CACHED_COUNTERS,
    DEFAULT_MAX_TTL_CACHED_COUNTERS, DEFAULT_TTL_RATIO_CACHED_COUNTERS,
};
use crate::storage::redis::redis_async::AsyncRedisStorage;
use crate::storage::redis::scripts::VALUES_AND_TTLS;
use crate::storage::{AsyncCounterStorage, Authorization, StorageErr};
use async_trait::async_trait;
use redis::aio::ConnectionManager;
use redis::ConnectionInfo;
use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

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
// - Introduce a mechanism to avoid going to Redis to fetch the same counter
// multiple times when it is not cached.

const DEFAULT_FLUSHING_PERIOD: Duration = Duration::from_secs(1);

pub struct CachedRedisStorage {
    cached_counters: Mutex<CountersCache>,
    batcher_counter_updates: Arc<Mutex<Batcher>>,
    async_redis_storage: AsyncRedisStorage,
    redis_conn_manager: ConnectionManager,
    batching_is_enabled: bool,
}

#[async_trait]
impl AsyncCounterStorage for CachedRedisStorage {
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

    // Notice that this method does not guarantee 100% accuracy when applying the
    // limits. In order to do so, we'd need to run this whole function
    // atomically, but that'd be too slow.
    // This function trades accuracy for speed.
    async fn check_and_update<'c>(
        &self,
        counters: &HashSet<&'c Counter>,
        delta: i64,
    ) -> Result<Authorization<'c>, StorageErr> {
        let mut con = self.redis_conn_manager.clone();

        let mut not_cached: Vec<&Counter> = vec![];

        // Check cached counters
        {
            let cached_counters = self.cached_counters.lock().await;
            for counter in counters {
                match cached_counters.get(counter) {
                    Some(val) => {
                        if val - delta < 0 {
                            return Ok(Authorization::Limited(counter));
                        }
                    }
                    None => {
                        not_cached.push(counter);
                    }
                }
            }
        }

        // Fetch non-cached counters, cache them, and check them
        if !not_cached.is_empty() {
            let time_start_get_ttl = Instant::now();

            let (counter_vals, counter_ttls_msecs) =
                Self::values_with_ttls(&not_cached, &mut con).await?;

            // Some time could have passed from the moment we got the TTL from Redis.
            // This margin is not exact, because we don't know exactly the
            // moment that Redis returned a particular TTL, but this
            // approximation should be good enough.
            let ttl_margin =
                Duration::from_millis((Instant::now() - time_start_get_ttl).as_millis() as u64);

            {
                let mut cached_counters = self.cached_counters.lock().await;
                for (i, &counter) in not_cached.iter().enumerate() {
                    cached_counters.insert(
                        counter.clone(),
                        counter_vals[i],
                        counter_ttls_msecs[i],
                        ttl_margin,
                    );
                }
            }

            for (i, counter) in not_cached.iter().enumerate() {
                match counter_vals[i] {
                    Some(val) => {
                        if val - delta < 0 {
                            return Ok(Authorization::Limited(counter));
                        }
                    }
                    None => {
                        if counter.max_value() - delta < 0 {
                            return Ok(Authorization::Limited(counter));
                        }
                    }
                }
            }
        }

        // Update cached values
        {
            let mut cached_counters = self.cached_counters.lock().await;
            for counter in counters {
                cached_counters.decrease_by(counter, delta);
            }
        }

        // Batch or update depending on configuration
        if self.batching_is_enabled {
            let batcher = self.batcher_counter_updates.lock().await;
            for counter in counters {
                batcher.add_counter(counter, delta).await
            }
        } else {
            for counter in counters {
                self.update_counter(counter, delta).await?
            }
        }

        Ok(Authorization::Ok)
    }

    async fn get_counters(&self, limits: HashSet<Limit>) -> Result<HashSet<Counter>, StorageErr> {
        self.async_redis_storage.get_counters(limits).await
    }

    async fn delete_counters(&self, limits: HashSet<Limit>) -> Result<(), StorageErr> {
        self.async_redis_storage.delete_counters(limits).await
    }

    async fn clear(&self) -> Result<(), StorageErr> {
        self.async_redis_storage.clear().await
    }
}

impl CachedRedisStorage {
    pub async fn new(redis_url: &str) -> Self {
        Self::new_with_options(
            redis_url,
            Some(DEFAULT_FLUSHING_PERIOD),
            DEFAULT_MAX_CACHED_COUNTERS,
            DEFAULT_MAX_TTL_CACHED_COUNTERS,
            DEFAULT_TTL_RATIO_CACHED_COUNTERS,
        )
        .await
    }

    async fn new_with_options(
        redis_url: &str,
        flushing_period: Option<Duration>,
        max_cached_counters: usize,
        ttl_cached_counters: Duration,
        ttl_ratio_cached_counters: u64,
    ) -> Self {
        let redis_conn_manager = ConnectionManager::new(
            redis::Client::open(ConnectionInfo::from_str(redis_url).unwrap()).unwrap(),
        )
        .await
        .unwrap();

        let async_redis_storage =
            AsyncRedisStorage::new_with_conn_manager(redis_conn_manager.clone());

        let batcher = Arc::new(Mutex::new(Batcher::new(async_redis_storage.clone())));
        if let Some(flushing_period) = flushing_period {
            let batcher_flusher = batcher.clone();
            tokio::spawn(async move {
                loop {
                    let time_start = Instant::now();
                    batcher_flusher.lock().await.flush().await;
                    let sleep_time = flushing_period
                        .checked_sub(time_start.elapsed())
                        .unwrap_or_else(|| Duration::from_secs(0));
                    tokio::time::sleep(sleep_time).await;
                }
            });
        }

        let cached_counters = CountersCacheBuilder::new()
            .max_cached_counters(max_cached_counters)
            .max_ttl_cached_counter(ttl_cached_counters)
            .ttl_ratio_cached_counter(ttl_ratio_cached_counters)
            .build();

        Self {
            cached_counters: Mutex::new(cached_counters),
            batcher_counter_updates: batcher,
            redis_conn_manager,
            async_redis_storage,
            batching_is_enabled: flushing_period.is_some(),
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

        let script = redis::Script::new(VALUES_AND_TTLS);
        let mut script_invocation = script.prepare_invoke();

        for counter_key in counter_keys {
            script_invocation.key(counter_key);
        }

        let script_res: Vec<Option<i64>> = script_invocation
            .invoke_async::<_, _>(&mut redis_con.clone())
            .await?;

        let mut counter_vals: Vec<Option<i64>> = vec![];
        let mut counter_ttls_msecs: Vec<i64> = vec![];

        for val_ttl_pair in script_res.chunks(2) {
            counter_vals.push(val_ttl_pair[0]);
            counter_ttls_msecs.push(val_ttl_pair[1].unwrap());
        }

        Ok((counter_vals, counter_ttls_msecs))
    }
}

pub struct CachedRedisStorageBuilder {
    redis_url: String,
    flushing_period: Option<Duration>,
    max_cached_counters: usize,
    max_ttl_cached_counters: Duration,
    ttl_ratio_cached_counters: u64,
}

impl CachedRedisStorageBuilder {
    pub fn new(redis_url: &str) -> Self {
        Self {
            redis_url: redis_url.to_string(),
            flushing_period: Some(DEFAULT_FLUSHING_PERIOD),
            max_cached_counters: DEFAULT_MAX_CACHED_COUNTERS,
            max_ttl_cached_counters: DEFAULT_MAX_TTL_CACHED_COUNTERS,
            ttl_ratio_cached_counters: DEFAULT_TTL_RATIO_CACHED_COUNTERS,
        }
    }

    pub fn flushing_period(mut self, flushing_period: Option<Duration>) -> Self {
        self.flushing_period = flushing_period;
        self
    }

    pub fn max_cached_counters(mut self, max_cached_counters: usize) -> Self {
        self.max_cached_counters = max_cached_counters;
        self
    }

    pub fn max_ttl_cached_counters(mut self, max_ttl_cached_counters: Duration) -> Self {
        self.max_ttl_cached_counters = max_ttl_cached_counters;
        self
    }

    pub fn ttl_ratio_cached_counters(mut self, ttl_ratio_cached_counters: u64) -> Self {
        self.ttl_ratio_cached_counters = ttl_ratio_cached_counters;
        self
    }

    pub async fn build(self) -> CachedRedisStorage {
        CachedRedisStorage::new_with_options(
            &self.redis_url,
            self.flushing_period,
            self.max_cached_counters,
            self.max_ttl_cached_counters,
            self.ttl_ratio_cached_counters,
        )
        .await
    }
}
