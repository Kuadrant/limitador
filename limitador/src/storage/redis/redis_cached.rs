use crate::counter::Counter;
use crate::limit::Limit;
use crate::storage::keys::*;
use crate::storage::redis::counters_cache::{
    CachedCounterValue, CountersCache, CountersCacheBuilder,
};
use crate::storage::redis::redis_async::AsyncRedisStorage;
use crate::storage::redis::scripts::BATCH_UPDATE_COUNTERS;
use crate::storage::redis::{
    DEFAULT_BATCH_SIZE, DEFAULT_FLUSHING_PERIOD_SEC, DEFAULT_MAX_CACHED_COUNTERS,
    DEFAULT_RESPONSE_TIMEOUT_MS,
};
use crate::storage::{AsyncCounterStorage, Authorization, StorageErr};
use async_trait::async_trait;
use metrics::gauge;
use redis::aio::{ConnectionLike, ConnectionManager, ConnectionManagerConfig};
use redis::{ConnectionInfo, RedisError};
use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{error, info, info_span, warn, Instrument};

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
    async fn is_within_limits(&self, counter: &Counter, delta: u64) -> Result<bool, StorageErr> {
        self.async_redis_storage
            .is_within_limits(counter, delta)
            .await
    }

    #[tracing::instrument(skip_all)]
    async fn update_counter(&self, counter: &Counter, delta: u64) -> Result<(), StorageErr> {
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
        delta: u64,
        load_counters: bool,
    ) -> Result<Authorization, StorageErr> {
        let mut not_cached: Vec<&mut Counter> = vec![];
        let mut first_limited = None;

        // Check cached counters
        for counter in counters.iter_mut() {
            match self.cached_counters.get(counter) {
                Some(val) => {
                    if first_limited.is_none() && val.is_limited(counter, delta) {
                        let a =
                            Authorization::Limited(counter.limit().name().map(|n| n.to_owned()));
                        if !load_counters {
                            return Ok(a);
                        }
                        first_limited = Some(a);
                    }
                    if load_counters {
                        counter.set_remaining(
                            val.remaining(counter)
                                .checked_sub(delta)
                                .unwrap_or_default(),
                        );
                        counter.set_expires_in(val.ttl());
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
                if first_limited.is_none() && remaining == 0 {
                    first_limited = Some(Authorization::Limited(
                        counter.limit().name().map(|n| n.to_owned()),
                    ));
                }
                if load_counters {
                    counter.set_remaining(remaining - delta);
                    counter.set_expires_in(fake.ttl()); // todo: this is a plain lie!
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
    async fn get_counters(
        &self,
        limits: &HashSet<Arc<Limit>>,
    ) -> Result<HashSet<Counter>, StorageErr> {
        self.async_redis_storage.get_counters(limits).await
    }

    #[tracing::instrument(skip_all)]
    async fn delete_counters(&self, limits: &HashSet<Arc<Limit>>) -> Result<(), StorageErr> {
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
            DEFAULT_BATCH_SIZE,
            Duration::from_secs(DEFAULT_FLUSHING_PERIOD_SEC),
            DEFAULT_MAX_CACHED_COUNTERS,
            Duration::from_millis(DEFAULT_RESPONSE_TIMEOUT_MS),
        )
        .await
    }

    async fn new_with_options(
        redis_url: &str,
        batch_size: usize,
        flushing_period: Duration,
        max_cached_counters: usize,
        response_timeout: Duration,
    ) -> Result<Self, RedisError> {
        let info = ConnectionInfo::from_str(redis_url)?;
        let redis_conn_manager = ConnectionManager::new_with_config(
            redis::Client::open(info)
                .expect("This couldn't fail in the past, yet now it did somehow!"),
            ConnectionManagerConfig::default()
                .set_connection_timeout((response_timeout * 3) + Duration::from_millis(50))
                .set_response_timeout(response_timeout)
                .set_number_of_retries(1),
        )
        .await?;

        let cached_counters = CountersCacheBuilder::new()
            .max_cached_counters(max_cached_counters)
            .build(flushing_period);

        let counters_cache = Arc::new(cached_counters);
        let partitioned = Arc::new(AtomicBool::new(false));
        let async_redis_storage =
            AsyncRedisStorage::new_with_conn_manager(redis_conn_manager.clone()).await?;

        {
            let counters_cache_clone = counters_cache.clone();
            let conn = redis_conn_manager.clone();
            let p = Arc::clone(&partitioned);
            tokio::spawn(async move {
                loop {
                    flush_batcher_and_update_counters(
                        conn.clone(),
                        counters_cache_clone.clone(),
                        p.clone(),
                        batch_size,
                    )
                    .await;
                }
            });
        }

        async_redis_storage
            .load_script(BATCH_UPDATE_COUNTERS)
            .await?;

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
            gauge!("datastore_partitioned").set(1);
            error!("Partition to Redis detected!")
        } else {
            gauge!("datastore_partitioned").set(0);
            warn!("Partition to Redis resolved!");
        }
    }
    we_flipped
}

pub struct CachedRedisStorageBuilder {
    redis_url: String,
    batch_size: usize,
    flushing_period: Duration,
    max_cached_counters: usize,
    response_timeout: Duration,
}

impl CachedRedisStorageBuilder {
    pub fn new(redis_url: &str) -> Self {
        Self {
            redis_url: redis_url.to_string(),
            batch_size: DEFAULT_BATCH_SIZE,
            flushing_period: Duration::from_secs(DEFAULT_FLUSHING_PERIOD_SEC),
            max_cached_counters: DEFAULT_MAX_CACHED_COUNTERS,
            response_timeout: Duration::from_millis(DEFAULT_RESPONSE_TIMEOUT_MS),
        }
    }

    pub fn batch_size(mut self, batch_size: usize) -> Self {
        self.batch_size = batch_size;
        self
    }

    pub fn flushing_period(mut self, flushing_period: Duration) -> Self {
        self.flushing_period = flushing_period;
        self
    }

    pub fn max_cached_counters(mut self, max_cached_counters: usize) -> Self {
        self.max_cached_counters = max_cached_counters;
        self
    }

    pub fn response_timeout(mut self, response_timeout: Duration) -> Self {
        self.response_timeout = response_timeout;
        self
    }

    pub async fn build(self) -> Result<CachedRedisStorage, RedisError> {
        CachedRedisStorage::new_with_options(
            &self.redis_url,
            self.batch_size,
            self.flushing_period,
            self.max_cached_counters,
            self.response_timeout,
        )
        .await
    }
}

async fn update_counters<C: ConnectionLike>(
    redis_conn: &mut C,
    counters_and_deltas: HashMap<Counter, Arc<CachedCounterValue>>,
) -> Result<Vec<(Counter, u64, u64, SystemTime)>, (Vec<(Counter, u64, u64, SystemTime)>, StorageErr)>
{
    let redis_script = redis::Script::new(BATCH_UPDATE_COUNTERS);
    let mut script_invocation = redis_script.prepare_invoke();

    let res = if counters_and_deltas.is_empty() {
        Default::default()
    } else {
        let mut res: Vec<(Counter, u64, u64, SystemTime)> =
            Vec::with_capacity(counters_and_deltas.len());

        for (counter, value) in counters_and_deltas {
            let (delta, last_value_from_redis) = value
                .pending_writes_and_value()
                .expect("State machine is wrong!");
            if delta > 0 {
                script_invocation.key(key_for_counter(&counter));
                script_invocation.key(key_for_counters_of_limit(counter.limit()));
                script_invocation.arg(counter.window().as_secs());
                script_invocation.arg(delta);
                // We need to store the counter in the actual order we are sending it to the script
                res.push((counter, last_value_from_redis, delta, UNIX_EPOCH));
            }
        }

        // The redis crate is not working with tables, thus the response will be a Vec of counter values
        let script_res: Vec<i64> = match script_invocation
            .invoke_async(redis_conn)
            .instrument(info_span!("datastore"))
            .await
        {
            Ok(res) => res,
            Err(err) => {
                return Err((res, err.into()));
            }
        };

        // We need to update the values and ttls returned by redis
        let counters_range = 0..res.len();
        let script_res_range = (0..script_res.len()).step_by(2);

        for (i, j) in counters_range.zip(script_res_range) {
            let (_, val, delta, expires_at) = &mut res[i];
            *delta = u64::try_from(script_res[j])
                .unwrap_or(0)
                .saturating_sub(*val); // new value - previous one = remote writes
            *val = u64::try_from(script_res[j]).unwrap_or(0); // update to value to newest
            *expires_at =
                UNIX_EPOCH + Duration::from_millis(u64::try_from(script_res[j + 1]).unwrap_or(0));
        }
        res
    };

    Ok(res)
}

#[allow(unknown_lints, clippy::manual_inspect)]
#[tracing::instrument(skip_all)]
async fn flush_batcher_and_update_counters<C: ConnectionLike>(
    mut redis_conn: C,
    cached_counters: Arc<CountersCache>,
    partitioned: Arc<AtomicBool>,
    batch_size: usize,
) {
    let updated_counters = cached_counters
        .batcher()
        .consume(batch_size, |counters| {
            if !counters.is_empty() && !partitioned.load(Ordering::Acquire) {
                info!("Flushing {} counter updates", counters.len());
            }
            update_counters(&mut redis_conn, counters)
        })
        .await
        .map(|result| {
            flip_partitioned(&partitioned, false);
            result
        })
        .or_else(|(data, err)| {
            if err.is_transient() {
                let new_partition = flip_partitioned(&partitioned, true);
                if new_partition {
                    warn!("Error flushing {}", err);
                }
                let counters = data.len();
                let mut reverted = 0;
                for (counter, old_value, pending_writes, _) in data {
                    if cached_counters
                        .return_pending_writes(&counter, old_value, pending_writes)
                        .is_err()
                    {
                        error!("Couldn't revert writes back to {:?}", &counter);
                    } else {
                        reverted += 1;
                    }
                }
                if new_partition {
                    warn!("Reverted {} of {} counter increments", reverted, counters);
                }
                Ok(Vec::new())
            } else {
                Err(err)
            }
        })
        .expect("Unrecoverable Redis error!");

    for (counter, new_value, remote_deltas, ttl) in updated_counters {
        cached_counters.apply_remote_delta(counter, new_value, remote_deltas, ttl);
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
    use redis::{Cmd, ErrorKind, RedisError, Value};
    use redis_test::{MockCmd, MockRedisConnection};
    use std::collections::HashMap;
    use std::io;
    use std::ops::Add;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
        const NEW_VALUE_FROM_REDIS: u64 = 10;
        const INITIAL_VALUE_FROM_REDIS: u64 = 1;
        const LOCAL_INCREMENTS: u64 = 2;

        let mut counters_and_deltas = HashMap::new();
        let counter = Counter::new(
            Limit::new(
                "test_namespace",
                10,
                60,
                vec!["req.method == 'GET'"],
                vec!["app_id"],
            )
            .expect("This must be a valid limit!"),
            Default::default(),
        );

        let arc = Arc::new(CachedCounterValue::from_authority(
            &counter,
            INITIAL_VALUE_FROM_REDIS,
        ));
        arc.delta(&counter, LOCAL_INCREMENTS);
        counters_and_deltas.insert(counter.clone(), arc);

        let one_sec_from_now = SystemTime::now().add(Duration::from_secs(1));
        let mock_response = Value::Array(vec![
            Value::Int(NEW_VALUE_FROM_REDIS as i64),
            Value::Int(
                one_sec_from_now
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as i64,
            ),
        ]);

        let mut mock_client = MockRedisConnection::new(vec![MockCmd::new(
            redis::cmd("EVALSHA")
                .arg("95a717e821d8fbdd667b5e4c6fede4c9cad16006")
                .arg("2")
                .arg(key_for_counter(&counter))
                .arg(key_for_counters_of_limit(counter.limit()))
                .arg(60)
                .arg(LOCAL_INCREMENTS),
            Ok(mock_response),
        )]);

        let mut result = update_counters(&mut mock_client, counters_and_deltas)
            .await
            .unwrap();

        let (c, new_value, remote_increments, expire_at) = result.remove(0);
        assert_eq!(key_for_counter(&counter), key_for_counter(&c));
        assert_eq!(NEW_VALUE_FROM_REDIS, new_value);
        assert_eq!(
            NEW_VALUE_FROM_REDIS - INITIAL_VALUE_FROM_REDIS - LOCAL_INCREMENTS,
            remote_increments
        );
        assert_eq!(
            one_sec_from_now
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis(),
            expire_at.duration_since(UNIX_EPOCH).unwrap().as_millis()
        );
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
            )
            .expect("This must be a valid limit!"),
            Default::default(),
        );

        let mock_response = Value::Array(vec![
            Value::Int(8),
            Value::Int(
                SystemTime::now()
                    .add(Duration::from_secs(1))
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as i64,
            ),
        ]);

        let mock_client = MockRedisConnection::new(vec![MockCmd::new(
            redis::cmd("EVALSHA")
                .arg("95a717e821d8fbdd667b5e4c6fede4c9cad16006")
                .arg("2")
                .arg(key_for_counter(&counter))
                .arg(key_for_counters_of_limit(counter.limit()))
                .arg(60)
                .arg(2),
            Ok(mock_response),
        )]);

        let cache = CountersCacheBuilder::new().build(Duration::from_millis(10));
        cache
            .batcher()
            .add(
                counter.clone(),
                Arc::new(CachedCounterValue::load_from_authority_asap(&counter, 2)),
            )
            .await;

        let cached_counters: Arc<CountersCache> = Arc::new(cache);
        let partitioned = Arc::new(AtomicBool::new(false));

        if let Some(c) = cached_counters.get(&counter) {
            assert_eq!(c.hits(&counter), 2);
        }

        flush_batcher_and_update_counters(mock_client, cached_counters.clone(), partitioned, 100)
            .await;

        let c = cached_counters.get(&counter).unwrap();
        assert_eq!(c.hits(&counter), 8);
        assert_eq!(c.pending_writes(), Ok(0));
    }

    #[tokio::test]
    async fn flush_batcher_reverts_on_err() {
        let counter = Counter::new(
            Limit::new(
                "test_namespace",
                10,
                60,
                vec!["req.method == 'POST'"],
                vec!["app_id"],
            )
            .expect("This must be a valid limit!"),
            Default::default(),
        );

        let error: RedisError = io::Error::new(io::ErrorKind::TimedOut, "That was long!").into();
        assert!(error.is_timeout());
        let mock_client = MockRedisConnection::new(vec![MockCmd::new::<&mut Cmd, Value>(
            redis::cmd("EVALSHA")
                .arg("95a717e821d8fbdd667b5e4c6fede4c9cad16006")
                .arg("2")
                .arg(key_for_counter(&counter))
                .arg(key_for_counters_of_limit(counter.limit()))
                .arg(60)
                .arg(3),
            Err(error),
        )]);

        let cache = CountersCacheBuilder::new().build(Duration::from_millis(10));
        let value = Arc::new(CachedCounterValue::from_authority(&counter, 2));
        value.delta(&counter, 3);
        cache.batcher().add(counter.clone(), value).await;

        let cached_counters: Arc<CountersCache> = Arc::new(cache);
        let partitioned = Arc::new(AtomicBool::new(false));

        if let Some(c) = cached_counters.get(&counter) {
            assert_eq!(c.hits(&counter), 5);
        }

        flush_batcher_and_update_counters(mock_client, cached_counters.clone(), partitioned, 100)
            .await;

        let c = cached_counters.get(&counter).unwrap();
        assert_eq!(c.hits(&counter), 5);
        assert_eq!(c.pending_writes(), Ok(3));
    }
}
