use crate::counter::Counter;
use crate::storage::atomic_expiring_value::{AtomicExpiringValue, AtomicExpiryTime};
use crate::storage::redis::{
    DEFAULT_MAX_CACHED_COUNTERS, DEFAULT_MAX_TTL_CACHED_COUNTERS_SEC,
    DEFAULT_TTL_RATIO_CACHED_COUNTERS,
};
use dashmap::mapref::entry::Entry;
use dashmap::DashMap;
use moka::sync::Cache;
use std::collections::HashMap;
use std::future::Future;
use std::ops::Not;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::select;
use tokio::sync::Notify;

#[derive(Debug)]
pub struct CachedCounterValue {
    value: AtomicExpiringValue,
    initial_value: AtomicI64,
    expiry: AtomicExpiryTime,
    from_authority: AtomicBool,
}

impl CachedCounterValue {
    pub fn from_authority(counter: &Counter, value: i64, ttl: Duration) -> Self {
        let now = SystemTime::now();
        Self {
            value: AtomicExpiringValue::new(value, now + Duration::from_secs(counter.seconds())),
            initial_value: AtomicI64::new(value),
            expiry: AtomicExpiryTime::from_now(ttl),
            from_authority: AtomicBool::new(true),
        }
    }

    pub fn load_from_authority_asap(counter: &Counter, temp_value: i64) -> Self {
        let now = SystemTime::now();
        Self {
            value: AtomicExpiringValue::new(
                temp_value,
                now + Duration::from_secs(counter.seconds()),
            ),
            initial_value: AtomicI64::new(0),
            expiry: AtomicExpiryTime::from_now(Duration::from_secs(counter.seconds())),
            from_authority: AtomicBool::new(false),
        }
    }

    pub fn expired_at(&self, now: SystemTime) -> bool {
        self.expiry.expired_at(now)
    }

    pub fn set_from_authority(&self, counter: &Counter, value: i64, expiry: Duration) {
        let time_window = Duration::from_secs(counter.seconds());
        self.initial_value.store(value, Ordering::SeqCst);
        self.value.set(value, time_window);
        self.expiry.update(expiry);
        self.from_authority.store(true, Ordering::Release);
    }

