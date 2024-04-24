use ::redis::RedisError;
use std::time::Duration;

mod counters_cache;
mod redis_async;
mod redis_cached;
mod redis_sync;
mod scripts;

pub const DEFAULT_FLUSHING_PERIOD_SEC: u64 = 0;
pub const DEFAULT_MAX_CACHED_COUNTERS: usize = 10000;
pub const DEFAULT_MAX_TTL_CACHED_COUNTERS_SEC: u64 = 5;
pub const DEFAULT_TTL_RATIO_CACHED_COUNTERS: u64 = 10;
pub const DEFAULT_RESPONSE_TIMEOUT_MS: u64 = 350;

use crate::counter::Counter;
use crate::storage::{Authorization, StorageErr};
pub use redis_async::AsyncRedisStorage;
pub use redis_cached::CachedRedisStorage;
pub use redis_cached::CachedRedisStorageBuilder;
pub use redis_sync::RedisStorage;

impl From<RedisError> for StorageErr {
    fn from(e: RedisError) -> Self {
        Self {
            msg: e.to_string(),
            transient: e.is_timeout()
                || e.is_connection_dropped()
                || e.is_cluster_error()
                || e.is_connection_refusal(),
        }
    }
}

pub fn is_limited(
    counters: &mut [Counter],
    delta: i64,
    script_res: Vec<Option<i64>>,
) -> Option<Authorization> {
    let mut counter_vals: Vec<Option<i64>> = vec![];
    let mut counter_ttls_msecs: Vec<Option<i64>> = vec![];

    for val_ttl_pair in script_res.chunks(2) {
        counter_vals.push(val_ttl_pair[0]);
        counter_ttls_msecs.push(val_ttl_pair[1]);
    }

    let mut first_limited = None;
    for (i, counter) in counters.iter_mut().enumerate() {
        // remaining  = max - (curr_val + delta)
        let remaining = counter.max_value() - (counter_vals[i].unwrap_or(0) + delta);
        counter.set_remaining(remaining);
        let expires_in = counter_ttls_msecs[i]
            .map(|x| {
                if x >= 0 {
                    Duration::from_millis(x as u64)
                } else {
                    Duration::from_secs(counter.seconds())
                }
            })
            .unwrap_or(Duration::from_secs(counter.seconds()));

        counter.set_expires_in(expires_in);
        if first_limited.is_none() && remaining < 0 {
            first_limited = Some(Authorization::Limited(
                counter.limit().name().map(|n| n.to_owned()),
            ))
        }
    }
    first_limited
}
