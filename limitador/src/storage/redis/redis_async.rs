extern crate redis;

use self::redis::aio::ConnectionManager;
use self::redis::ConnectionInfo;
use crate::counter::Counter;
use crate::limit::Limit;
use crate::storage::keys::*;
use crate::storage::redis::scripts::SCRIPT_UPDATE_COUNTER;
use crate::storage::{AsyncCounterStorage, Authorization, StorageErr};
use async_trait::async_trait;
use redis::AsyncCommands;
use std::collections::HashSet;
use std::str::FromStr;
use std::time::Duration;

// Note: this implementation does no guarantee exact limits. Ensuring that we
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
            .get::<String, Option<i64>>(key_for_counter(counter))
            .await?
        {
            Some(val) => Ok(val - delta >= 0),
            None => Ok((((counter.max_value() as i128) - delta as i128) as i64) >= 0),
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
        counters: HashSet<Counter>,
        delta: i64,
    ) -> Result<Authorization, StorageErr> {
        let mut con = self.conn_manager.clone();

        let counter_keys: Vec<String> = counters.iter().map(key_for_counter).collect();

        let counter_vals: Vec<Option<i64>> = redis::cmd("MGET")
            .arg(counter_keys)
            .query_async(&mut con)
            .await?;

        for (i, counter) in counters.iter().enumerate() {
            match counter_vals[i] {
                Some(val) => {
                    if val - delta < 0 {
                        return Ok(Authorization::Limited(
                            counter.limit().name().map(|n| n.to_owned()),
                        ));
                    }
                }
                None => {
                    if (((counter.max_value() as i128) - delta as i128) as i64) < 0 {
                        return Ok(Authorization::Limited(
                            counter.limit().name().map(|n| n.to_owned()),
                        ));
                    }
                }
            }
        }

        // TODO: this can be optimized by using pipelines with multiple updates
        for counter in counters {
            self.update_counter(&counter, delta).await?
        }

        Ok(Authorization::Ok)
    }

    async fn get_counters(&self, limits: HashSet<Limit>) -> Result<HashSet<Counter>, StorageErr> {
        let mut res = HashSet::new();

        let mut con = self.conn_manager.clone();

        for limit in limits {
            let counter_keys = con
                .smembers::<String, HashSet<String>>(key_for_counters_of_limit(&limit))
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
                if let Some(val) = con.get::<String, Option<i64>>(counter_key.clone()).await? {
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
    pub async fn new(redis_url: &str) -> Self {
        Self {
            conn_manager: ConnectionManager::new(
                redis::Client::open(ConnectionInfo::from_str(redis_url).unwrap()).unwrap(),
            )
            .await
            .unwrap(),
        }
    }

    pub fn new_with_conn_manager(conn_manager: ConnectionManager) -> Self {
        Self { conn_manager }
    }

    async fn delete_counters_associated_with_limit(&self, limit: &Limit) -> Result<(), StorageErr> {
        let mut con = self.conn_manager.clone();

        let counter_keys = con
            .smembers::<String, HashSet<String>>(key_for_counters_of_limit(limit))
            .await?;

        for counter_key in counter_keys {
            con.del(counter_key).await?;
        }

        Ok(())
    }
}
