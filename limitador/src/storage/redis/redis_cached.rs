use crate::counter::Counter;
use crate::limit::Limit;
use crate::storage::atomic_expiring_value::AtomicExpiringValue;
use crate::storage::keys::*;
use crate::storage::redis::counters_cache::{CountersCache, CountersCacheBuilder};
use crate::storage::redis::redis_async::AsyncRedisStorage;
use crate::storage::redis::scripts::VALUES_AND_TTLS;
use crate::storage::redis::{
    DEFAULT_FLUSHING_PERIOD_SEC, DEFAULT_MAX_CACHED_COUNTERS, DEFAULT_MAX_TTL_CACHED_COUNTERS_SEC,
    DEFAULT_RESPONSE_TIMEOUT_MS, DEFAULT_TTL_RATIO_CACHED_COUNTERS,
};
use crate::storage::{AsyncCounterStorage, Authorization, StorageErr};
use async_trait::async_trait;
use redis::aio::ConnectionManager;
use redis::{ConnectionInfo, RedisError};
use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use tracing::{error, warn};

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

pub struct CachedRedisStorage {
    cached_counters: Arc<CountersCache>,
    batcher_counter_updates: Arc<Mutex<HashMap<Counter, AtomicExpiringValue>>>,
    async_redis_storage: AsyncRedisStorage,
    redis_conn_manager: ConnectionManager,
    partitioned: Arc<AtomicBool>,
}

#[async_trait]
impl AsyncCounterStorage for CachedRedisStorage {
    #[tracing::instrument(skip_all)]
    async fn is_within_limits(&self, counter: &Counter, delta: i64) -> Result<bool, StorageErr> {
        self.async_redis_storage
            .is_within_limits(counter, delta)
            .await
    }

    #[tracing::instrument(skip_all)]
    async fn update_counter(&self, counter: &Counter, delta: i64) -> Result<(), StorageErr> {
        self.async_redis_storage
            .update_counter(counter, delta)
            .await
    }

    // Notice that this method does not guarantee 100% accuracy when applying the
    // limits. In order to do so, we'd need to run this whole function
    // atomically, but that'd be too slow.
    // This function trades accuracy for speed.
    #[tracing::instrument(skip_all)]
    async fn check_and_update<'a>(
        &self,
        counters: &mut Vec<Counter>,
        delta: i64,
        load_counters: bool,
    ) -> Result<Authorization, StorageErr> {
        let mut not_cached: Vec<&mut Counter> = vec![];
        let mut first_limited = None;

        let now = SystemTime::now();
        // Check cached counters
        for counter in counters.iter_mut() {
            match self.cached_counters.get(counter) {
                Some(val) if !val.expired_at(now) => {
                    if first_limited.is_none() && val.is_limited(counter, delta) {
                        let a =
                            Authorization::Limited(counter.limit().name().map(|n| n.to_owned()));
                        if !load_counters {
                            return Ok(a);
                        }
                        first_limited = Some(a);
                    }
                    if load_counters {
                        counter.set_remaining(val.remaining(counter) - delta);
                        counter.set_expires_in(val.to_next_window());
                    }
                }
                _ => {
                    not_cached.push(counter);
                }
            }
        }

        // Fetch non-cached counters, cache them, and check them
        if !not_cached.is_empty() {
            let time_start_get_ttl = Instant::now();

            let (counter_vals, counter_ttls_msecs) = if self.is_partitioned() {
                self.fallback_vals_ttls(&not_cached)
            } else {
                self.values_with_ttls(&not_cached).await.or_else(|err| {
                    if err.is_transient() {
                        self.partitioned(true);
                        Ok(self.fallback_vals_ttls(&not_cached))
                    } else {
                        Err(err)
                    }
                })?
            };

            // Some time could have passed from the moment we got the TTL from Redis.
            // This margin is not exact, because we don't know exactly the
            // moment that Redis returned a particular TTL, but this
            // approximation should be good enough.
            let ttl_margin =
                Duration::from_millis((Instant::now() - time_start_get_ttl).as_millis() as u64);

            for (i, counter) in not_cached.iter_mut().enumerate() {
                let cached_value = self.cached_counters.insert(
                    counter.clone(),
                    counter_vals[i],
                    counter_ttls_msecs[i],
                    ttl_margin,
                    now,
                );
                let remaining = cached_value.remaining(counter);
                if first_limited.is_none() && remaining <= 0 {
                    first_limited = Some(Authorization::Limited(
                        counter.limit().name().map(|n| n.to_owned()),
                    ));
                }
                if load_counters {
                    counter.set_remaining(remaining - delta);
                    counter.set_expires_in(cached_value.to_next_window());
                }
            }
        }

        if let Some(l) = first_limited {
            return Ok(l);
        }

        // Update cached values
        for counter in counters.iter() {
            self.cached_counters.increase_by(counter, delta);
        }

        // Batch or update depending on configuration
        let mut batcher = self.batcher_counter_updates.lock().unwrap();
        let now = SystemTime::now();
        for counter in counters.iter() {
            match batcher.get_mut(counter) {
                Some(val) => {
                    val.update(delta, counter.seconds(), now);
                }
                None => {
                    batcher.insert(
                        counter.clone(),
                        AtomicExpiringValue::new(
                            delta,
                            now + Duration::from_secs(counter.seconds()),
                        ),
                    );
                }
            }
        }

        Ok(Authorization::Ok)
    }

    #[tracing::instrument(skip_all)]
    async fn get_counters(&self, limits: HashSet<Limit>) -> Result<HashSet<Counter>, StorageErr> {
        self.async_redis_storage.get_counters(limits).await
    }

    #[tracing::instrument(skip_all)]
    async fn delete_counters(&self, limits: HashSet<Limit>) -> Result<(), StorageErr> {
        self.async_redis_storage.delete_counters(limits).await
    }

    #[tracing::instrument(skip_all)]
    async fn clear(&self) -> Result<(), StorageErr> {
        self.async_redis_storage.clear().await
    }
}

