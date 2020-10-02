extern crate redis;

use self::redis::Commands;
use crate::counter::Counter;
use crate::limit::{Limit, Namespace};
use crate::storage::redis::redis_keys::*;
use crate::storage::{Storage, StorageErr};
use std::collections::HashSet;
use std::iter::FromIterator;
use std::time::Duration;

const DEFAULT_REDIS_URL: &str = "redis://127.0.0.1:6379";

// Note: this implementation does no guarantee exact limits. Ensuring that we
// never go over the limits would hurt performance. This implementation
// sacrifices a bit of accuracy to be more performant.

pub struct RedisStorage {
    client: redis::Client,
}

impl Storage for RedisStorage {
    fn add_limit(&self, limit: &Limit) -> Result<(), StorageErr> {
        let mut con = self.client.get_connection()?;

        let set_key = key_for_limits_of_namespace(limit.namespace());
        let serialized_limit = serde_json::to_string(limit).unwrap();

        con.sadd::<String, String, _>(set_key, serialized_limit)?;
        Ok(())
    }

    fn get_limits(&self, namespace: &Namespace) -> Result<HashSet<Limit>, StorageErr> {
        let mut con = self.client.get_connection()?;

        let set_key = key_for_limits_of_namespace(namespace);

        let limits: HashSet<Limit> = HashSet::from_iter(
            con.smembers::<String, HashSet<String>>(set_key)?
                .iter()
                .map(|limit_json| serde_json::from_str(limit_json).unwrap()),
        );

        Ok(limits)
    }

    fn delete_limit(&self, limit: &Limit) -> Result<(), StorageErr> {
        let mut con = self.client.get_connection()?;

        self.delete_counters_associated_with_limit(limit)?;
        con.del(key_for_counters_of_limit(&limit))?;

        let set_key = key_for_limits_of_namespace(limit.namespace());
        let serialized_limit = serde_json::to_string(limit).unwrap();

        con.srem(set_key, serialized_limit)?;

        Ok(())
    }

    fn delete_limits(&self, namespace: &Namespace) -> Result<(), StorageErr> {
        let mut con = self.client.get_connection()?;

        self.delete_counters_of_namespace(namespace)?;

        for limit in self.get_limits(namespace)? {
            con.del(key_for_counters_of_limit(&limit))?;
        }

        let set_key = key_for_limits_of_namespace(namespace);
        con.del(set_key)?;

        Ok(())
    }

    fn is_within_limits(&self, counter: &Counter, delta: i64) -> Result<bool, StorageErr> {
        let mut con = self.client.get_connection()?;

        match con.get::<String, Option<i64>>(key_for_counter(counter))? {
            Some(val) => Ok(val - delta >= 0),
            None => Ok(counter.max_value() - delta >= 0),
        }
    }

    fn update_counter(&self, counter: &Counter, delta: i64) -> Result<(), StorageErr> {
        let mut con = self.client.get_connection()?;

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
            .query(&mut con)?;

        Ok(())
    }

    fn check_and_update(
        &self,
        counters: &HashSet<&Counter>,
        delta: i64,
    ) -> Result<bool, StorageErr> {
        let mut con = self.client.get_connection()?;

        let counter_keys: Vec<String> = counters
            .iter()
            .map(|counter| key_for_counter(counter))
            .collect();

        let counter_vals: Vec<Option<i64>> =
            redis::cmd("MGET").arg(counter_keys).query(&mut con)?;

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
            self.update_counter(counter, delta)?
        }

        Ok(true)
    }

    fn get_counters(&self, namespace: &Namespace) -> Result<HashSet<Counter>, StorageErr> {
        let mut res = HashSet::new();

        let mut con = self.client.get_connection()?;

        for limit in self.get_limits(namespace)? {
            let counter_keys =
                con.smembers::<String, HashSet<String>>(key_for_counters_of_limit(&limit))?;

            for counter_key in counter_keys {
                let mut counter: Counter = counter_from_counter_key(&counter_key);

                // If the key does not exist, it means that the counter expired,
                // so we don't have to return it.
                // TODO: we should delete the counter from the set of counters
                // associated with the limit taking into account that we should
                // do the "get" + "delete if none" atomically.
                // This does not cause any bugs, but consumes memory
                // unnecessarily.
                if let Some(val) = con.get::<String, Option<i64>>(counter_key.clone())? {
                    counter.set_remaining(val);
                    let ttl = con.ttl(&counter_key)?;
                    counter.set_expires_in(Duration::from_secs(ttl));

                    res.insert(counter);
                }
            }
        }

        Ok(res)
    }

    fn clear(&self) -> Result<(), StorageErr> {
        let mut con = self.client.get_connection()?;
        redis::cmd("FLUSHDB").execute(&mut con);
        Ok(())
    }
}

impl RedisStorage {
    pub fn new(redis_url: &str) -> RedisStorage {
        RedisStorage {
            client: redis::Client::open(redis_url).unwrap(),
        }
    }

    fn delete_counters_of_namespace(&self, namespace: &Namespace) -> Result<(), StorageErr> {
        for limit in self.get_limits(namespace)? {
            self.delete_counters_associated_with_limit(&limit)?
        }

        Ok(())
    }

    fn delete_counters_associated_with_limit(&self, limit: &Limit) -> Result<(), StorageErr> {
        let mut con = self.client.get_connection()?;

        let counter_keys =
            con.smembers::<String, HashSet<String>>(key_for_counters_of_limit(limit))?;

        for counter_key in counter_keys {
            con.del(counter_key)?;
        }

        Ok(())
    }
}

impl Default for RedisStorage {
    fn default() -> Self {
        RedisStorage::new(DEFAULT_REDIS_URL)
    }
}
