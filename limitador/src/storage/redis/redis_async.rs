extern crate redis;

use crate::counter::Counter;
use crate::limit::Limit;
use crate::storage::redis::redis_keys::*;
use crate::storage::{AsyncStorage, StorageErr};
use async_trait::async_trait;
use redis::AsyncCommands;
use std::collections::HashSet;
use std::iter::FromIterator;

const DEFAULT_REDIS_URL: &str = "redis://127.0.0.1:6379";

// Note: this implementation does no guarantee exact limits. Ensuring that we
// never go over the limits would hurt performance. This implementation
// sacrifices a bit of accuracy to be more performant.

// TODO: the code of this implementation is almost identical to the blocking
// one. The only exception is that the functions defined are "async" and all the
// calls to the client need to include ".await". We'll need to think about how
// to remove this duplication.

#[derive(Debug, Clone)]
pub struct AsyncRedisStorage {
    client: redis::Client,
}

#[async_trait]
impl AsyncStorage for AsyncRedisStorage {
    async fn add_limit(&self, limit: &Limit) -> Result<(), StorageErr> {
        let mut con = self.client.get_async_connection().await?;

        let set_key = key_for_limits_of_namespace(limit.namespace());
        let serialized_limit = serde_json::to_string(limit).unwrap();

        con.sadd::<String, String, _>(set_key, serialized_limit)
            .await?;

        Ok(())
    }

    async fn get_limits(&self, namespace: &str) -> Result<HashSet<Limit>, StorageErr> {
        let mut con = self.client.get_async_connection().await?;

        let set_key = key_for_limits_of_namespace(namespace);

        let limits: HashSet<Limit> = HashSet::from_iter(
            con.smembers::<String, HashSet<String>>(set_key)
                .await?
                .iter()
                .map(|limit_json| serde_json::from_str(limit_json).unwrap()),
        );

        Ok(limits)
    }

    async fn delete_limit(&self, limit: &Limit) -> Result<(), StorageErr> {
        let mut con = self.client.get_async_connection().await?;

        self.delete_counters_associated_with_limit(limit).await?;
        con.del(key_for_counters_of_limit(&limit)).await?;

        let set_key = key_for_limits_of_namespace(limit.namespace());
        let serialized_limit = serde_json::to_string(limit).unwrap();

        con.srem(set_key, serialized_limit).await?;

        Ok(())
    }

    async fn delete_limits(&self, namespace: &str) -> Result<(), StorageErr> {
        let mut con = self.client.get_async_connection().await?;

        self.delete_counters_of_namespace(namespace).await?;

        for limit in self.get_limits(namespace).await? {
            con.del(key_for_counters_of_limit(&limit)).await?;
        }

        let set_key = key_for_limits_of_namespace(namespace);
        con.del(set_key).await?;

        Ok(())
    }

    async fn is_within_limits(&self, counter: &Counter, delta: i64) -> Result<bool, StorageErr> {
        let mut con = self.client.get_async_connection().await?;

        match con
            .get::<String, Option<i64>>(key_for_counter(counter))
            .await?
        {
            Some(val) => Ok(val - delta >= 0),
            None => Ok(counter.max_value() - delta >= 0),
        }
    }

    async fn update_counter(&self, counter: &Counter, delta: i64) -> Result<(), StorageErr> {
        let mut con = self.client.get_async_connection().await?;

        let counter_key = key_for_counter(counter);

        // 1) Sets the counter key if it does not exist.
        // 2) Decreases the value.
        // 3) Adds the counter to the set of counters associated with the limit.
        // We need a MULTI/EXEC (atomic pipeline) to ensure that the key will
        // not expire between the set and the incr.
        redis::pipe()
            .atomic()
            .cmd("SET")
            .arg(&counter_key)
            .arg(counter.max_value())
            .arg("EX")
            .arg(counter.seconds())
            .arg("NX")
            .incr::<&str, i64>(&counter_key, -delta)
            .sadd::<&str, &str>(&key_for_counters_of_limit(counter.limit()), &counter_key)
            .query_async(&mut con)
            .await?;

        Ok(())
    }

    async fn check_and_update(
        &self,
        counters: &HashSet<&Counter>,
        delta: i64,
    ) -> Result<bool, StorageErr> {
        let mut con = self.client.get_async_connection().await?;

        let counter_keys: Vec<String> = counters
            .iter()
            .map(|counter| key_for_counter(counter))
            .collect();

        let counter_vals: Vec<Option<i64>> = redis::cmd("MGET")
            .arg(counter_keys)
            .query_async(&mut con)
            .await?;

        for (i, counter) in counters.iter().enumerate() {
            match counter_vals[i] {
                Some(val) => {
                    if val - delta < 0 {
                        return Ok(false);
                    }
                }
                None => {
                    if counter.max_value() - delta < 0 {
                        return Ok(false);
                    }
                }
            }
        }

        // TODO: this can be optimized by using pipelines with multiple updates
        for counter in counters {
            self.update_counter(counter, delta).await?
        }

        Ok(true)
    }

    async fn get_counters(&self, namespace: &str) -> Result<HashSet<Counter>, StorageErr> {
        let mut res = HashSet::new();

        let mut con = self.client.get_async_connection().await?;

        for limit in self.get_limits(namespace).await? {
            let counter_keys = con
                .smembers::<String, HashSet<String>>(key_for_counters_of_limit(&limit))
                .await?;

            for counter_key in counter_keys {
                let mut counter: Counter = counter_from_counter_key(&counter_key);

                // If the key does not exist, it means that the counter expired,
                // so we don't have to return it.
                // TODO: we should delete the counter from the set of counters
                // associated with the limit taking into account that we should
                // do the "get" + "delete if none" atomically.
                // This does not cause any bugs, but consumes memory
                // unnecessarily.
                if let Some(val) = con.get::<String, Option<i64>>(counter_key.clone()).await? {
                    counter.set_remaining(val);
                    let ttl = con.ttl(&counter_key).await?;
                    counter.set_expires_in(ttl);

                    res.insert(counter);
                }
            }
        }

        Ok(res)
    }

    async fn clear(&self) -> Result<(), StorageErr> {
        let mut con = self.client.get_async_connection().await?;
        redis::cmd("FLUSHDB").query_async(&mut con).await?;
        Ok(())
    }
}

impl AsyncRedisStorage {
    pub fn new(redis_url: &str) -> AsyncRedisStorage {
        AsyncRedisStorage {
            client: redis::Client::open(redis_url).unwrap(),
        }
    }

    async fn delete_counters_of_namespace(&self, namespace: &str) -> Result<(), StorageErr> {
        for limit in self.get_limits(namespace).await? {
            self.delete_counters_associated_with_limit(&limit).await?
        }

        Ok(())
    }

    async fn delete_counters_associated_with_limit(&self, limit: &Limit) -> Result<(), StorageErr> {
        let mut con = self.client.get_async_connection().await?;

        let counter_keys = con
            .smembers::<String, HashSet<String>>(key_for_counters_of_limit(limit))
            .await?;

        for counter_key in counter_keys {
            con.del(counter_key).await?;
        }

        Ok(())
    }
}

impl Default for AsyncRedisStorage {
    fn default() -> Self {
        AsyncRedisStorage::new(DEFAULT_REDIS_URL)
    }
}
