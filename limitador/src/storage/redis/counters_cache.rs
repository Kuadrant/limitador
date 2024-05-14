use crate::counter::Counter;
use crate::storage::atomic_expiring_value::AtomicExpiringValue;
use crate::storage::redis::DEFAULT_MAX_CACHED_COUNTERS;
use dashmap::mapref::entry::Entry;
use dashmap::DashMap;
use metrics::{counter, gauge, histogram};
use moka::notification::RemovalCause;
use moka::sync::Cache;
use std::collections::HashMap;
use std::future::Future;
use std::ops::Not;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::select;
use tokio::sync::{Notify, Semaphore};

#[derive(Debug)]
pub struct CachedCounterValue {
    value: AtomicExpiringValue,
    initial_value: AtomicU64,
    from_authority: AtomicBool,
}

impl CachedCounterValue {
    pub fn from_authority(counter: &Counter, value: u64) -> Self {
        let now = SystemTime::now();
        Self {
            value: AtomicExpiringValue::new(value, now + Duration::from_secs(counter.seconds())),
            initial_value: AtomicU64::new(value),
            from_authority: AtomicBool::new(true),
        }
    }

    pub fn load_from_authority_asap(counter: &Counter, temp_value: u64) -> Self {
        let now = SystemTime::now();
        Self {
            value: AtomicExpiringValue::new(
                temp_value,
                now + Duration::from_secs(counter.seconds()),
            ),
            initial_value: AtomicU64::new(0),
            from_authority: AtomicBool::new(false),
        }
    }

    pub fn add_from_authority(&self, delta: u64, expire_at: SystemTime) {
        self.value.add_and_set_expiry(delta, expire_at);
        self.initial_value.fetch_add(delta, Ordering::SeqCst);
        self.from_authority.store(true, Ordering::Release);
    }

    pub fn delta(&self, counter: &Counter, delta: u64) -> u64 {
        let value = self
            .value
            .update(delta, counter.seconds(), SystemTime::now());
        if value == delta {
            // new window, invalidate initial value
            // which happens _after_ the self.value was reset, see `pending_writes`
            self.initial_value.store(0, Ordering::SeqCst);
        }
        value
    }

    pub fn pending_writes(&self) -> Result<u64, ()> {
        self.pending_writes_and_value().map(|(writes, _)| writes)
    }

    pub fn pending_writes_and_value(&self) -> Result<(u64, u64), ()> {
        let start = self.initial_value.load(Ordering::SeqCst);
        let value = self.value.value_at(SystemTime::now());
        let offset = if start == 0 {
            value
        } else {
            let writes = value.checked_sub(start);
            // self.value expired, is now less than the writes of the previous window
            // which have not yet been reset... it'll be 0, so treat it as such.
            writes.unwrap_or(value)
        };
        match self
            .initial_value
            .compare_exchange(start, value, Ordering::SeqCst, Ordering::SeqCst)
        {
            Ok(_) => Ok((offset, value)),
            Err(newer) => {
                if newer == 0 {
                    // We got reset because of expiry, this fresh value can wait the next iteration
                    Ok((0, 0))
                } else {
                    // Concurrent call to this method?
                    // We could support that with a CAS loop in the future if needed
                    Err(())
                }
            }
        }
    }

    fn no_pending_writes(&self) -> bool {
        let start = self.initial_value.load(Ordering::SeqCst);
        let value = self.value.value_at(SystemTime::now());
        value - start == 0
    }

    pub fn hits(&self, _: &Counter) -> u64 {
        self.value.value_at(SystemTime::now())
    }

    pub fn remaining(&self, counter: &Counter) -> u64 {
        counter.max_value() - self.hits(counter)
    }

    pub fn is_limited(&self, counter: &Counter, delta: u64) -> bool {
        self.hits(counter) as i128 + delta as i128 > counter.max_value() as i128
    }

    pub fn to_next_window(&self) -> Duration {
        self.value.ttl()
    }

