use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::ToSocketAddrs;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio::sync::mpsc::Sender;
use tracing::debug;

use crate::counter::Counter;
use crate::limit::{Limit, Namespace};
use crate::storage::distributed::cr_counter_value::CrCounterValue;
use crate::storage::distributed::grpc::v1::CounterUpdate;
use crate::storage::distributed::grpc::{Broker, CounterEntry};
use crate::storage::{Authorization, CounterStorage, StorageErr};

mod cr_counter_value;
mod grpc;

pub type LimitsMap = HashMap<Vec<u8>, Arc<CrCounterValue<String>>>;

pub struct CrInMemoryStorage {
    identifier: String,
    limits: Arc<RwLock<LimitsMap>>,
    broker: Broker,
}

impl CounterStorage for CrInMemoryStorage {
    #[tracing::instrument(skip_all)]
    fn is_within_limits(&self, counter: &Counter, delta: u64) -> Result<bool, StorageErr> {
        let limits = self.limits.read().unwrap();

        let mut value = 0;
        let key = encode_counter_to_key(counter);
        if let Some(counter_value) = limits.get(&key) {
            value = counter_value.read()
        }
        Ok(counter.max_value() >= value + delta)
    }

    #[tracing::instrument(skip_all)]
    fn add_counter(&self, limit: &Limit) -> Result<(), StorageErr> {
        if limit.variables().is_empty() {
            let mut limits = self.limits.write().unwrap();
            let key = encode_limit_to_key(limit);
            limits.entry(key).or_insert(Arc::new(CrCounterValue::new(
                self.identifier.clone(),
                limit.max_value(),
                Duration::from_secs(limit.seconds()),
            )));
        }
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    fn update_counter(&self, counter: &Counter, delta: u64) -> Result<(), StorageErr> {
        let mut limits = self.limits.write().unwrap();
        let now = SystemTime::now();

        let key = encode_counter_to_key(counter);
        match limits.entry(key.clone()) {
            Entry::Vacant(entry) => {
                let duration = counter.window();
                let store_value = Arc::new(CrCounterValue::new(
                    self.identifier.clone(),
                    counter.max_value(),
                    duration,
                ));
                self.increment_counter(counter, key, store_value.clone(), delta, now);
                entry.insert(store_value);
            }
            Entry::Occupied(entry) => {
                self.increment_counter(counter, key, entry.get().clone(), delta, now);
            }
        };
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    fn check_and_update(
        &self,
        counters: &mut Vec<Counter>,
        delta: u64,
        load_counters: bool,
    ) -> Result<Authorization, StorageErr> {
        let mut first_limited = None;
        let mut counter_values_to_update: Vec<(Counter, Vec<u8>)> = Vec::new();
        let now = SystemTime::now();

        let mut process_counter =
            |counter: &mut Counter, value: u64, delta: u64| -> Option<Authorization> {
                if load_counters {
                    let remaining = counter.max_value().checked_sub(value + delta);
                    counter.set_remaining(remaining.unwrap_or(0));
                    if first_limited.is_none() && remaining.is_none() {
                        first_limited = Some(Authorization::Limited(
                            counter.limit().name().map(|n| n.to_owned()),
                        ));
                    }
                }
                if !Self::counter_is_within_limits(counter, Some(&value), delta) {
                    return Some(Authorization::Limited(
                        counter.limit().name().map(|n| n.to_owned()),
                    ));
                }
                None
            };

        // Process simple counters
        for counter in counters.iter_mut() {
            let key = encode_counter_to_key(counter);

            // most of the time the counter should exist, so first try with a read only lock
            // since that will allow us to have higher concurrency
            let counter_existed = {
                let key = key.clone();
                let limits = self.limits.read().unwrap();
                match limits.get(&key) {
                    None => false,
                    Some(store_value) => {
                        if let Some(limited) = process_counter(counter, store_value.read(), delta) {
                            if !load_counters {
                                return Ok(limited);
                            }
                        }
                        counter_values_to_update.push((counter.clone(), key));
                        true
                    }
                }
            };

            // we need to take the slow path since we need to mutate the limits map.
            if !counter_existed {
                // try again with a write lock to create the counter if it's still missing.
                let mut limits = self.limits.write().unwrap();
                let store_value =
                    limits
                        .entry(key.clone())
                        .or_insert(Arc::new(CrCounterValue::new(
                            self.identifier.clone(),
                            counter.max_value(),
                            counter.window(),
                        )));

                if let Some(limited) = process_counter(counter, store_value.read(), delta) {
                    if !load_counters {
                        return Ok(limited);
                    }
                }
                counter_values_to_update.push((counter.clone(), key));
            }
        }

        if let Some(limited) = first_limited {
            return Ok(limited);
        }

        // Update counters
        let limits = self.limits.read().unwrap();
        counter_values_to_update
            .into_iter()
            .for_each(|(counter, key)| {
                let store_value = limits.get(&key).unwrap();
                self.increment_counter(&counter, key, store_value.clone(), delta, now);
            });

        Ok(Authorization::Ok)
    }

    #[tracing::instrument(skip_all)]
    fn get_counters(&self, limits: &HashSet<Limit>) -> Result<HashSet<Counter>, StorageErr> {
        let mut res = HashSet::new();

        let limits: HashSet<_> = limits.iter().map(encode_limit_to_key).collect();

        let limits_map = self.limits.read().unwrap();
        for (key, counter_value) in limits_map.iter() {
            let counter_key = decode_counter_key(key).unwrap();
            let limit_key = if !counter_key.vars.is_empty() {
                let mut cloned = counter_key.clone();
                cloned.vars = HashMap::default();
                cloned.encode()
            } else {
                key.clone()
            };

            if limits.contains(&limit_key) {
                let counter = (&counter_key, &*counter_value.clone());
                let mut counter: Counter = counter.into();
                counter.set_remaining(counter.max_value() - counter_value.read());
                counter.set_expires_in(counter_value.ttl());
                if counter.expires_in().unwrap() > Duration::ZERO {
                    res.insert(counter);
                }
            }
        }
        Ok(res)
    }

    #[tracing::instrument(skip_all)]
    fn delete_counters(&self, limits: HashSet<Limit>) -> Result<(), StorageErr> {
        for limit in limits {
            self.delete_counters_of_limit(&limit);
        }
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    fn clear(&self) -> Result<(), StorageErr> {
        self.limits.write().unwrap().clear();
        Ok(())
    }
}

impl CrInMemoryStorage {
    pub fn new(
        identifier: String,
        _cache_size: u64,
        listen_address: String,
        peer_urls: Vec<String>,
    ) -> Self {
        let listen_address = listen_address.to_socket_addrs().unwrap().next().unwrap();
        let peer_urls = peer_urls.clone();
        let limits = Arc::new(RwLock::new(LimitsMap::new()));

        let limits_clone = limits.clone();

        let (re_sync_queue_tx, mut re_sync_queue_rx) = mpsc::channel(100);
        let broker = grpc::Broker::new(
            identifier.clone(),
            listen_address,
            peer_urls,
            Box::pin(move |update: CounterUpdate| {
                let values = BTreeMap::from_iter(
                    update
                        .values
                        .iter()
                        .map(|(k, v)| (k.to_owned(), v.to_owned())),
                );
                let limits = limits_clone.read().unwrap();
                let value = limits.get(&update.key).unwrap();
                value.merge((UNIX_EPOCH + Duration::from_secs(update.expires_at), values).into());
            }),
            re_sync_queue_tx,
        );

        {
            let broker = broker.clone();
            tokio::spawn(async move {
                broker.start().await;
            });
        }

        // process the re-sync requests...
        {
            let limits = limits.clone();
            tokio::spawn(async move {
                while let Some(sender) = re_sync_queue_rx.recv().await {
                    process_re_sync(&limits, sender).await;
                }
            });
        }

        Self {
            identifier,
            limits,
            broker,
        }
    }

    fn delete_counters_of_limit(&self, limit: &Limit) {
        let key = encode_limit_to_key(limit);
        self.limits.write().unwrap().remove(&key);
    }

    fn counter_is_within_limits(counter: &Counter, current_val: Option<&u64>, delta: u64) -> bool {
        match current_val {
            Some(current_val) => current_val + delta <= counter.max_value(),
            None => counter.max_value() >= delta,
        }
    }

    fn increment_counter(
        &self,
        counter: &Counter,
        store_key: Vec<u8>,
        store_value: Arc<CrCounterValue<String>>,
        delta: u64,
        when: SystemTime,
    ) {
        store_value.inc_at(delta, counter.window(), when);
        self.broker
            .publish(CounterEntry::new(store_key, store_value))
    }
}

async fn process_re_sync(limits: &Arc<RwLock<LimitsMap>>, sender: Sender<Option<CounterUpdate>>) {
    // sending all the counters to the peer might take a while, so we don't want to lock
    // the limits map for too long, lets figure first get the list of keys that needs to be sent.
    let keys: Vec<_> = {
        let limits = limits.read().unwrap();
        limits.keys().cloned().collect()
    };

    for key in keys {
        let update = {
            let limits = limits.read().unwrap();
            limits.get(&key).and_then(|store_value| {
                let (expiry, ourself, value) = store_value.local_values();
                if value == 0 || expiry <= SystemTime::now() {
                    None // no point in sending a counter that is empty
                } else {
                    let values = HashMap::from([(ourself.clone(), value)]);
                    Some(CounterUpdate {
                        key: key.clone(),
                        values,
                        expires_at: expiry.duration_since(UNIX_EPOCH).unwrap().as_secs(),
                    })
                }
            })
        };
        // skip None, it means the counter was deleted.
        if let Some(update) = update {
            match sender.send(Some(update)).await {
                Ok(_) => {}
                Err(err) => {
                    debug!("Failed to send re-sync counter update to peer: {:?}", err);
                    break;
                }
            }
        }
    }
    // signal the end of the re-sync
    _ = sender.send(None).await;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CounterKey {
    namespace: Namespace,
    seconds: u64,
    conditions: HashSet<String>,
    variables: HashSet<String>,
    vars: HashMap<String, String>,
}

impl CounterKey {
    fn new(limit: &Limit, vars: HashMap<String, String>) -> Self {
        CounterKey {
            namespace: limit.namespace().clone(),
            seconds: limit.seconds(),
            variables: limit.variables().clone(),
            conditions: limit.conditions().clone(),
            vars,
        }
    }

    fn encode(&self) -> Vec<u8> {
        postcard::to_stdvec(self).unwrap()
    }
}

impl From<(&CounterKey, &CrCounterValue<String>)> for Counter {
    fn from(value: (&CounterKey, &CrCounterValue<String>)) -> Self {
        let (counter_key, store_value) = value;
        let max_value = store_value.max_value();
        let mut counter = Self::new(
            Limit::new(
                counter_key.namespace.clone(),
                max_value,
                counter_key.seconds,
                counter_key.conditions.clone(),
                counter_key.vars.keys(),
            ),
            counter_key.vars.clone(),
        );
        counter.set_remaining(max_value - store_value.read());
        counter.set_expires_in(store_value.ttl());
        counter
    }
}

fn encode_counter_to_key(counter: &Counter) -> Vec<u8> {
    let key = CounterKey::new(counter.limit(), counter.set_variables().clone());
    postcard::to_stdvec(&key).unwrap()
}

fn encode_limit_to_key(limit: &Limit) -> Vec<u8> {
    let key = CounterKey::new(limit, HashMap::default());
    postcard::to_stdvec(&key).unwrap()
}

fn decode_counter_key(key: &Vec<u8>) -> postcard::Result<CounterKey> {
    postcard::from_bytes(key.as_slice())
}
