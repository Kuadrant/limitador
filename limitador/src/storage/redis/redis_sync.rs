extern crate redis;

use self::redis::{Commands, ConnectionInfo, ConnectionLike, IntoConnectionInfo, RedisError};
use crate::counter::Counter;
use crate::limit::{Limit, Namespace};
use crate::storage::keys::*;
use crate::storage::redis::scripts::{SCRIPT_DELETE_LIMIT, SCRIPT_UPDATE_COUNTER};
use crate::storage::{Authorization, Storage, StorageErr};
use r2d2::{ManageConnection, Pool};
use std::collections::HashSet;
use std::time::Duration;

const DEFAULT_REDIS_URL: &str = "redis://127.0.0.1:6379";
const MAX_REDIS_CONNS: u32 = 20; // TODO: make it configurable

// Note: this implementation does no guarantee exact limits. Ensuring that we
// never go over the limits would hurt performance. This implementation
// sacrifices a bit of accuracy to be more performant.

pub struct RedisStorage {
    conn_pool: Pool<RedisConnectionManager>,
}

impl Storage for RedisStorage {
    fn get_namespaces(&self) -> Result<HashSet<Namespace>, StorageErr> {
        let mut con = self.conn_pool.get()?;

        let namespaces = con.smembers::<String, HashSet<String>>(key_for_namespaces_set())?;

        Ok(namespaces.iter().map(|ns| ns.as_str().into()).collect())
    }

    fn add_limit(&self, limit: &Limit) -> Result<(), StorageErr> {
        let mut con = self.conn_pool.get()?;

        let set_key = key_for_limits_of_namespace(limit.namespace());
        let serialized_limit = serde_json::to_string(limit).unwrap();

        redis::pipe()
            .atomic()
            .sadd::<String, String>(set_key, serialized_limit)
            .sadd::<String, &str>(key_for_namespaces_set(), limit.namespace().as_ref())
            .query(&mut *con)?;

        Ok(())
    }

    fn get_limits(&self, namespace: &Namespace) -> Result<HashSet<Limit>, StorageErr> {
        let mut con = self.conn_pool.get()?;

        let set_key = key_for_limits_of_namespace(namespace);

        let limits: HashSet<Limit> = con
            .smembers::<String, HashSet<String>>(set_key)?
            .iter()
            .map(|limit_json| serde_json::from_str(limit_json).unwrap())
            .collect();

        Ok(limits)
    }

    fn delete_limit(&self, limit: &Limit) -> Result<(), StorageErr> {
        let mut con = self.conn_pool.get()?;

        self.delete_counters_associated_with_limit(limit)?;
        con.del(key_for_counters_of_limit(limit))?;

        let serialized_limit = serde_json::to_string(limit).unwrap();

        redis::Script::new(SCRIPT_DELETE_LIMIT)
            .key(key_for_limits_of_namespace(limit.namespace()))
            .key(key_for_namespaces_set())
            .arg(serialized_limit)
            .arg(limit.namespace().as_ref())
            .invoke(&mut *con)?;

        Ok(())
    }

    fn delete_limits(&self, namespace: &Namespace) -> Result<(), StorageErr> {
        let mut con = self.conn_pool.get()?;

        self.delete_counters_of_namespace(namespace)?;

        for limit in self.get_limits(namespace)? {
            con.del(key_for_counters_of_limit(&limit))?;
        }

        let set_key = key_for_limits_of_namespace(namespace);
        con.del(set_key)?;

        Ok(())
    }

    fn is_within_limits(&self, counter: &Counter, delta: i64) -> Result<bool, StorageErr> {
        let mut con = self.conn_pool.get()?;

        match con.get::<String, Option<i64>>(key_for_counter(counter))? {
            Some(val) => Ok(val - delta >= 0),
            None => Ok(counter.max_value() - delta >= 0),
        }
    }

    fn update_counter(&self, counter: &Counter, delta: i64) -> Result<(), StorageErr> {
        let mut con = self.conn_pool.get()?;

        redis::Script::new(SCRIPT_UPDATE_COUNTER)
            .key(key_for_counter(counter))
            .key(key_for_counters_of_limit(counter.limit()))
            .arg(counter.max_value())
            .arg(counter.seconds())
            .arg(delta)
            .invoke(&mut *con)?;

        Ok(())
    }

