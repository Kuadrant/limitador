use crate::counter::Counter;
use crate::limit::Limit;
use crate::storage::keys::*;
use crate::storage::redis::counters_cache::{
    CachedCounterValue, CountersCache, CountersCacheBuilder,
};
use crate::storage::redis::redis_async::AsyncRedisStorage;
use crate::storage::redis::scripts::BATCH_UPDATE_COUNTERS;
use crate::storage::redis::{
    DEFAULT_FLUSHING_PERIOD_SEC, DEFAULT_MAX_CACHED_COUNTERS, DEFAULT_MAX_TTL_CACHED_COUNTERS_SEC,
    DEFAULT_RESPONSE_TIMEOUT_MS, DEFAULT_TTL_RATIO_CACHED_COUNTERS,
};
use crate::storage::{AsyncCounterStorage, Authorization, StorageErr};
use async_trait::async_trait;
use redis::aio::{ConnectionLike, ConnectionManager};
use redis::{ConnectionInfo, RedisError};
use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tracing::{debug_span, error, warn, Instrument};

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
    async_redis_storage: AsyncRedisStorage,
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
            for counter in not_cached.iter_mut() {
                let fake = CachedCounterValue::load_from_authority_asap(counter, 0);
                let remaining = fake.remaining(counter);
                if first_limited.is_none() && remaining <= 0 {
                    first_limited = Some(Authorization::Limited(
                        counter.limit().name().map(|n| n.to_owned()),
                    ));
                }
                if load_counters {
                    counter.set_remaining(remaining - delta);
                    counter.set_expires_in(fake.to_next_window()); // todo: this is a plain lie!
                }
            }
        }

        if let Some(l) = first_limited {
            return Ok(l);
        }

        // Update cached values
        for counter in counters.iter() {
            self.cached_counters.increase_by(counter, delta).await;
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
            1,
            response_timeout,
            // TLS handshake might result in an additional 2 RTTs to Redis, adding some headroom as well
            (response_timeout * 3) + Duration::from_millis(50),
        )
        .await?;

        let cached_counters = CountersCacheBuilder::new()
            .max_cached_counters(max_cached_counters)
            .max_ttl_cached_counter(ttl_cached_counters)
            .ttl_ratio_cached_counter(ttl_ratio_cached_counters)
            .build(flushing_period);

        let counters_cache = Arc::new(cached_counters);
        let partitioned = Arc::new(AtomicBool::new(false));
        let async_redis_storage =
            AsyncRedisStorage::new_with_conn_manager(redis_conn_manager.clone());

        {
            let storage = async_redis_storage.clone();
            let counters_cache_clone = counters_cache.clone();
            let conn = redis_conn_manager.clone();
            let p = Arc::clone(&partitioned);
            tokio::spawn(async move {
                loop {
                    flush_batcher_and_update_counters(
                        conn.clone(),
                        storage.is_alive().await,
                        counters_cache_clone.clone(),
                        p.clone(),
                    )
                    .await;
                }
            });
        }

        Ok(Self {
            cached_counters: counters_cache,
            async_redis_storage,
        })
    }
}

