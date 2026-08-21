extern crate redis;

use self::redis::{Commands, ConnectionInfo, ConnectionLike, IntoConnectionInfo, RedisError};
use crate::counter::Counter;
use crate::limit::Limit;
use crate::storage::keys::*;
use crate::storage::redis::scripts::{
    GET_COUNTERS_AND_PRUNE, SCRIPT_UPDATE_COUNTER, VALUES_AND_TTLS,
};
use crate::storage::redis::{is_limited, live_counters_from_script_res, COUNTERS_SCAN_BATCH_SIZE};
use crate::storage::{Authorization, CounterStorage, StorageErr};
use r2d2::{ManageConnection, Pool};
use std::collections::HashSet;
use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;

const DEFAULT_REDIS_URL: &str = "redis://127.0.0.1:6379";
const MAX_REDIS_CONNS: u32 = 20; // TODO: make it configurable

// Note: this implementation does no guarantee exact limits. Ensuring that we
// never go over the limits would hurt performance. This implementation
// sacrifices a bit of accuracy to be more performant.

pub struct RedisStorage {
    conn_pool: Pool<RedisConnectionManager>,
}

impl CounterStorage for RedisStorage {
    #[tracing::instrument(skip_all)]
    fn is_within_limits(&self, counter: &Counter, delta: u64) -> Result<bool, StorageErr> {
        let mut con = self.conn_pool.get()?;

        match con.get::<Vec<u8>, Option<i64>>(key_for_counter(counter))? {
            Some(val) => Ok(u64::try_from(val).unwrap_or(0) + delta <= counter.max_value()),
            None => Ok(counter.max_value().checked_sub(delta).is_some()),
        }
    }

