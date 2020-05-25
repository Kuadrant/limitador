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
            None => con
                .set_ex::<String, i64, ()>(
                    counter_key,
                    counter.max_value() - delta,
                    counter.seconds() as usize,
                )
                .map_err(|e| e.into()),
        }
    }

    fn get_counters(&mut self, _namespace: &str) -> Vec<(Counter, i64, Duration)> {
        unimplemented!()
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
}

impl Default for RedisStorage {
    fn default() -> Self {
        RedisStorage::new(DEFAULT_REDIS_URL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use serial_test::serial;

    #[test]
    #[serial]
    fn add_limit() {
        clean_db();
        let namespace = "test_namespace";
        let mut storage = RedisStorage::default();
        let limit = Limit::new(namespace, 10, 60, vec!["x == 10"], vec!["x"]);
        storage.add_limit(limit.clone()).unwrap();
        assert!(storage.get_limits(namespace).unwrap().contains(&limit))
    }

    #[test]
    #[serial]
    fn apply_limit() {
        clean_db();

        let max_hits = 3;
        let namespace = "test_namespace";

        let limit = Limit::new(
            namespace,
            max_hits,
            60,
            vec!["req.method == GET"],
            vec!["req.method", "app_id"],
        );

        let mut storage = RedisStorage::default();

        storage.add_limit(limit.clone()).unwrap();

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("namespace".to_string(), namespace.to_string());
        values.insert("req.method".to_string(), "GET".to_string());
        values.insert("app_id".to_string(), "test_app_id".to_string());

        let counter = Counter::new(limit, values);

        for _ in 0..max_hits {
            assert!(storage.is_within_limits(&counter, 1).unwrap());
            storage.update_counter(&counter, 1).unwrap();
        }
        assert_eq!(false, storage.is_within_limits(&counter, 1).unwrap());
    }

    fn clean_db() {
        let redis_client = redis::Client::open("redis://127.0.0.1:6379").unwrap();
        let mut con = redis_client.get_connection().unwrap();
        redis::cmd("FLUSHDB").execute(&mut con);
    }
}