fn flip_partitioned(storage: &AtomicBool, partition: bool) -> bool {
    let we_flipped = storage
        .compare_exchange(!partition, partition, Ordering::Release, Ordering::Acquire)
        .is_ok();
    if we_flipped {
        if partition {
            error!("Partition to Redis detected!")
        } else {
            warn!("Partition to Redis resolved!");
        }
    }
    we_flipped
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

async fn update_counters<C: ConnectionLike>(
    redis_conn: &mut C,
    counters_and_deltas: HashMap<Counter, Arc<CachedCounterValue>>,
) -> Result<Vec<(Counter, i64, i64)>, StorageErr> {
    let redis_script = redis::Script::new(BATCH_UPDATE_COUNTERS);
    let mut script_invocation = redis_script.prepare_invoke();

    let mut res: Vec<(Counter, i64, i64)> = Vec::with_capacity(counters_and_deltas.len());
    if counters_and_deltas.is_empty() {
        return Ok(res);
    }

    for (counter, delta) in counters_and_deltas {
        let delta = delta.pending_writes().expect("State machine is wrong!");
        if delta > 0 {
            script_invocation.key(key_for_counter(&counter));
            script_invocation.key(key_for_counters_of_limit(counter.limit()));
            script_invocation.arg(counter.seconds());
            script_invocation.arg(delta);
            // We need to store the counter in the actual order we are sending it to the script
            res.push((counter, 0, 0));
        }
    }

    let span = debug_span!("datastore");
    // The redis crate is not working with tables, thus the response will be a Vec of counter values
    let script_res: Vec<i64> = script_invocation
        .invoke_async(redis_conn)
        .instrument(span)
        .await?;

    // We need to update the values and ttls returned by redis
    let counters_range = 0..res.len();
    let script_res_range = (0..script_res.len()).step_by(2);

    for (i, j) in counters_range.zip(script_res_range) {
        let (_, val, ttl) = &mut res[i];
        *val = script_res[j];
        *ttl = script_res[j + 1];
    }

    Ok(res)
}

async fn flush_batcher_and_update_counters<C: ConnectionLike>(
    mut redis_conn: C,
    storage_is_alive: bool,
    cached_counters: Arc<CountersCache>,
    partitioned: Arc<AtomicBool>,
) {
    if partitioned.load(Ordering::Acquire) || !storage_is_alive {
        if !cached_counters.batcher().is_empty() {
            flip_partitioned(&partitioned, false);
        }
    } else {
        let updated_counters = cached_counters
            .batcher()
            .consume(100, |counters| update_counters(&mut redis_conn, counters))
            .await
            .or_else(|err| {
                if err.is_transient() {
                    flip_partitioned(&partitioned, true);
                    Ok(Vec::new())
                } else {
                    Err(err)
                }
            })
            .expect("Unrecoverable Redis error!");

        let time_start_update_counters = Instant::now();

        for (counter, value, ttl) in updated_counters {
            cached_counters.insert(
                counter,
                Option::from(value),
                ttl,
                Duration::from_millis(
                    (Instant::now() - time_start_update_counters).as_millis() as u64
                ),
                SystemTime::now(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::counter::Counter;
    use crate::limit::Limit;
    use crate::storage::keys::{key_for_counter, key_for_counters_of_limit};
    use crate::storage::redis::counters_cache::{
        CachedCounterValue, CountersCache, CountersCacheBuilder,
    };
    use crate::storage::redis::redis_cached::{flush_batcher_and_update_counters, update_counters};
    use crate::storage::redis::CachedRedisStorage;
    use redis::{ErrorKind, Value};
    use redis_test::{MockCmd, MockRedisConnection};
    use std::collections::HashMap;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use std::time::{Duration, SystemTime};

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

    #[tokio::test]
    async fn batch_update_counters() {
        let mut counters_and_deltas = HashMap::new();
        let counter = Counter::new(
            Limit::new(
                "test_namespace",
                10,
                60,
                vec!["req.method == 'GET'"],
                vec!["app_id"],
            ),
            Default::default(),
        );

        counters_and_deltas.insert(
            counter.clone(),
            Arc::new(CachedCounterValue::from_authority(
                &counter,
                1,
                Duration::from_secs(60),
            )),
        );

        let mock_response = Value::Bulk(vec![Value::Int(10), Value::Int(60)]);

        let mut mock_client = MockRedisConnection::new(vec![MockCmd::new(
            redis::cmd("EVALSHA")
                .arg("1e87383cf7dba2bd0f9972ed73671274e6cbd5da")
                .arg("2")
                .arg(key_for_counter(&counter))
                .arg(key_for_counters_of_limit(counter.limit()))
                .arg(60)
                .arg(1),
            Ok(mock_response.clone()),
        )]);

        let result = update_counters(&mut mock_client, counters_and_deltas).await;

        assert!(result.is_ok());

        let (c, v, t) = result.unwrap()[0].clone();
        assert_eq!(
            "req.method == \"GET\"",
            c.limit().conditions().iter().collect::<Vec<_>>()[0]
        );
        assert_eq!(10, v);
        assert_eq!(60, t);
    }

    #[tokio::test]
    async fn flush_batcher_and_update_counters_test() {
        let counter = Counter::new(
            Limit::new(
                "test_namespace",
                10,
                60,
                vec!["req.method == 'POST'"],
                vec!["app_id"],
            ),
            Default::default(),
        );

        let mock_response = Value::Bulk(vec![Value::Int(8), Value::Int(60)]);

        let mock_client = MockRedisConnection::new(vec![MockCmd::new(
            redis::cmd("EVALSHA")
                .arg("1e87383cf7dba2bd0f9972ed73671274e6cbd5da")
                .arg("2")
                .arg(key_for_counter(&counter))
                .arg(key_for_counters_of_limit(counter.limit()))
                .arg(60)
                .arg(2),
            Ok(mock_response.clone()),
        )]);

        let cache = CountersCacheBuilder::new().build(Duration::from_millis(1));
        cache
            .batcher()
            .add(
                counter.clone(),
                Arc::new(CachedCounterValue::from_authority(
                    &counter,
                    2,
                    Duration::from_secs(60),
                )),
            )
            .await;
        cache.insert(
            counter.clone(),
            Some(1),
            10,
            Duration::from_secs(0),
            SystemTime::now(),
        );
        let cached_counters: Arc<CountersCache> = Arc::new(cache);
        let partitioned = Arc::new(AtomicBool::new(false));

        if let Some(c) = cached_counters.get(&counter) {
            assert_eq!(c.hits(&counter), 2);
        }

        flush_batcher_and_update_counters(mock_client, true, cached_counters.clone(), partitioned)
            .await;

        if let Some(c) = cached_counters.get(&counter) {
            assert_eq!(c.hits(&counter), 8);
        }
    }
}
