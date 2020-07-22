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
    fn add_limit_without_vars() {
        clean_db();
        let namespace = "test_namespace";
        let mut storage = RedisStorage::default();
        let limit = Limit::new(namespace, 10, 60, vec!["x == 10"], Vec::<String>::new());
        storage.add_limit(limit.clone()).unwrap();
        assert!(storage.get_limits(namespace).unwrap().contains(&limit))
    }

    #[test]
    #[serial]
    fn delete_limit() {
        clean_db();
        let namespace = "test_namespace";
        let mut storage = RedisStorage::default();
        let limit = Limit::new(namespace, 10, 60, vec!["x == 10"], vec!["y"]);
        storage.add_limit(limit.clone()).unwrap();

        storage.delete_limit(&limit).unwrap();

        assert!(storage.get_limits(namespace).unwrap().is_empty())
    }

    #[test]
    #[serial]
    fn delete_limit_also_deletes_associated_counters() {
        clean_db();
        let namespace = "test_namespace";
        let mut storage = RedisStorage::default();
        let limit = Limit::new(namespace, 10, 60, vec!["x == 1"], vec!["z"]);

        storage.add_limit(limit.clone()).unwrap();

        let mut values = HashMap::new();
        values.insert("x".to_string(), "1".to_string());
        values.insert("z".to_string(), "2".to_string());
        let counter = Counter::new(limit.clone(), values);
        storage.update_counter(&counter, 1).unwrap();

        storage.delete_limit(&limit).unwrap();

        assert!(storage.get_counters(namespace).unwrap().is_empty())
    }

    #[test]
    #[serial]
    fn delete_limits() {
        clean_db();
        let namespace = "test_namespace";
        let mut storage = RedisStorage::default();

        [
            Limit::new(namespace, 10, 60, vec!["x == 10"], vec!["z"]),
            Limit::new(namespace, 20, 60, vec!["y == 5"], vec!["z"]),
        ]
        .iter()
        .for_each(|limit| storage.add_limit(limit.clone()).unwrap());

        storage.delete_limits(namespace).unwrap();

        assert!(storage.get_limits(namespace).unwrap().is_empty())
    }

    #[test]
    #[serial]
    fn delete_limits_does_not_delete_limits_from_other_namespaces() {
        clean_db();
        let namespace1 = "test_namespace_1";
        let namespace2 = "test_namespace_2";
        let mut storage = RedisStorage::default();

        storage
            .add_limit(Limit::new(namespace1, 10, 60, vec!["x == 10"], vec!["z"]))
            .unwrap();
        storage
            .add_limit(Limit::new(namespace2, 5, 60, vec!["x == 10"], vec!["z"]))
            .unwrap();

        storage.delete_limits(namespace1).unwrap();

        assert!(storage.get_limits(namespace1).unwrap().is_empty());
        assert_eq!(storage.get_limits(namespace2).unwrap().len(), 1)
    }

    #[test]
    #[serial]
    fn delete_limits_of_a_namespace_also_deletes_counters() {
        clean_db();
        let namespace = "test_namespace";
        let mut storage = RedisStorage::default();
        let limit = Limit::new(namespace, 10, 60, vec!["x == 1"], vec!["z"]);

        storage.add_limit(limit.clone()).unwrap();

        let mut values = HashMap::new();
        values.insert("x".to_string(), "1".to_string());
        values.insert("z".to_string(), "2".to_string());
        let counter = Counter::new(limit.clone(), values);
        storage.update_counter(&counter, 1).unwrap();

        storage.delete_limits(namespace).unwrap();

        assert!(storage.get_counters(namespace).unwrap().is_empty())
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

    #[test]
    #[serial]
    fn get_counters() {
        clean_db();

        let namespace = "test_namespace";
        let max_hits = 10;
        let hits_app_1 = 1;
        let hits_app_2 = 5;

        let limit = Limit::new(
            namespace,
            max_hits,
            60,
            vec!["req.method == GET"],
            vec!["app_id"],
        );

        let mut storage = RedisStorage::default();
        storage.add_limit(limit.clone()).unwrap();

        let mut values = HashMap::new();
        values.insert("req.method".to_string(), "GET".to_string());
        values.insert("app_id".to_string(), "1".to_string());
        let counter_1 = Counter::new(limit.clone(), values.clone());
        storage.update_counter(&counter_1, hits_app_1).unwrap();

        values.insert("app_id".to_string(), "2".to_string());
        let counter_2 = Counter::new(limit.clone(), values.clone());
        storage.update_counter(&counter_2, hits_app_2).unwrap();

        let counters = storage.get_counters(namespace).unwrap();

        assert_eq!(counters.len(), 2);

        for counter_data in counters {
            let app_id = counter_data.0.set_variables().get("app_id").unwrap();

            match app_id.as_str() {
                "1" => assert_eq!(counter_data.1, max_hits - hits_app_1),
                "2" => assert_eq!(counter_data.1, max_hits - hits_app_2),
                _ => panic!("Unexpected app ID"),
            }
        }
    }

    #[test]
    #[serial]
    fn get_counters_can_return_empty_list() {
        clean_db();

        // There's a limit, but no counters. The result should be empty.

        let limit = Limit::new(
            "test_namespace",
            10,
            60,
            vec!["req.method == GET"],
            vec!["app_id"],
        );

        let mut storage = RedisStorage::default();
        storage.add_limit(limit.clone()).unwrap();

        assert!(storage.get_counters("test_namespace").unwrap().is_empty());
    }

    fn clean_db() {
        let redis_client = redis::Client::open("redis://127.0.0.1:6379").unwrap();
        let mut con = redis_client.get_connection().unwrap();
        redis::cmd("FLUSHDB").execute(&mut con);
    }
}
