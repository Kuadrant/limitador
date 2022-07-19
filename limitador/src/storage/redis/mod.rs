use ::redis::RedisError;

mod batcher;
mod counters_cache;
mod redis_async;
mod redis_cached;
mod redis_sync;
mod scripts;

pub const DEFAULT_FLUSHING_PERIOD_SEC: u64 = 1;
pub const DEFAULT_MAX_CACHED_COUNTERS: usize = 10000;
pub const DEFAULT_MAX_TTL_CACHED_COUNTERS_SEC: u64 = 5;
pub const DEFAULT_TTL_RATIO_CACHED_COUNTERS: u64 = 10;

use crate::storage::StorageErr;
pub use redis_async::AsyncRedisStorage;
pub use redis_cached::CachedRedisStorage;
pub use redis_cached::CachedRedisStorageBuilder;
pub use redis_sync::RedisStorage;

impl From<RedisError> for StorageErr {
    fn from(e: RedisError) -> Self {
        Self { msg: e.to_string() }
    }
}

impl From<::r2d2::Error> for StorageErr {
    fn from(e: ::r2d2::Error) -> Self {
        Self { msg: e.to_string() }
    }
}