    pub fn requires_fast_flush(&self, within: &Duration) -> bool {
        self.from_authority.load(Ordering::Acquire).not() || &self.value.ttl() <= within
    }
}

pub struct Batcher {
    updates: DashMap<Counter, Arc<CachedCounterValue>>,
    notifier: Notify,
    interval: Duration,
    priority_flush: AtomicBool,
    limiter: Semaphore,
}

impl Batcher {
    fn new(period: Duration, max_cached_counters: usize) -> Self {
        Self {
            updates: Default::default(),
            notifier: Default::default(),
            interval: period,
            priority_flush: AtomicBool::new(false),
            limiter: Semaphore::new(max_cached_counters),
        }
    }

    pub async fn add(&self, counter: Counter, value: Arc<CachedCounterValue>) {
        let priority = value.requires_fast_flush(&self.interval);
        match self.updates.entry(counter.clone()) {
            Entry::Occupied(needs_merge) => {
                let arc = needs_merge.get();
                if !Arc::ptr_eq(arc, &value) {
                    arc.delta(&counter, value.pending_writes().unwrap());
                }
            }
            Entry::Vacant(miss) => {
                self.limiter.acquire().await.unwrap().forget();
                gauge!("batcher_size").increment(1);
                miss.insert_entry(value);
            }
        };
        if priority {
            self.priority_flush.store(true, Ordering::Release);
        }
        self.notifier.notify_one();
    }

