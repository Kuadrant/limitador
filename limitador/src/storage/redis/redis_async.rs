extern crate redis;

use self::redis::aio::ConnectionManager;
use self::redis::ConnectionInfo;
use crate::counter::Counter;
use crate::limit::Limit;
use crate::storage::keys::*;
use crate::storage::redis::is_limited;
use crate::storage::redis::scripts::{SCRIPT_UPDATE_COUNTER, VALUES_AND_TTLS};
use crate::storage::{AsyncCounterStorage, Authorization, StorageErr};
use async_trait::async_trait;
use redis::{AsyncCommands, RedisError};
use std::collections::HashSet;
use std::str::FromStr;
use std::time::Duration;

// Note: this implementation does not guarantee exact limits. Ensuring that we
// never go over the limits would hurt performance. This implementation
// sacrifices a bit of accuracy to be more performant.

// TODO: the code of this implementation is almost identical to the blocking
// one. The only exception is that the functions defined are "async" and all the
// calls to the client need to include ".await". We'll need to think about how
// to remove this duplication.

#[derive(Clone)]
pub struct AsyncRedisStorage {
    conn_manager: ConnectionManager,
}

#[async_trait]
impl AsyncCounterStorage for AsyncRedisStorage {
    async fn is_within_limits(&self, counter: &Counter, delta: i64) -> Result<bool, StorageErr> {
        let mut con = self.conn_manager.clone();

        match con
            .get::<Vec<u8>, Option<i64>>(key_for_counter(counter))
            .await?
        {
            Some(val) => Ok(val - delta >= 0),
            None => Ok(counter.max_value() - delta >= 0),
        }
    }

    async fn update_counter(&self, counter: &Counter, delta: i64) -> Result<(), StorageErr> {
        let mut con = self.conn_manager.clone();

        redis::Script::new(SCRIPT_UPDATE_COUNTER)
            .key(key_for_counter(counter))
            .key(key_for_counters_of_limit(counter.limit()))
            .arg(counter.max_value())
            .arg(counter.seconds())
            .arg(delta)
            .invoke_async::<_, _>(&mut con)
            .await?;

        Ok(())
    }

    async fn check_and_update(
        &self,
        counters: &mut Vec<Counter>,
        delta: i64,
        load_counters: bool,
    ) -> Result<Authorization, StorageErr> {
        let mut con = self.conn_manager.clone();
        let counter_keys: Vec<Vec<u8>> = counters.iter().map(key_for_counter).collect();

        if load_counters {
            let script = redis::Script::new(VALUES_AND_TTLS);
            let mut script_invocation = script.prepare_invoke();

            for counter_key in &counter_keys {
                script_invocation.key(counter_key);
            }

            let script_res: Vec<Option<i64>> = script_invocation.invoke_async(&mut con).await?;
            if let Some(res) = is_limited(counters, delta, script_res) {
                return Ok(res);
            }
        } else {
            let counter_vals: Vec<Option<i64>> = redis::cmd("MGET")
                .arg(counter_keys.clone())
                .query_async(&mut con)
                .await?;

            for (i, counter) in counters.iter().enumerate() {
                let remaining = counter_vals[i].unwrap_or(counter.max_value()) - delta;
                if remaining < 0 {
                    return Ok(Authorization::Limited(
                        counter.limit().name().map(|n| n.to_owned()),
                    ));
                }
            }
        }

        // TODO: this can be optimized by using pipelines with multiple updates
        for (counter_idx, key) in counter_keys.into_iter().enumerate() {
            let counter = &counters[counter_idx];
            redis::Script::new(SCRIPT_UPDATE_COUNTER)
                .key(key)
                .key(key_for_counters_of_limit(counter.limit()))
                .arg(counter.max_value())
                .arg(counter.seconds())
                .arg(delta)
                .invoke_async::<_, _>(&mut con)
                .await?;
        }

        Ok(Authorization::Ok)
    }

    async fn get_counters(&self, limits: HashSet<Limit>) -> Result<HashSet<Counter>, StorageErr> {
        let mut res = HashSet::new();

        let mut con = self.conn_manager.clone();

        for limit in limits {
            let counter_keys = con
                .smembers::<Vec<u8>, HashSet<Vec<u8>>>(key_for_counters_of_limit(&limit))
                .await?;

            for counter_key in counter_keys {
                let mut counter: Counter = counter_from_counter_key(&counter_key, &limit);

                // If the key does not exist, it means that the counter expired,
                // so we don't have to return it.
                // TODO: we should delete the counter from the set of counters
                // associated with the limit taking into account that we should
                // do the "get" + "delete if none" atomically.
                // This does not cause any bugs, but consumes memory
                // unnecessarily.
                if let Some(val) = con.get::<Vec<u8>, Option<i64>>(counter_key.clone()).await? {
                    counter.set_remaining(val);
                    let ttl = con.ttl(&counter_key).await?;
                    counter.set_expires_in(Duration::from_secs(ttl));

                    res.insert(counter);
                }
            }
        }

        Ok(res)
    }

    async fn delete_counters(&self, limits: HashSet<Limit>) -> Result<(), StorageErr> {
        for limit in limits {
            self.delete_counters_associated_with_limit(&limit).await?;
        }
        Ok(())
    }

    async fn clear(&self) -> Result<(), StorageErr> {
        let mut con = self.conn_manager.clone();
        redis::cmd("FLUSHDB").query_async(&mut con).await?;
        Ok(())
    }
}

impl AsyncRedisStorage {
    pub async fn new(redis_url: &str) -> Result<Self, RedisError> {
        let info = ConnectionInfo::from_str(redis_url)?;
        Ok(Self {
            conn_manager: ConnectionManager::new(
                redis::Client::open(info)
                    .expect("This couldn't fail in the past, yet now it did somehow!"),
            )
            .await?,
        })
    }

    pub fn new_with_conn_manager(conn_manager: ConnectionManager) -> Self {
        Self { conn_manager }
    }

    async fn delete_counters_associated_with_limit(&self, limit: &Limit) -> Result<(), StorageErr> {
        let mut con = self.conn_manager.clone();

        let counter_keys = con
            .smembers::<Vec<u8>, HashSet<Vec<u8>>>(key_for_counters_of_limit(limit))
            .await?;

        for counter_key in counter_keys {
            con.del(counter_key).await?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::storage::redis::AsyncRedisStorage;
    use redis::ErrorKind;

    #[tokio::test]
    async fn errs_on_bad_url() {
        let result = AsyncRedisStorage::new("cassandra://127.0.0.1:6379").await;
        assert!(result.is_err());
        assert_eq!(result.err().unwrap().kind(), ErrorKind::InvalidClientConfig);
    }

    #[tokio::test]
    async fn errs_on_connection_issue() {
        let result = AsyncRedisStorage::new("redis://127.0.0.1:21").await;
        assert!(result.is_err());
        let error = result.err().unwrap();
        assert_eq!(error.kind(), ErrorKind::IoError);
        assert!(error.is_connection_refusal())
    }
}