    fn check_and_update<'c>(
        &self,
        counters: &HashSet<&'c Counter>,
        delta: i64,
    ) -> Result<Authorization<'c>, StorageErr> {
        let mut con = self.conn_pool.get()?;

        let counter_keys: Vec<String> = counters
            .iter()
            .map(|counter| key_for_counter(counter))
            .collect();

        let counter_vals: Vec<Option<i64>> =
            redis::cmd("MGET").arg(counter_keys).query(&mut *con)?;

        for (i, counter) in counters.iter().enumerate() {
            match counter_vals[i] {
                Some(val) => {
                    if val - delta < 0 {
                        return Ok(Authorization::Limited(counter));
                    }
                }
                None => {
                    if counter.max_value() - delta < 0 {
                        return Ok(Authorization::Limited(counter));
                    }
                }
            }
        }

        // TODO: this can be optimized by using pipelines with multiple updates
        for counter in counters {
            self.update_counter(counter, delta)?
        }

        Ok(Authorization::Ok)
    }

    fn get_counters(&self, namespace: &Namespace) -> Result<HashSet<Counter>, StorageErr> {
        let mut res = HashSet::new();

        let mut con = self.conn_pool.get()?;

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
        let mut con = self.conn_pool.get()?;
        redis::cmd("FLUSHDB").execute(&mut *con);
        Ok(())
    }
}

impl RedisStorage {
    pub fn new(redis_url: &str) -> Self {
        let conn_manager = RedisConnectionManager::new(redis_url).unwrap();
        let conn_pool = Pool::builder()
            .connection_timeout(Duration::from_secs(3))
            .max_size(MAX_REDIS_CONNS)
            .build(conn_manager)
            .unwrap();

        Self { conn_pool }
    }

    fn delete_counters_of_namespace(&self, namespace: &Namespace) -> Result<(), StorageErr> {
        for limit in self.get_limits(namespace)? {
            self.delete_counters_associated_with_limit(&limit)?
        }

        Ok(())
    }

    fn delete_counters_associated_with_limit(&self, limit: &Limit) -> Result<(), StorageErr> {
        let mut con = self.conn_pool.get()?;

        let counter_keys =
            con.smembers::<String, HashSet<String>>(key_for_counters_of_limit(limit))?;

        for counter_key in counter_keys {
            con.del(counter_key)?;
        }

        Ok(())
    }
}

// The RedisConnectionManager is very similar to the one found in the r2d2_redis
// crate. That crate has not been updated in a long time and depends on an old
// version of the Redis crate. That's why I decided not to import it.

#[derive(Debug)]
pub struct RedisConnectionManager {
    connection_info: ConnectionInfo,
}

impl RedisConnectionManager {
    pub fn new<T: IntoConnectionInfo>(params: T) -> Result<Self, RedisError> {
        Ok(Self {
            connection_info: params.into_connection_info()?,
        })
    }
}

impl ManageConnection for RedisConnectionManager {
    type Connection = redis::Connection;
    type Error = RedisError;

    fn connect(&self) -> Result<Self::Connection, Self::Error> {
        match redis::Client::open(self.connection_info.clone()) {
            Ok(client) => client.get_connection(),
            Err(err) => Err(err),
        }
    }

    fn is_valid(&self, conn: &mut Self::Connection) -> Result<(), Self::Error> {
        redis::cmd("PING").query(conn)
    }

    fn has_broken(&self, conn: &mut Self::Connection) -> bool {
        !conn.is_open()
    }
}

impl Default for RedisStorage {
    fn default() -> Self {
        Self::new(DEFAULT_REDIS_URL)
    }
}

#[cfg(test)]
mod test {
    use crate::storage::redis::RedisStorage;

    #[test]
    fn create_default() {
        let _ = RedisStorage::default();
    }

    #[test]
    #[ignore]
    fn create_storage_with_custom_url() {
        let _r = RedisStorage::new("redis://127.0.0.1:6379");
    }
}
