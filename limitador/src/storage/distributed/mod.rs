use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::ToSocketAddrs;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::sync::mpsc;
use tokio::sync::mpsc::Sender;
use tracing::debug;

use crate::counter::Counter;
use crate::limit::{Context, Limit};
use crate::storage::distributed::cr_counter_value::CrCounterValue;
use crate::storage::distributed::grpc::v1::CounterUpdate;
use crate::storage::distributed::grpc::{Broker, CounterEntry};
use crate::storage::keys::bin::key_for_counter_v2;
use crate::storage::{Authorization, CounterStorage, StorageErr};

mod cr_counter_value;
#[allow(clippy::result_large_err)]
mod grpc;

pub type LimitsMap = HashMap<Vec<u8>, Arc<CounterEntry>>;

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
            value = counter_value.value.read()
        }
        Ok(counter.max_value() >= value + delta)
    }

    #[tracing::instrument(skip_all)]
    fn add_counter(&self, limit: &Limit) -> Result<(), StorageErr> {
        if limit.variables().is_empty() {
            let mut limits = self.limits.write().unwrap();
            let key = encode_limit_to_key(limit);
            limits.entry(key.clone()).or_insert(Arc::new(CounterEntry {
                key,
                counter: Counter::new(limit.clone(), &Context::default())
                    .expect("counter creation can't fail! no vars to resolve!")
                    .expect("must have a counter"),
                value: CrCounterValue::new(
                    self.identifier.clone(),
                    limit.max_value(),
                    Duration::from_secs(limit.seconds()),
                ),
            }));
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
                let value = Arc::new(CounterEntry {
                    key: key.clone(),
                    counter: counter.clone(),
                    value: CrCounterValue::new(
                        self.identifier.clone(),
                        counter.max_value(),
                        duration,
                    ),
                });
                self.increment_counter(value.clone(), delta, now);
                entry.insert(value);
            }
            Entry::Occupied(entry) => {
                self.increment_counter(entry.get().clone(), delta, now);
            }
        };
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    fn check_and_update(
        &self,
        counters: &mut Vec<Counter>,
        delta: u64,
        check: bool,
        update: bool,
        load_counters: bool,
    ) -> Result<Authorization, StorageErr> {
        let mut first_limited = None;
        let mut counter_values_to_update: Vec<Vec<u8>> = Vec::new();
        let now = SystemTime::now();

        let mut process_counter =
            |counter: &mut Counter, value: u64, delta: u64| -> () {
                let remaining = counter.max_value().checked_sub(value + delta);
                if load_counters {
                    counter.set_remaining(remaining.unwrap_or(0));
                }
                if first_limited.is_none() && remaining.is_none() {
                    first_limited = Some(Authorization::Limited(
                        counter.limit().name().map(|n| n.to_owned()),
                    ));
                }
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
                        process_counter(counter, store_value.value.read(), delta);
                        if update {
                            counter_values_to_update.push(key);
                        }
                        true
                    }
                }
            };

            // we need to take the slow path since we need to mutate the limits map.
            if !counter_existed {
                // try again with a write lock to create the counter if it's still missing.
                let mut limits = self.limits.write().unwrap();
                let store_value = limits.entry(key.clone()).or_insert(Arc::new(CounterEntry {
                    key: key.clone(),
                    counter: counter.clone(),
                    value: CrCounterValue::new(
                        self.identifier.clone(),
                        counter.max_value(),
                        counter.window(),
                    ),
                }));

                process_counter(counter, store_value.value.read(), delta);
                if update {
                    counter_values_to_update.push(key);
                }
            }
        }

        if update {
            // Update counters
            let limits = self.limits.read().unwrap();
            counter_values_to_update.into_iter().for_each(|key| {
                let store_value = limits.get(&key).unwrap();
                self.increment_counter(store_value.clone(), delta, now);
            });
        }

        if check {
            if let Some(limited) = first_limited {
                return Ok(limited);
            }
        }
        
        Ok(Authorization::Ok)
    }

    #[tracing::instrument(skip_all)]
    fn get_counters(&self, limits: &HashSet<Arc<Limit>>) -> Result<HashSet<Counter>, StorageErr> {
        let mut res = HashSet::new();
        let limits_map = self.limits.read().unwrap();
        for (_, counter_entry) in limits_map.iter() {
            if limits.contains(counter_entry.counter.limit()) {
                let mut counter: Counter = counter_entry.counter.clone();
                counter.set_remaining(counter.max_value() - counter_entry.value.read());
                counter.set_expires_in(counter_entry.value.ttl());
                if counter.expires_in().unwrap() > Duration::ZERO {
                    res.insert(counter);
                }
            }
        }
        Ok(res)
    }

    #[tracing::instrument(skip_all)]
    fn delete_counters(&self, limits: &HashSet<Arc<Limit>>) -> Result<(), StorageErr> {
        for limit in limits {
            self.delete_counters_of_limit(limit);
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
                value
                    .value
                    .merge((UNIX_EPOCH + Duration::from_secs(update.expires_at), values).into());
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

    fn increment_counter(&self, counter_entry: Arc<CounterEntry>, delta: u64, when: SystemTime) {
        counter_entry
            .value
            .inc_at(delta, counter_entry.counter.window(), when);
        self.broker.publish(counter_entry)
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
                let (expiry, ourself, value) = store_value.value.local_values();
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

fn encode_counter_to_key(counter: &Counter) -> Vec<u8> {
    key_for_counter_v2(counter)
}

fn encode_limit_to_key(limit: &Limit) -> Vec<u8> {
    // fixme this is broken!
    let vars: HashMap<String, String> = limit
        .variables()
        .into_iter()
        .map(|k| (k, "".to_string()))
        .collect();
    let ctx = vars.into();
    let counter = Counter::new(limit.clone(), &ctx)
        .expect("counter creation can't fail! faked vars!")
        .expect("must have a counter");
    key_for_counter_v2(&counter)
}