    pub fn delta(&self, counter: &Counter, delta: i64) -> i64 {
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

    pub fn pending_writes(&self) -> Result<i64, ()> {
        let start = self.initial_value.load(Ordering::SeqCst);
        let value = self.value.value_at(SystemTime::now());
        let offset = if start == 0 {
            value
        } else {
            let writes = value - start;
            if writes >= 0 {
                writes
            } else {
                // self.value expired, is now less than the writes of the previous window
                // which have not yet been reset... it'll be 0, so treat it as such.
                value
            }
        };
        match self
            .initial_value
            .compare_exchange(start, value, Ordering::SeqCst, Ordering::SeqCst)
        {
            Ok(_) => Ok(offset),
            Err(newer) => {
                if newer == 0 {
                    // We got reset because of expiry, this fresh value can wait the next iteration
                    Ok(0)
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

    pub fn hits(&self, _: &Counter) -> i64 {
        self.value.value_at(SystemTime::now())
    }

    pub fn remaining(&self, counter: &Counter) -> i64 {
        counter.max_value() - self.hits(counter)
    }

    pub fn is_limited(&self, counter: &Counter, delta: i64) -> bool {
        self.hits(counter) as i128 + delta as i128 > counter.max_value() as i128
    }

    pub fn to_next_window(&self) -> Duration {
        self.value.ttl()
    }

    pub fn requires_fast_flush(&self, within: &Duration) -> bool {
        self.from_authority.load(Ordering::Acquire).not()
            || self.expired_at(SystemTime::now())
            || &self.value.ttl() <= within
    }
}

pub struct Batcher {
    updates: DashMap<Counter, Arc<CachedCounterValue>>,
    notifier: Notify,
    interval: Duration,
    priority_flush: AtomicBool,
}

impl Batcher {
    fn new(period: Duration) -> Self {
        Self {
            updates: Default::default(),
            notifier: Default::default(),
            interval: period,
            priority_flush: AtomicBool::new(false),
        }
    }

    pub fn add(&self, counter: Counter, value: Arc<CachedCounterValue>) {
        let priority = value.requires_fast_flush(&self.interval);
        match self.updates.entry(counter.clone()) {
            Entry::Occupied(needs_merge) => {
                let arc = needs_merge.get();
                if !Arc::ptr_eq(arc, &value) {
                    arc.delta(&counter, value.pending_writes().unwrap());
                }
            }
            Entry::Vacant(miss) => {
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
                let result = consumer(result).await;
                batch.iter().for_each(|counter| {
                    self.updates
                        .remove_if(counter, |_, v| v.no_pending_writes());
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
        Self::new(Duration::from_millis(100))
    }
}

pub struct CountersCache {
    max_ttl_cached_counters: Duration,
    pub ttl_ratio_cached_counters: u64,
    cache: Cache<Counter, Arc<CachedCounterValue>>,
    batcher: Batcher,
}

impl CountersCache {
    pub fn get(&self, counter: &Counter) -> Option<Arc<CachedCounterValue>> {
        let option = self.cache.get(counter);
        if option.is_none() {
            let from_queue = self.batcher.updates.get(counter);
            if let Some(entry) = from_queue {
                self.cache.insert(counter.clone(), entry.value().clone());
                return Some(entry.value().clone());
            }
        }
        option
    }

    pub fn batcher(&self) -> &Batcher {
        &self.batcher
    }

    pub fn insert(
        &self,
        counter: Counter,
        redis_val: Option<i64>,
        redis_ttl_ms: i64,
        ttl_margin: Duration,
        now: SystemTime,
    ) -> Arc<CachedCounterValue> {
        let counter_val = redis_val.unwrap_or(0);
        let cache_ttl = self.ttl_from_redis_ttl(
            redis_ttl_ms,
            counter.seconds(),
            counter_val,
            counter.max_value(),
        );
        if let Some(ttl) = cache_ttl.checked_sub(ttl_margin) {
            if ttl > Duration::ZERO {
                let previous = self.cache.get_with(counter.clone(), || {
                    if let Some(entry) = self.batcher.updates.get(&counter) {
                        entry.value().clone()
                    } else {
                        Arc::new(CachedCounterValue::from_authority(
                            &counter,
                            counter_val,
                            ttl,
                        ))
                    }
                });
                if previous.expired_at(now) || previous.value.value() < counter_val {
                    previous.set_from_authority(&counter, counter_val, ttl);
                }
                return previous;
            }
        }
        Arc::new(CachedCounterValue::load_from_authority_asap(
            &counter,
            counter_val,
        ))
    }

    pub fn increase_by(&self, counter: &Counter, delta: i64) {
        let val = self.cache.get_with_by_ref(counter, || {
            if let Some(entry) = self.batcher.updates.get(counter) {
                entry.value().clone()
            } else {
                Arc::new(CachedCounterValue::load_from_authority_asap(counter, 0))
            }
        });
        val.delta(counter, delta);
        self.batcher.add(counter.clone(), val.clone());
    }

    fn ttl_from_redis_ttl(
        &self,
        redis_ttl_ms: i64,
        counter_seconds: u64,
        counter_val: i64,
        counter_max: i64,
    ) -> Duration {
        // Redis returns -2 when the key does not exist. Ref:
        // https://redis.io/commands/ttl
        // This function returns a ttl of the given counter seconds in this
        // case.

        let counter_ttl = if redis_ttl_ms >= 0 {
            Duration::from_millis(redis_ttl_ms as u64)
        } else {
            Duration::from_secs(counter_seconds)
        };

        // If a counter is already at counter_max, we can cache it for as long as its TTL
        // is in Redis. This does not depend on the requests received by other
        // instances of Limitador. No matter what they do, we know that the
        // counter is not going to recover its quota until it expires in Redis.
        if counter_val >= counter_max {
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
            Duration::from_millis(counter_ttl.as_millis() as u64 / self.ttl_ratio_cached_counters);

        if res > self.max_ttl_cached_counters {
            res = self.max_ttl_cached_counters;
        }

        res
    }
}

pub struct CountersCacheBuilder {
    max_cached_counters: usize,
    max_ttl_cached_counters: Duration,
    ttl_ratio_cached_counters: u64,
}

impl CountersCacheBuilder {
    pub fn new() -> Self {
        Self {
            max_cached_counters: DEFAULT_MAX_CACHED_COUNTERS,
            max_ttl_cached_counters: Duration::from_secs(DEFAULT_MAX_TTL_CACHED_COUNTERS_SEC),
            ttl_ratio_cached_counters: DEFAULT_TTL_RATIO_CACHED_COUNTERS,
        }
    }

    pub fn max_cached_counters(mut self, max_cached_counters: usize) -> Self {
        self.max_cached_counters = max_cached_counters;
        self
    }

    pub fn max_ttl_cached_counter(mut self, max_ttl_cached_counter: Duration) -> Self {
        self.max_ttl_cached_counters = max_ttl_cached_counter;
        self
    }

    pub fn ttl_ratio_cached_counter(mut self, ttl_ratio_cached_counter: u64) -> Self {
        self.ttl_ratio_cached_counters = ttl_ratio_cached_counter;
        self
    }

    pub fn build(&self, period: Duration) -> CountersCache {
        CountersCache {
            max_ttl_cached_counters: self.max_ttl_cached_counters,
            ttl_ratio_cached_counters: self.ttl_ratio_cached_counters,
            cache: Cache::new(self.max_cached_counters as u64),
            batcher: Batcher::new(period),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limit::Limit;
    use std::collections::HashMap;

    mod cached_counter_value {
        use crate::storage::redis::counters_cache::tests::test_counter;
        use crate::storage::redis::counters_cache::CachedCounterValue;
        use std::ops::Not;
        use std::time::{Duration, SystemTime};

        #[test]
        fn records_pending_writes() {
            let counter = test_counter(10, None);
            let value = CachedCounterValue::from_authority(&counter, 0, Duration::from_secs(1));
            assert_eq!(value.pending_writes(), Ok(0));
            value.delta(&counter, 5);
            assert_eq!(value.pending_writes(), Ok(5));
        }

        #[test]
        fn consumes_pending_writes() {
            let counter = test_counter(10, None);
            let value = CachedCounterValue::from_authority(&counter, 0, Duration::from_secs(1));
            value.delta(&counter, 5);
            assert_eq!(value.pending_writes(), Ok(5));
            assert_eq!(value.pending_writes(), Ok(0));
        }

        #[test]
        fn no_pending_writes() {
            let counter = test_counter(10, None);
            let value = CachedCounterValue::from_authority(&counter, 0, Duration::from_secs(1));
            value.delta(&counter, 5);
            assert!(value.no_pending_writes().not());
            assert!(value.pending_writes().is_ok());
            assert!(value.no_pending_writes());
        }

        #[test]
        fn setting_from_auth_resets_pending_writes() {
            let counter = test_counter(10, None);
            let value = CachedCounterValue::from_authority(&counter, 0, Duration::from_secs(1));
            value.delta(&counter, 5);
            assert!(value.no_pending_writes().not());
            value.set_from_authority(&counter, 6, Duration::from_secs(1));
            assert!(value.no_pending_writes());
            assert_eq!(value.pending_writes(), Ok(0));
        }

        #[test]
        fn from_authority_no_need_to_flush() {
            let counter = test_counter(10, None);
            let value = CachedCounterValue::from_authority(&counter, 0, Duration::from_secs(10));
            assert!(value.requires_fast_flush(&Duration::from_secs(30)).not());
        }

        #[test]
        fn from_authority_needs_to_flush_within_ttl() {
            let counter = test_counter(10, None);
            let value = CachedCounterValue::from_authority(&counter, 0, Duration::from_secs(1));
            assert!(value.requires_fast_flush(&Duration::from_secs(90)));
        }

        #[test]
        fn fake_needs_to_flush_within_ttl() {
            let counter = test_counter(10, None);
            let value = CachedCounterValue::load_from_authority_asap(&counter, 0);
            assert!(value.requires_fast_flush(&Duration::from_secs(30)));
        }

        #[test]
        fn expiry_of_cached_entry() {
            let counter = test_counter(10, None);
            let cache_entry_ttl = Duration::from_secs(1);
            let value = CachedCounterValue::from_authority(&counter, 0, cache_entry_ttl);
            let now = SystemTime::now();
            assert!(value.expired_at(now).not());
            assert!(value.expired_at(now + cache_entry_ttl));
        }

        #[test]
        fn delegates_to_underlying_value() {
            let hits = 4;

            let counter = test_counter(10, None);
            let value = CachedCounterValue::from_authority(&counter, 0, Duration::from_secs(1));
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
        use crate::storage::redis::counters_cache::tests::test_counter;
        use crate::storage::redis::counters_cache::{Batcher, CachedCounterValue};
        use std::sync::Arc;
        use std::time::{Duration, SystemTime};

        #[tokio::test]
        async fn consume_waits_when_empty() {
            let duration = Duration::from_millis(100);
            let batcher = Batcher::new(duration);
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
            let batcher = Arc::new(Batcher::new(duration));
            let start = SystemTime::now();
            {
                let batcher = Arc::clone(&batcher);
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(40)).await;
                    let counter = test_counter(6, None);
                    let arc = Arc::new(CachedCounterValue::from_authority(
                        &counter,
                        0,
                        Duration::from_secs(1),
                    ));
                    batcher.add(counter, arc);
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
            let batcher = Arc::new(Batcher::new(duration));
            let start = SystemTime::now();
            {
                let batcher = Arc::clone(&batcher);
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(40)).await;
                    let counter = test_counter(6, None);
                    let arc = Arc::new(CachedCounterValue::from_authority(
                        &counter,
                        0,
                        Duration::from_secs(1),
                    ));
                    batcher.add(counter, arc);
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
            let batcher = Arc::new(Batcher::new(duration));
            let start = SystemTime::now();
            {
                let counter = test_counter(6, None);
                let arc = Arc::new(CachedCounterValue::from_authority(
                    &counter,
                    0,
                    Duration::from_secs(1),
                ));
                batcher.add(counter, arc);
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
            let batcher = Arc::new(Batcher::new(duration));
            let start = SystemTime::now();
            {
                let batcher = Arc::clone(&batcher);
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(40)).await;
                    let counter = test_counter(6, None);
                    let arc = Arc::new(CachedCounterValue::load_from_authority_asap(&counter, 0));
                    batcher.add(counter, arc);
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
        cache.insert(
            counter.clone(),
            Some(10),
            10,
            Duration::from_secs(0),
            SystemTime::now(),
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
        cache.insert(
            counter.clone(),
            Some(current_value),
            10,
            Duration::from_secs(0),
            SystemTime::now(),
        );

        assert_eq!(
            cache.get(&counter).map(|e| e.hits(&counter)).unwrap(),
            current_value
        );
    }

    #[test]
    fn insert_saves_zero_when_redis_val_is_none() {
        let max_val = 10;
        let counter = test_counter(max_val, None);

        let cache = CountersCacheBuilder::new().build(Duration::default());
        cache.insert(
            counter.clone(),
            None,
            10,
            Duration::from_secs(0),
            SystemTime::now(),
        );

        assert_eq!(cache.get(&counter).map(|e| e.hits(&counter)).unwrap(), 0);
    }

    #[test]
    fn increase_by() {
        let current_val = 10;
        let increase_by = 8;
        let counter = test_counter(current_val, None);

        let cache = CountersCacheBuilder::new().build(Duration::default());
        cache.insert(
            counter.clone(),
            Some(current_val),
            10,
            Duration::from_secs(0),
            SystemTime::now(),
        );
        cache.increase_by(&counter, increase_by);

        assert_eq!(
            cache.get(&counter).map(|e| e.hits(&counter)).unwrap(),
            (current_val + increase_by)
        );
    }

    fn test_counter(max_val: i64, other_values: Option<HashMap<String, String>>) -> Counter {
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