impl CachedRedisStorage {
    pub async fn new(redis_url: &str) -> Result<Self, RedisError> {
        Self::new_with_options(
            redis_url,
            Duration::from_secs(DEFAULT_FLUSHING_PERIOD_SEC),
            DEFAULT_MAX_CACHED_COUNTERS,
            Duration::from_secs(DEFAULT_MAX_TTL_CACHED_COUNTERS_SEC),
            DEFAULT_TTL_RATIO_CACHED_COUNTERS,
            Duration::from_millis(DEFAULT_RESPONSE_TIMEOUT_MS),
        )
        .await
    }

    async fn new_with_options(
        redis_url: &str,
        flushing_period: Duration,
        max_cached_counters: usize,
        ttl_cached_counters: Duration,
        ttl_ratio_cached_counters: u64,
        response_timeout: Duration,
    ) -> Result<Self, RedisError> {
        let info = ConnectionInfo::from_str(redis_url)?;
        let redis_conn_manager = ConnectionManager::new_with_backoff_and_timeouts(
            redis::Client::open(info)
                .expect("This couldn't fail in the past, yet now it did somehow!"),
            2,
            100,
            6,
            response_timeout,
            Duration::from_secs(5),
        )
        .await?;

        let cached_counters = CountersCacheBuilder::new()
            .max_cached_counters(max_cached_counters)
            .max_ttl_cached_counter(ttl_cached_counters)
            .ttl_ratio_cached_counter(ttl_ratio_cached_counters)
            .build();

        let cacher = Arc::new(cached_counters);
        let cacher_clone = cacher.clone();

        let partitioned = Arc::new(AtomicBool::new(false));
        let async_redis_storage =
            AsyncRedisStorage::new_with_conn_manager(redis_conn_manager.clone());

        let storage = async_redis_storage.clone();
        let batcher: Arc<Mutex<HashMap<Counter, AtomicExpiringValue>>> =
            Arc::new(Mutex::new(Default::default()));
        let p = Arc::clone(&partitioned);
        let batcher_flusher = batcher.clone();
        let mut interval = tokio::time::interval(flushing_period);
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        flush_batcher_and_update_counters(batcher_flusher.clone(), storage.clone(), cacher_clone.clone(), p.clone()).await;
                    }
                }
            }
        });

        Ok(Self {
            cached_counters: cacher,
            batcher_counter_updates: batcher,
            redis_conn_manager,
            async_redis_storage,
            partitioned,
        })
    }

    fn is_partitioned(&self) -> bool {
        self.partitioned.load(Ordering::Acquire)
    }

    fn partitioned(&self, partition: bool) -> bool {
        if partition {
            error!("Partition to Redis detected!")
        }
        self.partitioned
            .compare_exchange(!partition, partition, Ordering::Release, Ordering::Acquire)
            .is_ok()
    }

    fn fallback_vals_ttls(&self, counters: &Vec<&mut Counter>) -> (Vec<Option<i64>>, Vec<i64>) {
        let mut vals = Vec::with_capacity(counters.len());
        let mut ttls = Vec::with_capacity(counters.len());
        for counter in counters {
            vals.push(Some(0i64));
            ttls.push(counter.limit().seconds() as i64 * 1000);
        }
        (vals, ttls)
    }

    async fn values_with_ttls(
        &self,
        counters: &[&mut Counter],
    ) -> Result<(Vec<Option<i64>>, Vec<i64>), StorageErr> {
        let mut redis_con = self.redis_conn_manager.clone();

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
            .invoke_async::<_, _>(&mut redis_con)
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
    flushing_period: Duration,
    max_cached_counters: usize,
    max_ttl_cached_counters: Duration,
    ttl_ratio_cached_counters: u64,
    response_timeout: Duration,
}

