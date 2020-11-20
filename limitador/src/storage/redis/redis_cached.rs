use crate::counter::Counter;
use crate::limit::{Limit, Namespace};
use crate::storage::redis::batcher::Batcher;
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
// - The TTLs, the flushing period, and the max number of cached elements should
// be configurable.
// - We shouldn't just cache the remaining of the counter. We should take into
// account that there might be other instances running.
// - Introduce a mechanism to avoid going to Redis to fetch the same counter
// multiple times when it is not cached.

const DEFAULT_FLUSHING_PERIOD: Duration = Duration::from_secs(1);
const DEFAULT_TTL_CACHED_LIMITS: Duration = Duration::from_secs(10);
const DEFAULT_MAX_CACHED_NAMESPACES: usize = 1000;
const DEFAULT_MAX_CACHED_COUNTERS: usize = 10000;
const MAX_TTL_CACHED_COUNTER: Duration = Duration::from_secs(5);
const TTL_RATIO_CACHED_COUNTER: u64 = 10;

pub struct CachedRedisStorage {
    cached_limits_by_namespace: Mutex<TtlCache<Namespace, HashSet<Limit>>>,
    cached_counters: Mutex<CountersCache>,
    batcher_counter_updates: Arc<Mutex<Batcher>>,
    async_redis_storage: AsyncRedisStorage,
    redis_conn_manager: ConnectionManager,
}

struct CountersCache {
    cache: TtlCache<Counter, i64>,
}

impl CountersCache {
    pub fn new() -> CountersCache {
        CountersCache {
            cache: TtlCache::new(DEFAULT_MAX_CACHED_COUNTERS),
        }
    }

    pub fn get(&self, counter: &Counter) -> Option<i64> {
        match self.cache.get(counter) {
            Some(val) => Some(*val),
            None => None,
        }
    }

    pub fn insert(&mut self, counter: Counter, redis_val: Option<i64>, redis_ttl: i64) {
        let counter_val = Self::value_from_redis_val(redis_val, counter.max_value());

        self.cache.insert(
            counter.clone(),
            counter_val,
            Self::ttl_from_redis_ttl(redis_ttl, counter.seconds(), counter_val),
        );
    }

    pub fn decrease_by(&mut self, counter: &Counter, delta: i64) {
        if let Some(val) = self.cache.get_mut(counter) {
            *val -= delta
        };
    }

    fn value_from_redis_val(redis_val: Option<i64>, counter_max: i64) -> i64 {
        match redis_val {
            Some(val) => val,
            None => counter_max,
        }
    }

    fn ttl_from_redis_ttl(redis_ttl: i64, counter_seconds: u64, counter_val: i64) -> Duration {
        // Redis returns -2 when the key does not exist. Ref:
        // https://redis.io/commands/ttl
        // This function returns a ttl of the given counter seconds in this
        // case.

        let counter_ttl = if redis_ttl >= 0 {
            Duration::from_secs(redis_ttl as u64)
        } else {
            Duration::from_secs(counter_seconds)
        };

        // If a counter is already at 0, we can cache it for as long as its TTL
        // is in Redis. This does not depend on the requests received by other
        // instances of Limitador. No matter what they do, we know that the
        // counter is not going to recover its quota until it expires in Redis.
        if counter_val <= 0 {
            return counter_ttl;
        }

        // Expire the counter in the cache before it expires in Redis.
        // There might be several Limitador instances updating the Redis
        // counter. The tradeoff is as follows: the shorter the TTL in the
        // cache, the sooner we'll take into account those updates coming from
        // other instances. If the TTL in the cache is long, there will be less
        // accesses to Redis, so latencies will be better. However, it'll be
        // easier to go over the limits defined, because not taking into account
        // updates from other Limitador instances.
        let mut res =
            Duration::from_millis(counter_ttl.as_millis() as u64 / TTL_RATIO_CACHED_COUNTER);

        if res > MAX_TTL_CACHED_COUNTER {
            res = MAX_TTL_CACHED_COUNTER;
        }

        res
    }
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
                cached_limits.insert(namespace.clone(), limits.clone(), DEFAULT_TTL_CACHED_LIMITS);
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
                let sleep_time = DEFAULT_FLUSHING_PERIOD
                    .checked_sub(time_start.elapsed())
                    .unwrap_or_else(|| Duration::from_secs(0));
                tokio::time::delay_for(sleep_time).await;
            }
        });

        CachedRedisStorage {
            cached_limits_by_namespace: Mutex::new(TtlCache::new(DEFAULT_MAX_CACHED_NAMESPACES)),
            cached_counters: Mutex::new(CountersCache::new()),
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
