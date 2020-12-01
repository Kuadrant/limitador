use crate::counter::Counter;
use std::time::Duration;
use ttl_cache::TtlCache;

pub const DEFAULT_MAX_CACHED_COUNTERS: usize = 10000;
pub const DEFAULT_MAX_TTL_CACHED_COUNTERS: Duration = Duration::from_secs(5);
pub const DEFAULT_TTL_RATIO_CACHED_COUNTERS: u64 = 10;

pub struct CountersCache {
    max_ttl_cached_counters: Duration,
    ttl_ratio_cached_counters: u64,
    cache: TtlCache<Counter, i64>,
}

pub struct CountersCacheBuilder {
    max_cached_counters: usize,
    max_ttl_cached_counters: Duration,
    ttl_ratio_cached_counters: u64,
}

impl CountersCacheBuilder {
    pub fn new() -> CountersCacheBuilder {
        CountersCacheBuilder {
            max_cached_counters: DEFAULT_MAX_CACHED_COUNTERS,
            max_ttl_cached_counters: DEFAULT_MAX_TTL_CACHED_COUNTERS,
            ttl_ratio_cached_counters: DEFAULT_TTL_RATIO_CACHED_COUNTERS,
        }
    }

    pub fn max_cached_counters(mut self, max_cached_counters: usize) -> CountersCacheBuilder {
        self.max_cached_counters = max_cached_counters;
        self
    }

    pub fn max_ttl_cached_counter(
        mut self,
        max_ttl_cached_counter: Duration,
    ) -> CountersCacheBuilder {
        self.max_ttl_cached_counters = max_ttl_cached_counter;
        self
    }

    pub fn ttl_ratio_cached_counter(
        mut self,
        ttl_ratio_cached_counter: u64,
    ) -> CountersCacheBuilder {
        self.ttl_ratio_cached_counters = ttl_ratio_cached_counter;
        self
    }

    pub fn build(&self) -> CountersCache {
        CountersCache {
            max_ttl_cached_counters: self.max_ttl_cached_counters,
            ttl_ratio_cached_counters: self.ttl_ratio_cached_counters,
            cache: TtlCache::new(self.max_cached_counters),
        }
    }
}

impl CountersCache {
    pub fn get(&self, counter: &Counter) -> Option<i64> {
        match self.cache.get(counter) {
            Some(val) => Some(*val),
            None => None,
        }
    }

    pub fn insert(
        &mut self,
        counter: Counter,
        redis_val: Option<i64>,
        redis_ttl_ms: i64,
        ttl_margin: Duration,
    ) {
        let counter_val = Self::value_from_redis_val(redis_val, counter.max_value());
        let counter_ttl = self.ttl_from_redis_ttl(redis_ttl_ms, counter.seconds(), counter_val);
        if let Some(ttl) = counter_ttl.checked_sub(ttl_margin) {
            if ttl > Duration::from_secs(0) {
                self.cache.insert(counter, counter_val, ttl);
            }
        }
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

    fn ttl_from_redis_ttl(
        &self,
        redis_ttl_ms: i64,
        counter_seconds: u64,
        counter_val: i64,
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
            Duration::from_millis(counter_ttl.as_millis() as u64 / self.ttl_ratio_cached_counters);

        if res > self.max_ttl_cached_counters {
            res = self.max_ttl_cached_counters;
        }

        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limit::Limit;
    use std::collections::HashMap;

    #[test]
    fn get_existing_counter() {
        let mut values = HashMap::new();
        values.insert("app_id".to_string(), "1".to_string());
        let counter = Counter::new(
            Limit::new(
                "test_namespace",
                10,
                60,
                vec!["req.method == POST"],
                vec!["app_id"],
            ),
            values,
        );

        let mut cache = CountersCacheBuilder::new().build();
        cache.insert(counter.clone(), Some(10), 10, Duration::from_secs(0));

        assert!(cache.get(&counter).is_some());
    }

    #[test]
    fn get_non_existing_counter() {
        let mut values = HashMap::new();
        values.insert("app_id".to_string(), "1".to_string());
        let counter = Counter::new(
            Limit::new(
                "test_namespace",
                10,
                60,
                vec!["req.method == POST"],
                vec!["app_id"],
            ),
            values,
        );

        let cache = CountersCacheBuilder::new().build();

        assert!(cache.get(&counter).is_none());
    }

    #[test]
    fn insert_saves_the_given_value_when_is_some() {
        let max_val = 10;
        let current_value = max_val / 2;
        let mut values = HashMap::new();
        values.insert("app_id".to_string(), "1".to_string());
        let counter = Counter::new(
            Limit::new(
                "test_namespace",
                max_val,
                60,
                vec!["req.method == POST"],
                vec!["app_id"],
            ),
            values,
        );

        let mut cache = CountersCacheBuilder::new().build();
        cache.insert(
            counter.clone(),
            Some(current_value),
            10,
            Duration::from_secs(0),
        );

        assert_eq!(cache.get(&counter).unwrap(), current_value);
    }

    #[test]
    fn insert_saves_max_value_when_redis_val_is_none() {
        let max_val = 10;
        let mut values = HashMap::new();
        values.insert("app_id".to_string(), "1".to_string());
        let counter = Counter::new(
            Limit::new(
                "test_namespace",
                max_val,
                60,
                vec!["req.method == POST"],
                vec!["app_id"],
            ),
            values,
        );

        let mut cache = CountersCacheBuilder::new().build();
        cache.insert(counter.clone(), None, 10, Duration::from_secs(0));

        assert_eq!(cache.get(&counter).unwrap(), max_val);
    }

    #[test]
    fn decrease_by() {
        let current_val = 10;
        let decrease_by = 8;
        let mut values = HashMap::new();
        values.insert("app_id".to_string(), "1".to_string());
        let counter = Counter::new(
            Limit::new(
                "test_namespace",
                current_val,
                60,
                vec!["req.method == POST"],
                vec!["app_id"],
            ),
            values,
        );

        let mut cache = CountersCacheBuilder::new().build();
        cache.insert(
            counter.clone(),
            Some(current_val),
            10,
            Duration::from_secs(0),
        );
        cache.decrease_by(&counter, decrease_by);

        assert_eq!(cache.get(&counter).unwrap(), current_val - decrease_by);
    }
}
