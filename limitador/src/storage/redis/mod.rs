use ::redis::RedisError;

mod batcher;
mod redis_async;
mod redis_cached;
mod redis_keys;
mod redis_sync;

use crate::storage::StorageErr;
pub use redis_async::AsyncRedisStorage;
pub use redis_cached::CachedRedisStorage;
pub use redis_sync::RedisStorage;

impl From<RedisError> for StorageErr {
    fn from(e: RedisError) -> Self {
        StorageErr { msg: e.to_string() }
    }
}
