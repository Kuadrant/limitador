extern crate redis;

use self::redis::Commands;
use crate::counter::Counter;
use crate::limit::Limit;
use crate::storage::{Storage, StorageErr};
use std::collections::HashSet;
use std::iter::FromIterator;
use std::time::Duration;

// TODO: define keys so that all the ones that belong to the same namespace
// go to the same shard.

// TODO: try redis-rs async functions.

const DEFAULT_REDIS_URL: &str = "redis://127.0.0.1:6379";

pub struct RedisStorage {
    client: redis::Client,
}

impl Storage for RedisStorage {
    fn add_limit(&mut self, limit: Limit) -> Result<(), StorageErr> {
        let mut con = self.client.get_connection()?;

        let set_key = Self::key_for_limits_of_namespace(limit.namespace());
        let serialized_limit = serde_json::to_string(&limit).unwrap();

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
        // TODO: Delete the counters associated with a limit too.

        let mut con = self.client.get_connection()?;

        let set_key = Self::key_for_limits_of_namespace(limit.namespace());
        let serialized_limit = serde_json::to_string(limit).unwrap();

        con.srem(set_key, serialized_limit)?;

        Ok(())
    }

    fn delete_limits(&mut self, namespace: &str) -> Result<(), StorageErr> {
        // TODO: delete counters associated with the limits.

        let mut con = self.client.get_connection()?;

        let set_key = Self::key_for_limits_of_namespace(namespace);

        con.del(set_key)?;

        self.delete_counters_of_namespace(namespace)?;

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

    fn get_counters(
        &mut self,
        namespace: &str,
    ) -> Result<Vec<(Counter, i64, Duration)>, StorageErr> {
        let mut res = vec![];

        let mut con = self.client.get_connection()?;

        for limit in self.get_limits(namespace)? {
            let counter_keys =
                con.smembers::<String, HashSet<String>>(Self::key_for_counters_of_limit(&limit))?;

            for counter_key in counter_keys {
                let counter: Counter = serde_json::from_str(&counter_key).unwrap();
                let val = match con.get::<String, Option<i64>>(counter_key.clone())? {
                    Some(val) => val,
                    None => 0,
                };
                let ttl = con.ttl(&counter_key)?;

                res.push((counter, val, Duration::new(ttl, 0)));
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

    fn key_for_limits_of_namespace(namespace: &str) -> String {
        format!("limits_of_namespace:{}", namespace)
    }

    fn key_for_counter(counter: &Counter) -> String {
        serde_json::to_string(counter).unwrap()
    }

    fn key_for_counters_of_limit(limit: &Limit) -> String {
        format!(
            "counters_of_limit:{}",
            serde_json::to_string(limit).unwrap()
        )
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