    #[tracing::instrument(skip_all)]
    fn add_counter(&self, _limit: &Limit) -> Result<(), StorageErr> {
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    fn update_counter(&self, counter: &Counter, delta: u64) -> Result<(), StorageErr> {
        let mut con = self.conn_pool.get()?;

        redis::Script::new(SCRIPT_UPDATE_COUNTER)
            .key(key_for_counter(counter))
            .key(key_for_counters_of_limit(counter.limit()))
            .arg(counter.window().as_secs())
            .arg(delta)
            .invoke::<()>(&mut *con)?;

        Ok(())
    }

    #[tracing::instrument(skip_all)]
    fn check_and_update(
        &self,
        counters: &mut Vec<Counter>,
        delta: u64,
        load_counters: bool,
    ) -> Result<Authorization, StorageErr> {
        let mut con = self.conn_pool.get()?;
        let counter_keys: Vec<Vec<u8>> = counters.iter().map(key_for_counter).collect();

        if load_counters {
            let script = redis::Script::new(VALUES_AND_TTLS);
            let mut script_invocation = script.prepare_invoke();
            for counter_key in &counter_keys {
                script_invocation.key(counter_key);
            }
            let script_res: Vec<Option<i64>> = script_invocation.invoke(&mut *con)?;

            if let Some(res) = is_limited(counters, delta, script_res) {
                return Ok(res);
            }
        } else {
            let counter_vals: Vec<Option<i64>> = redis::cmd("MGET")
                .arg(counter_keys.clone())
                .query(&mut *con)?;

            for (i, counter) in counters.iter().enumerate() {
                // remaining  = max - (curr_val + delta)
                let remaining = counter
                    .max_value()
                    .checked_sub(u64::try_from(counter_vals[i].unwrap_or(0)).unwrap_or(0) + delta);
                if remaining.is_none() {
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
                .arg(counter.window().as_secs())
                .arg(delta)
                .invoke::<()>(&mut *con)?;
        }

        Ok(Authorization::Ok)
    }

    #[tracing::instrument(skip_all)]
    fn get_counters(&self, limits: &HashSet<Arc<Limit>>) -> Result<HashSet<Counter>, StorageErr> {
        let mut res = HashSet::new();

        let mut con = self.conn_pool.get()?;

        for limit in limits {
            let set_key = key_for_counters_of_limit(limit);
            let mut cursor = 0u64;

            loop {
                // SSCAN can repeat a member, or skip one added mid-scan. Duplicates collapse in `res`
                // Skipped member stays in the set, so a later get_counters returns it.
                let (next_cursor, members): (u64, Vec<Vec<u8>>) = redis::cmd("SSCAN")
                    .arg(&set_key)
                    .arg(cursor)
                    .arg("COUNT")
                    .arg(COUNTERS_SCAN_BATCH_SIZE)
                    .query(&mut *con)?;

                if !members.is_empty() {
                    let script = redis::Script::new(GET_COUNTERS_AND_PRUNE);
                    let mut script_invocation = script.prepare_invoke();
                    script_invocation.key(&set_key);
                    for member in &members {
                        script_invocation.key(member);
                    }
                    let script_res: Vec<Option<i64>> = script_invocation.invoke(&mut *con)?;

                    res.extend(live_counters_from_script_res(limit, &members, &script_res));
                }

                if next_cursor == 0 {
                    break;
                }
                cursor = next_cursor;
            }
        }

        Ok(res)
    }

    #[tracing::instrument(skip_all)]
    fn delete_counters(&self, limits: &HashSet<Arc<Limit>>) -> Result<(), StorageErr> {
        let mut con = self.conn_pool.get()?;

        for limit in limits {
            let counter_keys = con
                .smembers::<Vec<u8>, HashSet<Vec<u8>>>(key_for_counters_of_limit(limit.deref()))?;

            for counter_key in counter_keys {
                con.del::<_, ()>(counter_key)?;
            }
            con.del::<_, ()>(key_for_counters_of_limit(limit))?;
        }

        Ok(())
    }

    #[tracing::instrument(skip_all)]
    fn clear(&self) -> Result<(), StorageErr> {
        let mut con = self.conn_pool.get()?;
        redis::cmd("FLUSHDB").exec(&mut *con)?;
        Ok(())
    }
}

impl RedisStorage {
    pub fn new(redis_url: &str) -> Result<Self, String> {
        let conn_manager = match RedisConnectionManager::new(redis_url) {
            Ok(conn_manager) => conn_manager,
            Err(err) => {
                return Err(err.to_string());
            }
        };
        match Pool::builder()
            .connection_timeout(Duration::from_secs(3))
            .max_size(MAX_REDIS_CONNS)
            .build(conn_manager)
        {
            Ok(conn_pool) => Ok(Self { conn_pool }),
            Err(err) => Err(err.to_string()),
        }
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
        Self::new(DEFAULT_REDIS_URL).unwrap()
    }
}

impl From<::r2d2::Error> for StorageErr {
    fn from(e: ::r2d2::Error) -> Self {
        Self {
            msg: e.to_string(),
            source: Some(Box::new(e)),
            transient: false,
        }
    }
}

#[cfg(test)]
mod test {
    use crate::storage::redis::RedisStorage;

    #[test]
    fn errs_on_bad_url() {
        let result = RedisStorage::new("cassandra://127.0.0.1:6379");
        assert!(result.is_err());
        assert_eq!(
            result.err().unwrap(),
            "Redis URL did not parse- InvalidClientConfig".to_string()
        )
    }

    #[test]
    fn errs_on_connection_issue() {
        // this used to panic! And I really don't see how to bubble the redis error back up:
        // r2d2 consumes it
        // RedisError are not publicly constructable
        // So using String as error type… sad
        let result = RedisStorage::new("redis://127.0.0.1:21");
        assert!(result.is_err());
        assert!(result.err().unwrap().contains("Connection refused"));
    }

    #[test]
    #[ignore]
    fn create_storage_with_custom_url() {
        let _r = RedisStorage::new("redis://127.0.0.1:6379");
    }
}