    pub async fn consume<F, Fut, O>(&self, max: usize, consumer: F) -> O
    where
        F: FnOnce(HashMap<Counter, Arc<CachedCounterValue>>) -> Fut,
        Fut: Future<Output = O>,
    {
        let mut ready = self.batch_ready(max);
        loop {
            if ready {
                let mut batch = Vec::with_capacity(max);
                batch.extend(
                    self.updates
                        .iter()
                        .filter(|entry| entry.value().requires_fast_flush(&self.interval))
                        .take(max)
                        .map(|e| e.key().clone()),
                );
                if let Some(remaining) = max.checked_sub(batch.len()) {
                    batch.extend(self.updates.iter().take(remaining).map(|e| e.key().clone()));
                }
                let mut result = HashMap::new();
                for counter in &batch {
                    let value = self.updates.get(counter).unwrap().clone();
                    result.insert(counter.clone(), value);
                }
                histogram!("batcher_flush_size").record(result.len() as f64);
                let result = consumer(result).await;
                batch.iter().for_each(|counter| {
                    let prev = self
                        .updates
                        .remove_if(counter, |_, v| v.no_pending_writes());
                    if prev.is_some() {
                        self.limiter.add_permits(1);
                        gauge!("batcher_size").decrement(1);
                    }
                });
                return result;
            } else {
                ready = select! {
                    _ = self.notifier.notified() => self.batch_ready(max),
                    _ = tokio::time::sleep(self.interval) => true,
                }
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.updates.is_empty()
    }

    fn batch_ready(&self, size: usize) -> bool {
        self.updates.len() >= size
            || self
                .priority_flush
                .compare_exchange(true, false, Ordering::Release, Ordering::Acquire)
                .is_ok()
    }
}

impl Default for Batcher {
    fn default() -> Self {
        Self::new(Duration::from_millis(100), DEFAULT_MAX_CACHED_COUNTERS)
    }
}

pub struct CountersCache {
    cache: Cache<Counter, Arc<CachedCounterValue>>,
    batcher: Batcher,
}

impl CountersCache {
    pub fn get(&self, counter: &Counter) -> Option<Arc<CachedCounterValue>> {
        let option = self.cache.get(counter);
        if option.is_none() {
            let from_queue = self.batcher.updates.get(counter);
            if let Some(entry) = from_queue {
                gauge!("cache_size").increment(1);
                self.cache.insert(counter.clone(), entry.value().clone());
                return Some(entry.value().clone());
            }
        }
        option
    }

    pub fn batcher(&self) -> &Batcher {
        &self.batcher
    }

    pub fn apply_remote_delta(
        &self,
        counter: Counter,
        redis_val: u64,
        remote_deltas: u64,
        redis_expiry: i64,
    ) -> Arc<CachedCounterValue> {
        if redis_expiry > 0 {
            let expiry_ts = SystemTime::UNIX_EPOCH + Duration::from_millis(redis_expiry as u64);
            if expiry_ts > SystemTime::now() {
                let mut from_cache = true;
                let cached = self.cache.get_with(counter.clone(), || {
                    gauge!("cache_size").increment(1);
                    from_cache = false;
                    if let Some(entry) = self.batcher.updates.get(&counter) {
                        let cached_value = entry.value();
                        cached_value.add_from_authority(remote_deltas, expiry_ts);
                        cached_value.clone()
                    } else {
                        Arc::new(CachedCounterValue::from_authority(&counter, redis_val))
                    }
                });
                if from_cache {
                    cached.add_from_authority(remote_deltas, expiry_ts);
                }
                return cached;
            }
        }
        Arc::new(CachedCounterValue::load_from_authority_asap(
            &counter, redis_val,
        ))
    }

    pub async fn increase_by(&self, counter: &Counter, delta: u64) {
        let val = self.cache.get_with_by_ref(counter, || {
            gauge!("cache_size").increment(1);
            if let Some(entry) = self.batcher.updates.get(counter) {
                entry.value().clone()
            } else {
                Arc::new(CachedCounterValue::load_from_authority_asap(counter, 0))
            }
        });
        val.delta(counter, delta);
        self.batcher.add(counter.clone(), val.clone()).await;
    }
}

pub struct CountersCacheBuilder {
    max_cached_counters: usize,
}

impl CountersCacheBuilder {
    pub fn new() -> Self {
        Self {
            max_cached_counters: DEFAULT_MAX_CACHED_COUNTERS,
        }
    }

    pub fn max_cached_counters(mut self, max_cached_counters: usize) -> Self {
        self.max_cached_counters = max_cached_counters;
        self
    }

    fn eviction_listener(
        _key: Arc<Counter>,
        value: Arc<CachedCounterValue>,
        _removal_cause: RemovalCause,
    ) {
        gauge!("cache_size").decrement(1);
        if value.no_pending_writes().not() {
            counter!("evicted_pending_writes").increment(1);
        }
    }

    pub fn build(&self, period: Duration) -> CountersCache {
        CountersCache {
            cache: Cache::builder()
                .max_capacity(self.max_cached_counters as u64)
                .eviction_listener(Self::eviction_listener)
                .build(),
            batcher: Batcher::new(period, self.max_cached_counters),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::ops::Add;
    use std::time::UNIX_EPOCH;

    use crate::limit::Limit;

    use super::*;

    mod cached_counter_value {
        use std::ops::{Add, Not};
        use std::time::{Duration, SystemTime};

        use crate::storage::redis::counters_cache::tests::test_counter;
        use crate::storage::redis::counters_cache::CachedCounterValue;

        #[test]
        fn records_pending_writes() {
            let counter = test_counter(10, None);
            let value = CachedCounterValue::from_authority(&counter, 0);
            assert_eq!(value.pending_writes(), Ok(0));
            value.delta(&counter, 5);
            assert_eq!(value.pending_writes(), Ok(5));
        }

        #[test]
        fn consumes_pending_writes() {
            let counter = test_counter(10, None);
            let value = CachedCounterValue::from_authority(&counter, 0);
            value.delta(&counter, 5);
            assert_eq!(value.pending_writes(), Ok(5));
            assert_eq!(value.pending_writes(), Ok(0));
        }

        #[test]
        fn no_pending_writes() {
            let counter = test_counter(10, None);
            let value = CachedCounterValue::from_authority(&counter, 0);
            value.delta(&counter, 5);
            assert!(value.no_pending_writes().not());
            assert!(value.pending_writes().is_ok());
            assert!(value.no_pending_writes());
        }

        #[test]
        fn adding_from_auth_not_affecting_pending_writes() {
            let counter = test_counter(10, None);
            let value = CachedCounterValue::from_authority(&counter, 0);
            value.delta(&counter, 5);
            assert!(value.no_pending_writes().not());
            value.add_from_authority(6, SystemTime::now().add(Duration::from_secs(1)));
            assert!(value.no_pending_writes().not());
            assert_eq!(value.pending_writes(), Ok(5));
        }

        #[test]
        fn from_authority_no_need_to_flush() {
            let counter = test_counter(10, None);
            let value = CachedCounterValue::from_authority(&counter, 0);
            assert!(value.requires_fast_flush(&Duration::from_secs(30)).not());
        }

        #[test]
        fn from_authority_needs_to_flush_within_ttl() {
            let counter = test_counter(10, None);
            let value = CachedCounterValue::from_authority(&counter, 0);
            assert!(value.requires_fast_flush(&Duration::from_secs(90)));
        }

        #[test]
        fn fake_needs_to_flush_within_ttl() {
            let counter = test_counter(10, None);
            let value = CachedCounterValue::load_from_authority_asap(&counter, 0);
            assert!(value.requires_fast_flush(&Duration::from_secs(30)));
        }

        #[test]
        fn delegates_to_underlying_value() {
            let hits = 4;

            let counter = test_counter(10, None);
            let value = CachedCounterValue::from_authority(&counter, 0);
            value.delta(&counter, hits);
            assert!(value.to_next_window() > Duration::from_millis(59999));
            assert_eq!(value.hits(&counter), hits);
            let remaining = counter.max_value() - hits;
            assert_eq!(value.remaining(&counter), remaining);
            assert!(value.is_limited(&counter, 1).not());
            assert!(value.is_limited(&counter, remaining).not());
            assert!(value.is_limited(&counter, remaining + 1));
        }
    }

    mod batcher {
        use std::sync::Arc;
        use std::time::{Duration, SystemTime};

        use crate::storage::redis::counters_cache::tests::test_counter;
        use crate::storage::redis::counters_cache::{Batcher, CachedCounterValue};
        use crate::storage::redis::DEFAULT_MAX_CACHED_COUNTERS;

        #[tokio::test]
        async fn consume_waits_when_empty() {
            let duration = Duration::from_millis(100);
            let batcher = Batcher::new(duration, DEFAULT_MAX_CACHED_COUNTERS);
            let start = SystemTime::now();
            batcher
                .consume(2, |items| {
                    assert!(items.is_empty());
                    assert!(SystemTime::now().duration_since(start).unwrap() >= duration);
                    async {}
                })
                .await;
        }

        #[tokio::test]
        async fn consume_waits_when_batch_not_filled() {
            let duration = Duration::from_millis(100);
            let batcher = Arc::new(Batcher::new(duration, DEFAULT_MAX_CACHED_COUNTERS));
            let start = SystemTime::now();
            {
                let batcher = Arc::clone(&batcher);
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(40)).await;
                    let counter = test_counter(6, None);
                    let arc = Arc::new(CachedCounterValue::from_authority(&counter, 0));
                    batcher.add(counter, arc).await;
                });
            }
            batcher
                .consume(2, |items| {
                    assert_eq!(items.len(), 1);
                    assert!(
                        SystemTime::now().duration_since(start).unwrap()
                            >= Duration::from_millis(100)
                    );
                    async {}
                })
                .await;
        }

        #[tokio::test]
        async fn consume_waits_until_batch_is_filled() {
            let duration = Duration::from_millis(100);
            let batcher = Arc::new(Batcher::new(duration, DEFAULT_MAX_CACHED_COUNTERS));
            let start = SystemTime::now();
            {
                let batcher = Arc::clone(&batcher);
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(40)).await;
                    let counter = test_counter(6, None);
                    let arc = Arc::new(CachedCounterValue::from_authority(&counter, 0));
                    batcher.add(counter, arc).await;
                });
            }
            batcher
                .consume(1, |items| {
                    assert_eq!(items.len(), 1);
                    let wait_period = SystemTime::now().duration_since(start).unwrap();
                    assert!(wait_period >= Duration::from_millis(40));
                    assert!(wait_period < Duration::from_millis(50));
                    async {}
                })
                .await;
        }

        #[tokio::test]
        async fn consume_immediately_when_batch_is_filled() {
            let duration = Duration::from_millis(100);
            let batcher = Arc::new(Batcher::new(duration, DEFAULT_MAX_CACHED_COUNTERS));
            let start = SystemTime::now();
            {
                let counter = test_counter(6, None);
                let arc = Arc::new(CachedCounterValue::from_authority(&counter, 0));
                batcher.add(counter, arc).await;
            }
            batcher
                .consume(1, |items| {
                    assert_eq!(items.len(), 1);
                    assert!(
                        SystemTime::now().duration_since(start).unwrap() < Duration::from_millis(5)
                    );
                    async {}
                })
                .await;
        }

        #[tokio::test]
        async fn consume_triggers_on_fast_flush() {
            let duration = Duration::from_millis(100);
            let batcher = Arc::new(Batcher::new(duration, DEFAULT_MAX_CACHED_COUNTERS));
            let start = SystemTime::now();
            {
                let batcher = Arc::clone(&batcher);
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(40)).await;
                    let counter = test_counter(6, None);
                    let arc = Arc::new(CachedCounterValue::load_from_authority_asap(&counter, 0));
                    batcher.add(counter, arc).await;
                });
            }
            batcher
                .consume(2, |items| {
                    assert_eq!(items.len(), 1);
                    let wait_period = SystemTime::now().duration_since(start).unwrap();
                    assert!(wait_period >= Duration::from_millis(40));
                    assert!(wait_period < Duration::from_millis(50));
                    async {}
                })
                .await;
        }
    }

    #[test]
    fn get_existing_counter() {
        let counter = test_counter(10, None);

        let cache = CountersCacheBuilder::new().build(Duration::default());
        cache.apply_remote_delta(
            counter.clone(),
            10,
            0,
            SystemTime::now()
                .add(Duration::from_secs(1))
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_micros() as i64,
        );

        assert!(cache.get(&counter).is_some());
    }

    #[test]
    fn get_non_existing_counter() {
        let counter = test_counter(10, None);

        let cache = CountersCacheBuilder::new().build(Duration::default());

        assert!(cache.get(&counter).is_none());
    }

    #[test]
    fn insert_saves_the_given_value_when_is_some() {
        let max_val = 10;
        let current_value = max_val / 2;
        let counter = test_counter(max_val, None);

        let cache = CountersCacheBuilder::new().build(Duration::default());
        cache.apply_remote_delta(
            counter.clone(),
            current_value,
            0,
            SystemTime::now()
                .add(Duration::from_secs(1))
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_micros() as i64,
        );

        assert_eq!(
            cache.get(&counter).map(|e| e.hits(&counter)).unwrap(),
            current_value
        );
    }

    #[tokio::test]
    async fn increase_by() {
        let current_val = 10;
        let increase_by = 8;
        let counter = test_counter(current_val, None);

        let cache = CountersCacheBuilder::new().build(Duration::default());
        cache.apply_remote_delta(
            counter.clone(),
            current_val,
            0,
            SystemTime::now()
                .add(Duration::from_secs(1))
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_micros() as i64,
        );
        cache.increase_by(&counter, increase_by).await;

        assert_eq!(
            cache.get(&counter).map(|e| e.hits(&counter)).unwrap(),
            (current_val + increase_by)
        );
    }

    fn test_counter(max_val: u64, other_values: Option<HashMap<String, String>>) -> Counter {
        let mut values = HashMap::new();
        values.insert("app_id".to_string(), "1".to_string());
        if let Some(overrides) = other_values {
            values.extend(overrides);
        }
        Counter::new(
            Limit::new(
                "test_namespace",
                max_val,
                60,
                vec!["req.method == 'POST'"],
                vec!["app_id"],
            ),
            values,
        )
    }
}
