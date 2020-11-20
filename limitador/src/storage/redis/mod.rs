use ::redis::RedisError;

mod batcher;
mod counters_cache;
mod redis_async;
mod redis_cached;
mod redis_keys;
mod redis_sync;
mod scripts;

use crate::storage::StorageErr;
pub use redis_async::AsyncRedisStorage;
pub use redis_cached::CachedRedisStorage;
pub use redis_sync::RedisStorage;

impl From<RedisError> for StorageErr {
    fn from(e: RedisError) -> Self {
        StorageErr { msg: e.to_string() }
    }
}

impl From<::r2d2::Error> for StorageErr {
    fn from(e: ::r2d2::Error) -> Self {
        StorageErr { msg: e.to_string() }
    }
}
