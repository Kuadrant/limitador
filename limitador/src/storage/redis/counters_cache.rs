use crate::counter::Counter;
use std::time::Duration;
use ttl_cache::TtlCache;

const DEFAULT_MAX_CACHED_COUNTERS: usize = 10000;
const MAX_TTL_CACHED_COUNTER: Duration = Duration::from_secs(5);
const TTL_RATIO_CACHED_COUNTER: u64 = 10;

pub struct CountersCache {
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
