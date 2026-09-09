extern crate redis;

use self::redis::{Commands, ConnectionInfo, ConnectionLike, IntoConnectionInfo, RedisError};
use crate::counter::Counter;
use crate::limit::Limit;
use crate::reservation::ReservationId;
use crate::storage::keys::*;
use crate::storage::redis::is_limited;
use crate::storage::redis::scripts::{SCRIPT_RESERVE, SCRIPT_UPDATE_COUNTER, VALUES_AND_TTLS};
use crate::storage::{Authorization, CounterStorage, StorageErr};
use r2d2::{ManageConnection, Pool};
use std::collections::HashSet;
use std::ops::Deref;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
            let counter_keys =
                con.smembers::<Vec<u8>, HashSet<Vec<u8>>>(key_for_counters_of_limit(limit))?;

            for counter_key in counter_keys {
                let mut counter: Counter =
                    counter_from_counter_key(&counter_key, Arc::clone(limit));

                // If the key does not exist, it means that the counter expired,
                // so we don't have to return it.
                // TODO: we should delete the counter from the set of counters
                // associated with the limit taking into account that we should
                // do the "get" + "delete if none" atomically.
                // This does not cause any bugs, but consumes memory
                // unnecessarily.
                if let Some(val) = con.get::<Vec<u8>, Option<i64>>(counter_key.clone())? {
                    counter.set_remaining(
                        limit
                            .max_value()
                            .saturating_sub(u64::try_from(val).unwrap_or(0)),
                    );
                    let ttl = con.ttl(&counter_key)?;
                    counter.set_expires_in(Duration::from_secs(ttl));

                    res.insert(counter);
                }
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

    #[tracing::instrument(skip_all)]
    fn reserve(
        &self,
        counters: &mut Vec<Counter>,
        reservation_id: &ReservationId,
        amount: u64,
        ttl: Duration,
        load_counters: bool,
    ) -> Result<Authorization, StorageErr> {
        let mut con = self.conn_pool.get()?;

        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        let script = redis::Script::new(SCRIPT_RESERVE);
        let mut invocation = script.prepare_invoke();
        for counter in counters.iter() {
            invocation.key(key_for_counter(counter));
            invocation.key(key_for_reservations(counter));
            invocation.key(key_for_counters_of_limit(counter.limit()));
        }
        for counter in counters.iter() {
            invocation.arg(counter.window().as_secs());
            invocation.arg(counter.max_value());
        }
        invocation
            .arg(amount)
            .arg(ttl.as_millis() as i64)
            .arg(reservation_id.as_str())
            .arg(now_ms);

        let raw: Vec<i64> = invocation.invoke(&mut *con)?;
        let admitted = raw.last().copied().unwrap_or(0) == 1;

        let mut first_limited = None;
        for (i, counter) in counters.iter_mut().enumerate() {
            let value = raw[i * 3].max(0) as u64;
            let outstanding = raw[i * 3 + 1].max(0) as u64;
            let window_ttl_ms = raw[i * 3 + 2].max(0) as u64;
            let total = value + outstanding + amount;

            if load_counters {
                let remaining = counter.max_value().checked_sub(total);
                counter.set_remaining(remaining.unwrap_or_default());
                counter.set_expires_in(Duration::from_millis(window_ttl_ms));
            }
            if first_limited.is_none() && total > counter.max_value() {
                first_limited = Some(counter.limit().name().map(|n| n.to_owned()));
            }
        }

        if admitted {
            Ok(Authorization::Ok)
        } else {
            Ok(Authorization::Limited(first_limited.unwrap_or(None)))
        }
    }

    #[tracing::instrument(skip_all)]
    fn release_reservation(
        &self,
        counters: &[Counter],
        reservation_id: &ReservationId,
    ) -> Result<bool, StorageErr> {
        let mut con = self.conn_pool.get()?;

        let mut released = false;
        for counter in counters {
            let removed: i64 = con.hdel(key_for_reservations(counter), reservation_id.as_str())?;
            released |= removed > 0;
        }
        Ok(released)
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
    use crate::counter::Counter;
    use crate::limit::{Context, Expression, Limit};
    use crate::reservation::ReservationId;
    use crate::storage::keys::key_for_counter;
    use crate::storage::redis::RedisStorage;
    use crate::storage::{Authorization, CounterStorage};
    use serial_test::serial;
    use std::sync::Arc;
    use std::time::Duration;

    // Regression test for the bug fixed by adding `NX` to `SCRIPT_UPDATE_COUNTER`'s
    // `expire` call: `Reserve` pre-creates a counter's key at value 0 to start its window,
    // so the first real `update_counter` after it (here, via `Commit`) must not treat that
    // as "a fresh key" and re-anchor the TTL - the window `Reserve` already started must be
    // preserved.
    #[test]
    #[serial]
    fn reserve_then_update_counter_does_not_extend_the_ttl() {
        let storage = RedisStorage::default();
        storage.clear().unwrap();

        let namespace = "reserve_then_update_ttl_test";
        let limit = Arc::new(Limit::new(
            namespace,
            10,
            60,
            vec![],
            Vec::<Expression>::default(),
        ));
        let ctx = Context::default();
        let mut counters = vec![Counter::new(limit, &ctx).unwrap().unwrap()];

        let reservation_id = ReservationId::new();
        let auth = storage
            .reserve(
                &mut counters,
                &reservation_id,
                1,
                Duration::from_secs(60),
                false,
            )
            .unwrap();
        assert!(matches!(auth, Authorization::Ok));

        let key = key_for_counter(&counters[0]);
        let mut con = storage.conn_pool.get().unwrap();
        let ttl_after_reserve: i64 = redis::cmd("PTTL").arg(&key).query(&mut *con).unwrap();
        assert!(
            ttl_after_reserve > 0,
            "expected Reserve to have already started the counter's window"
        );

        // A real, if brief, sleep so a "window reset" would show up as the TTL going back up
        // towards the full 60s, not just measurement noise around an unchanged value.
        std::thread::sleep(Duration::from_millis(50));

        storage.update_counter(&counters[0], 1).unwrap();

        let ttl_after_update: i64 = redis::cmd("PTTL").arg(&key).query(&mut *con).unwrap();
        assert!(
            ttl_after_update <= ttl_after_reserve,
            "the counter's real first update must not re-anchor the window Reserve already \
             started: ttl_after_reserve={ttl_after_reserve}ms, ttl_after_update={ttl_after_update}ms"
        );
    }

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
