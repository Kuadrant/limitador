extern crate redis;

use self::redis::aio::{ConnectionLike, ConnectionManager};
use self::redis::ConnectionInfo;
use crate::counter::Counter;
use crate::limit::Limit;
use crate::storage::keys::*;
use crate::storage::redis::scripts::{
    GET_COUNTERS_AND_PRUNE, SCRIPT_UPDATE_COUNTER, VALUES_AND_TTLS,
};
use crate::storage::redis::{is_limited, live_counters_from_script_res, COUNTERS_SCAN_BATCH_SIZE};
use crate::storage::{AsyncCounterStorage, Authorization, StorageErr};
use async_trait::async_trait;
use redis::{AsyncCommands, ErrorKind, RedisError};
use std::collections::HashSet;
use std::ops::Deref;
use std::str::FromStr;
use std::sync::Arc;
use tracing::{info_span, Instrument};

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
    #[tracing::instrument(skip_all)]
    async fn is_within_limits(&self, counter: &Counter, delta: u64) -> Result<bool, StorageErr> {
        let mut con = self.conn_manager.clone();

        match con
            .get::<Vec<u8>, Option<i64>>(key_for_counter(counter))
            .instrument(info_span!("datastore"))
            .await?
        {
            Some(val) => Ok(u64::try_from(val).unwrap_or(0) + delta <= counter.max_value()),
            None => Ok(counter.max_value().checked_sub(delta).is_some()),
        }
    }

    #[tracing::instrument(skip_all)]
    async fn update_counter(&self, counter: &Counter, delta: u64) -> Result<(), StorageErr> {
        let mut con = self.conn_manager.clone();

        redis::Script::new(SCRIPT_UPDATE_COUNTER)
            .key(key_for_counter(counter))
            .key(key_for_counters_of_limit(counter.limit()))
            .arg(counter.window().as_secs())
            .arg(delta)
            .invoke_async::<()>(&mut con)
            .instrument(info_span!("datastore"))
            .await?;

        Ok(())
    }

    #[tracing::instrument(skip_all)]
    async fn check_and_update<'a>(
        &self,
        counters: &mut Vec<Counter>,
        delta: u64,
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

            let script_res: Vec<Option<i64>> = {
                script_invocation
                    .invoke_async(&mut con)
                    .instrument(info_span!("datastore"))
                    .await?
            };
            if let Some(res) = is_limited(counters, delta, script_res) {
                return Ok(res);
            }
        } else {
            let counter_vals: Vec<Option<i64>> = {
                redis::cmd("MGET")
                    .arg(counter_keys.clone())
                    .query_async(&mut con)
                    .instrument(info_span!("datastore"))
                    .await?
            };

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

        let script = redis::Script::new(SCRIPT_UPDATE_COUNTER);
        let mut pipeline = redis::pipe();
        let mut pipeline = &mut pipeline;
        for (counter_idx, key) in counter_keys.iter().enumerate() {
            let counter = &counters[counter_idx];
            pipeline = pipeline
                .invoke_script(
                    script
                        .key(key)
                        .key(key_for_counters_of_limit(counter.limit()))
                        .arg(counter.window().as_secs())
                        .arg(delta),
                )
                .ignore()
        }
        if let Err(err) = pipeline
            .query_async::<()>(&mut con)
            .instrument(info_span!("datastore"))
            .await
        {
            if err.kind() == ErrorKind::NoScriptError {
                script.prepare_invoke().load_async(&mut con).await?;
                pipeline
                    .query_async::<()>(&mut con)
                    .instrument(info_span!("datastore"))
                    .await?;
            } else {
                Err(err)?;
            }
        }

        Ok(Authorization::Ok)
    }

    #[tracing::instrument(skip_all)]
    async fn get_counters(
        &self,
        limits: &HashSet<Arc<Limit>>,
    ) -> Result<HashSet<Counter>, StorageErr> {
        let mut res = HashSet::new();

        let mut con = self.conn_manager.clone();

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
                    .query_async(&mut con)
                    .instrument(info_span!("datastore"))
                    .await?;

                if !members.is_empty() {
                    let script_res = read_and_prune_counters(&mut con, &set_key, &members).await?;
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
    async fn delete_counters(&self, limits: &HashSet<Arc<Limit>>) -> Result<(), StorageErr> {
        for limit in limits {
            self.delete_counters_associated_with_limit(limit.deref())
                .instrument(info_span!("datastore"))
                .await?
        }
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    async fn clear(&self) -> Result<(), StorageErr> {
        let mut con = self.conn_manager.clone();
        redis::cmd("FLUSHDB")
            .query_async::<()>(&mut con)
            .instrument(info_span!("datastore"))
            .await?;
        Ok(())
    }
}

impl AsyncRedisStorage {
    pub async fn new(redis_url: &str) -> Result<Self, RedisError> {
        let info = ConnectionInfo::from_str(redis_url)?;
        Self::new_with_conn_manager(
            ConnectionManager::new(
                redis::Client::open(info)
                    .expect("This couldn't fail in the past, yet now it did somehow!"),
            )
            .await?,
        )
        .await
    }

    pub async fn new_with_conn_manager(
        conn_manager: ConnectionManager,
    ) -> Result<Self, RedisError> {
        let store = Self { conn_manager };
        store.load_script(SCRIPT_UPDATE_COUNTER).await?;
        store.load_script(VALUES_AND_TTLS).await?;
        store.load_script(GET_COUNTERS_AND_PRUNE).await?;
        Ok(store)
    }

    async fn delete_counters_associated_with_limit(&self, limit: &Limit) -> Result<(), StorageErr> {
        let mut con = self.conn_manager.clone();

        let counter_keys = {
            con.smembers::<Vec<u8>, HashSet<Vec<u8>>>(key_for_counters_of_limit(limit))
                .instrument(info_span!("datastore"))
                .await?
        };

        for counter_key in counter_keys {
            con.del::<_, ()>(counter_key)
                .instrument(info_span!("datastore"))
                .await?;
        }

        con.del::<_, ()>(key_for_counters_of_limit(limit)).await?;

        Ok(())
    }

    pub(super) async fn load_script(&self, script: &str) -> Result<(), RedisError> {
        let mut con = self.conn_manager.clone();
        let script = redis::Script::new(script);
        script.prepare_invoke().load_async(&mut con).await?;
        Ok(())
    }
}

async fn read_and_prune_counters<C: ConnectionLike>(
    redis_conn: &mut C,
    set_key: &[u8],
    members: &[Vec<u8>],
) -> Result<Vec<Option<i64>>, StorageErr> {
    let script = redis::Script::new(GET_COUNTERS_AND_PRUNE);
    let mut script_invocation = script.prepare_invoke();
    script_invocation.key(set_key);
    for member in members {
        script_invocation.key(member);
    }

    Ok(script_invocation
        .invoke_async(redis_conn)
        .instrument(info_span!("datastore"))
        .await?)
}

#[cfg(test)]
mod tests {
    use super::read_and_prune_counters;
    use crate::storage::redis::scripts::GET_COUNTERS_AND_PRUNE;
    use crate::storage::redis::AsyncRedisStorage;
    use redis::{ErrorKind, Value};
    use redis_test::{MockCmd, MockRedisConnection};

    #[tokio::test]
    async fn read_and_prune_counters_declares_every_counter_as_a_key() {
        let script = redis::Script::new(GET_COUNTERS_AND_PRUNE);
        let set_key = b"namespace:{test_namespace},counters_of_limit:{}".to_vec();
        let members = vec![
            b"namespace:{test_namespace},counter:alive".to_vec(),
            b"namespace:{test_namespace},counter:expired".to_vec(),
        ];

        // Every counter is declared as a key of its own, not an argument, so Redis cluster can route
        // the script
        let mut mock_client = MockRedisConnection::new(vec![MockCmd::new(
            redis::cmd("EVALSHA")
                .arg(script.get_hash())
                .arg(3)
                .arg(&set_key)
                .arg(&members[0])
                .arg(&members[1]),
            Ok(Value::Array(vec![
                Value::BulkString(b"3".to_vec()),
                Value::Int(59_000),
                Value::Nil,
                Value::Int(-2),
            ])),
        )]);

        let script_res = read_and_prune_counters(&mut mock_client, &set_key, &members)
            .await
            .expect("should have read the batch");

        // A nil value marks the member the script removed, and does not shift the pairs that follow
        assert_eq!(script_res, vec![Some(3), Some(59_000), None, Some(-2)]);
    }

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