impl CachedRedisStorageBuilder {
    pub fn new(redis_url: &str) -> Self {
        Self {
            redis_url: redis_url.to_string(),
            flushing_period: Duration::from_secs(DEFAULT_FLUSHING_PERIOD_SEC),
            max_cached_counters: DEFAULT_MAX_CACHED_COUNTERS,
            max_ttl_cached_counters: Duration::from_secs(DEFAULT_MAX_TTL_CACHED_COUNTERS_SEC),
            ttl_ratio_cached_counters: DEFAULT_TTL_RATIO_CACHED_COUNTERS,
            response_timeout: Duration::from_millis(DEFAULT_RESPONSE_TIMEOUT_MS),
        }
    }

    pub fn flushing_period(mut self, flushing_period: Duration) -> Self {
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

    pub fn response_timeout(mut self, response_timeout: Duration) -> Self {
        self.response_timeout = response_timeout;
        self
    }

    pub async fn build(self) -> Result<CachedRedisStorage, RedisError> {
        CachedRedisStorage::new_with_options(
            &self.redis_url,
            self.flushing_period,
            self.max_cached_counters,
            self.max_ttl_cached_counters,
            self.ttl_ratio_cached_counters,
            self.response_timeout,
        )
        .await
    }
}

async fn flush_batcher_and_update_counters(
    batcher: Arc<Mutex<HashMap<Counter, AtomicExpiringValue>>>,
    storage: AsyncRedisStorage,
    cached_counters: Arc<CountersCache>,
    partitioned: Arc<AtomicBool>,
) {
    if partitioned.load(Ordering::Acquire) {
        if storage.is_alive().await {
            warn!("Partition to Redis resolved!");
            partitioned.store(false, Ordering::Release);
        }
    } else {
        let counters = {
            let mut batch = batcher.lock().unwrap();
            std::mem::take(&mut *batch)
        };

        let mut conn = storage.conn_manager.clone();

        let updated_counters = AsyncRedisStorage::update_counters(&mut conn, counters)
            .await
            .or_else(|err| {
                if err.is_transient() {
                    partitioned.store(true, Ordering::Release);
                    Ok(Vec::new())
                } else {
                    Err(err)
                }
            })
            .expect("Unrecoverable Redis error!");

        for (counter_key, value) in updated_counters {
            let counter = partial_counter_from_counter_key(&counter_key);
            cached_counters.increase_by(&counter, value);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::storage::redis::CachedRedisStorage;
    use redis::ErrorKind;

    #[tokio::test]
    async fn errs_on_bad_url() {
        let result = CachedRedisStorage::new("cassandra://127.0.0.1:6379").await;
        assert!(result.is_err());
        assert_eq!(result.err().unwrap().kind(), ErrorKind::InvalidClientConfig);
    }

    #[tokio::test]
    async fn errs_on_connection_issue() {
        let result = CachedRedisStorage::new("redis://127.0.0.1:21").await;
        assert!(result.is_err());
        let error = result.err().unwrap();
        assert_eq!(error.kind(), ErrorKind::IoError);
        assert!(error.is_connection_refusal())
    }
}
