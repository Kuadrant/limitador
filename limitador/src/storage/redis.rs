extern crate redis;

use self::redis::Commands;
use crate::counter::Counter;
use crate::limit::Limit;
use crate::storage::{Storage, StorageErr};
use std::collections::HashSet;
use std::iter::FromIterator;

// TODO: try redis-rs async functions.

const DEFAULT_REDIS_URL: &str = "redis://127.0.0.1:6379";

pub struct RedisStorage {
    client: redis::Client,
}

impl Storage for RedisStorage {
    fn add_limit(&mut self, limit: &Limit) -> Result<(), StorageErr> {
        let mut con = self.client.get_connection()?;

        let set_key = Self::key_for_limits_of_namespace(limit.namespace());
        let serialized_limit = serde_json::to_string(limit).unwrap();

        con.sadd::<String, String, _>(set_key, serialized_limit)?;
        Ok(())
    }

    fn get_limits(&self, namespace: &str) -> Result<HashSet<Limit>, StorageErr> {
        let mut con = self.client.get_connection()?;

        let set_key = Self::key_for_limits_of_namespace(namespace);

        let limits: HashSet<Limit> = HashSet::from_iter(
            con.smembers::<String, HashSet<String>>(set_key)?
                .iter()
                .map(|limit_json| serde_json::from_str(limit_json).unwrap()),
        );

        Ok(limits)
    }

    fn delete_limit(&mut self, limit: &Limit) -> Result<(), StorageErr> {
        let mut con = self.client.get_connection()?;

        self.delete_counters_associated_with_limit(limit)?;
        con.del(Self::key_for_counters_of_limit(&limit))?;

        let set_key = Self::key_for_limits_of_namespace(limit.namespace());
        let serialized_limit = serde_json::to_string(limit).unwrap();

        con.srem(set_key, serialized_limit)?;

        Ok(())
    }

    fn delete_limits(&mut self, namespace: &str) -> Result<(), StorageErr> {
        let mut con = self.client.get_connection()?;

        self.delete_counters_of_namespace(namespace)?;

        for limit in self.get_limits(namespace)? {
            con.del(Self::key_for_counters_of_limit(&limit))?;
        }

        let set_key = Self::key_for_limits_of_namespace(namespace);
        con.del(set_key)?;

        Ok(())
    }

    fn is_within_limits(&self, counter: &Counter, delta: i64) -> Result<bool, StorageErr> {
        let mut con = self.client.get_connection()?;

        match con.get::<String, Option<i64>>(Self::key_for_counter(counter))? {
            Some(val) => Ok(val - delta >= 0),
            None => Ok(counter.max_value() - delta >= 0),
        }
    }

    fn update_counter(&mut self, counter: &Counter, delta: i64) -> Result<(), StorageErr> {
        let mut con = self.client.get_connection()?;

        let counter_key = Self::key_for_counter(counter);

        match con.get::<String, Option<i64>>(counter_key.clone())? {
            Some(_val) => con
                .incr::<String, i64, ()>(counter_key, -delta)
                .map_err(|e| e.into()),
            None => {
                con.set_ex::<String, i64, ()>(
                    counter_key,
                    counter.max_value() - delta,
                    counter.seconds() as usize,
                )?;

                self.add_counter_limit_association(counter).map_err(|e| e)
            }
        }
    }

    fn get_counters(&mut self, namespace: &str) -> Result<HashSet<Counter>, StorageErr> {
        let mut res = HashSet::new();

        let mut con = self.client.get_connection()?;

        for limit in self.get_limits(namespace)? {
            let counter_keys =
                con.smembers::<String, HashSet<String>>(Self::key_for_counters_of_limit(&limit))?;

            for counter_key in counter_keys {
                let mut counter: Counter = Self::counter_from_counter_key(&counter_key);

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
                    counter.set_expires_in(ttl);

                    res.insert(counter);
                }
            }
        }

        Ok(res)
    }
}

impl RedisStorage {
    pub fn new(redis_url: &str) -> RedisStorage {
        RedisStorage {
            client: redis::Client::open(redis_url).unwrap(),
        }
    }

    // To use Redis cluster and some Redis proxies, all the keys used in a
    // command or in a pipeline need to be sharded to the same server.
    // To help with that, Redis uses "hash tags". When a key contains "{" and
    // "}" only what's inside them is hashed.
    // To ensure that all the keys involved in a given operation belong to the
    // same shard, we can shard by namespace.
    // When there are multiple pairs of "{" and "}" only the first one is taken
    // into account. Ref: https://redis.io/topics/cluster-spec (key hash tags).
    // Reminder: in format!(), "{" is escaped with "{{".

    fn key_for_limits_of_namespace(namespace: &str) -> String {
        format!("limits_of_namespace:{{{}}}", namespace)
    }

    fn key_for_counter(counter: &Counter) -> String {
        format!(
            "namespace:{{{}}},counter:{}",
            counter.namespace(),
            serde_json::to_string(counter).unwrap()
        )
    }

    fn key_for_counters_of_limit(limit: &Limit) -> String {
        format!(
            "namespace:{{{}}},counters_of_limit:{}",
            limit.namespace(),
            serde_json::to_string(limit).unwrap()
        )
    }

    fn counter_from_counter_key(key: &str) -> Counter {
        let counter_prefix = "counter:";
        let start_pos_counter = key.find(counter_prefix).unwrap() + counter_prefix.len();

        serde_json::from_str(&key[start_pos_counter..]).unwrap()
    }

    fn add_counter_limit_association(&mut self, counter: &Counter) -> Result<(), StorageErr> {
        let mut con = self.client.get_connection()?;

        con.sadd::<String, String, _>(
            Self::key_for_counters_of_limit(counter.limit()),
            Self::key_for_counter(counter),
        )
        .map_err(|e| e.into())
    }

    fn delete_counters_of_namespace(&mut self, namespace: &str) -> Result<(), StorageErr> {
        for limit in self.get_limits(namespace)? {
            self.delete_counters_associated_with_limit(&limit)?
        }

        Ok(())
    }

    fn delete_counters_associated_with_limit(&mut self, limit: &Limit) -> Result<(), StorageErr> {
        let mut con = self.client.get_connection()?;

        let counter_keys =
            con.smembers::<String, HashSet<String>>(Self::key_for_counters_of_limit(limit))?;

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
