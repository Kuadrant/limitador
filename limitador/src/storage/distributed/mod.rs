use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::ToSocketAddrs;
use std::ops::Deref;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use moka::sync::Cache;
use serde::{Deserialize, Serialize};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::sync::mpsc::Sender;

use crate::counter::Counter;
use crate::limit::{Limit, Namespace};
use crate::storage::distributed::cr_counter_value::CrCounterValue;
use crate::storage::{Authorization, CounterStorage, StorageErr};

mod cr_counter_value;

type NamespacedLimitCounters<T> = HashMap<Namespace, HashMap<Limit, T>>;

pub struct CrInMemoryStorage {
    identifier: String,
    sender: Sender<CounterValueMessage>,
    limits_for_namespace: Arc<RwLock<NamespacedLimitCounters<CrCounterValue<String>>>>,
    qualified_counters: Arc<Cache<Counter, Arc<CrCounterValue<String>>>>,
}

impl CounterStorage for CrInMemoryStorage {
    #[tracing::instrument(skip_all)]
    fn is_within_limits(&self, counter: &Counter, delta: u64) -> Result<bool, StorageErr> {
        let limits_by_namespace = self.limits_for_namespace.read().unwrap();

        let mut value = 0;

        if counter.is_qualified() {
            if let Some(counter) = self.qualified_counters.get(counter) {
                value = counter.read();
            }
        } else if let Some(limits) = limits_by_namespace.get(counter.limit().namespace()) {
            if let Some(counter) = limits.get(counter.limit()) {
                value = counter.read();
            }
        }

        Ok(counter.max_value() >= value + delta)
    }

    #[tracing::instrument(skip_all)]
    fn add_counter(&self, limit: &Limit) -> Result<(), StorageErr> {
        if limit.variables().is_empty() {
            let mut limits_by_namespace = self.limits_for_namespace.write().unwrap();
            limits_by_namespace
                .entry(limit.namespace().clone())
                .or_default()
                .entry(limit.clone())
                .or_insert(CrCounterValue::new(
                    self.identifier.clone(),
                    Duration::from_secs(limit.seconds()),
                ));
        }
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    fn update_counter(&self, counter: &Counter, delta: u64) -> Result<(), StorageErr> {
        let mut limits_by_namespace = self.limits_for_namespace.write().unwrap();
        let now = SystemTime::now();
        if counter.is_qualified() {
            let value = match self.qualified_counters.get(counter) {
                None => self.qualified_counters.get_with(counter.clone(), || {
                    Arc::new(CrCounterValue::new(
                        self.identifier.clone(),
                        Duration::from_secs(counter.seconds()),
                    ))
                }),
                Some(counter) => counter,
            };
            self.increment_counter(counter.clone(), &value, delta, now);
        } else {
            match limits_by_namespace.entry(counter.limit().namespace().clone()) {
                Entry::Vacant(v) => {
                    let mut limits = HashMap::new();
                    let duration = Duration::from_secs(counter.seconds());
                    let counter_val = CrCounterValue::new(self.identifier.clone(), duration);
                    self.increment_counter(counter.clone(), &counter_val, delta, now);
                    limits.insert(counter.limit().clone(), counter_val);
                    v.insert(limits);
                }
                Entry::Occupied(mut o) => match o.get_mut().entry(counter.limit().clone()) {
                    Entry::Vacant(v) => {
                        let duration = Duration::from_secs(counter.seconds());
                        let counter_value = CrCounterValue::new(self.identifier.clone(), duration);
                        self.increment_counter(counter.clone(), &counter_value, delta, now);
                        v.insert(counter_value);
                    }
                    Entry::Occupied(o) => {
                        self.increment_counter(counter.clone(), o.get(), delta, now);
                    }
                },
            }
        }
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    fn check_and_update(
        &self,
        counters: &mut Vec<Counter>,
        delta: u64,
        load_counters: bool,
    ) -> Result<Authorization, StorageErr> {
        let limits_by_namespace = self.limits_for_namespace.write().unwrap();
        let mut first_limited = None;
        let mut counter_values_to_update: Vec<(&CrCounterValue<String>, Counter)> = Vec::new();
        let mut qualified_counter_values_to_updated: Vec<(Arc<CrCounterValue<String>>, Counter)> =
            Vec::new();
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
        for counter in counters.iter_mut().filter(|c| !c.is_qualified()) {
            let atomic_expiring_value: &CrCounterValue<String> = limits_by_namespace
                .get(counter.limit().namespace())
                .and_then(|limits| limits.get(counter.limit()))
                .unwrap();

            if let Some(limited) = process_counter(counter, atomic_expiring_value.read(), delta) {
                if !load_counters {
                    return Ok(limited);
                }
            }
            counter_values_to_update.push((atomic_expiring_value, counter.clone()));
        }

        // Process qualified counters
        for counter in counters.iter_mut().filter(|c| c.is_qualified()) {
            let value = match self.qualified_counters.get(counter) {
                None => self.qualified_counters.get_with(counter.clone(), || {
                    Arc::new(CrCounterValue::new(
                        self.identifier.clone(),
                        Duration::from_secs(counter.seconds()),
                    ))
                }),
                Some(counter) => counter,
            };

            if let Some(limited) = process_counter(counter, value.read(), delta) {
                if !load_counters {
                    return Ok(limited);
                }
            }

            qualified_counter_values_to_updated.push((value, counter.clone()));
        }

        if let Some(limited) = first_limited {
            return Ok(limited);
        }

        // Update counters
        counter_values_to_update
            .into_iter()
            .for_each(|(v, counter)| {
                self.increment_counter(counter, v, delta, now);
            });
        qualified_counter_values_to_updated
            .into_iter()
            .for_each(|(v, counter)| {
                self.increment_counter(counter, v.deref(), delta, now);
            });

        Ok(Authorization::Ok)
    }

    #[tracing::instrument(skip_all)]
    fn get_counters(&self, limits: &HashSet<Limit>) -> Result<HashSet<Counter>, StorageErr> {
        let mut res = HashSet::new();

        let namespaces: HashSet<&Namespace> = limits.iter().map(Limit::namespace).collect();
        let limits_by_namespace = self.limits_for_namespace.read().unwrap();

        for namespace in namespaces {
            if let Some(limits) = limits_by_namespace.get(namespace) {
                for limit in limits.keys() {
                    if limits.contains_key(limit) {
                        for (counter, expiring_value) in self.counters_in_namespace(namespace) {
                            let mut counter_with_val = counter.clone();
                            counter_with_val.set_remaining(
                                counter_with_val.max_value() - expiring_value.read(),
                            );
                            counter_with_val.set_expires_in(expiring_value.ttl());
                            if counter_with_val.expires_in().unwrap() > Duration::ZERO {
                                res.insert(counter_with_val);
                            }
                        }
                    }
                }
            }
        }

        for (counter, expiring_value) in self.qualified_counters.iter() {
            if limits.contains(counter.limit()) {
                let mut counter_with_val = counter.deref().clone();
                counter_with_val
                    .set_remaining(counter_with_val.max_value() - expiring_value.read());
                counter_with_val.set_expires_in(expiring_value.ttl());
                if counter_with_val.expires_in().unwrap() > Duration::ZERO {
                    res.insert(counter_with_val);
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
        self.limits_for_namespace.write().unwrap().clear();
        Ok(())
    }
}

impl CrInMemoryStorage {
    pub fn new(
        identifier: String,
        cache_size: u64,
        local: String,
        broadcast: Option<String>,
    ) -> Self {
        let (sender, mut rx) = mpsc::channel(1000);

        let local = local.to_socket_addrs().unwrap().next().unwrap();
        if let Some(remote) = broadcast.clone() {
            tokio::spawn(async move {
                let sock = UdpSocket::bind(local).await.unwrap();
                sock.set_broadcast(true).unwrap();
                sock.connect(remote).await.unwrap();
                loop {
                    let message: CounterValueMessage = rx.recv().await.unwrap();
                    let buf = postcard::to_stdvec(&message).unwrap();
                    match sock.send(&buf).await {
                        Ok(len) => {
                            if len != buf.len() {
                                println!("Couldn't send complete message!");
                            }
                        }
                        Err(err) => println!("Couldn't send update: {:?}", err),
                    };
                }
            });
        }

        let limits_for_namespace = Arc::new(RwLock::new(HashMap::<
            Namespace,
            HashMap<Limit, CrCounterValue<String>>,
        >::new()));
        let qualified_counters: Arc<Cache<Counter, Arc<CrCounterValue<String>>>> =
            Arc::new(Cache::new(cache_size));

        {
            let limits_for_namespace = limits_for_namespace.clone();
            let qualified_counters = qualified_counters.clone();

            if let Some(broadcast) = broadcast.clone() {
                tokio::spawn(async move {
                    let sock = UdpSocket::bind(broadcast).await.unwrap();
                    sock.set_broadcast(true).unwrap();
                    let mut buf = [0; 1024];
                    loop {
                        let (len, addr) = sock.recv_from(&mut buf).await.unwrap();
                        if addr != local {
                            match postcard::from_bytes::<CounterValueMessage>(&buf[..len]) {
                                Ok(message) => {
                                    let CounterValueMessage {
                                        counter_key,
                                        expiry,
                                        values,
                                    } = message;
                                    let counter = <CounterKey as Into<Counter>>::into(counter_key);
                                    if counter.is_qualified() {
                                        if let Some(counter) = qualified_counters.get(&counter) {
                                            counter.merge(
                                                (UNIX_EPOCH + Duration::from_secs(expiry), values)
                                                    .into(),
                                            );
                                        }
                                    } else {
                                        let counters = limits_for_namespace.read().unwrap();
                                        let limits = counters.get(counter.namespace()).unwrap();
                                        let value = limits.get(counter.limit()).unwrap();
                                        value.merge(
                                            (UNIX_EPOCH + Duration::from_secs(expiry), values)
                                                .into(),
                                        );
                                    };
                                }
                                Err(err) => {
                                    println!(
                                        "Error from {} bytes: {:?} \n{:?}",
                                        len,
                                        err,
                                        &buf[..len]
                                    )
                                }
                            }
                        }
                    }
                });
            }
        }

        Self {
            identifier,
            sender,
            limits_for_namespace,
            qualified_counters,
        }
    }

    fn counters_in_namespace(
        &self,
        namespace: &Namespace,
    ) -> HashMap<Counter, CrCounterValue<String>> {
        let mut res: HashMap<Counter, CrCounterValue<String>> = HashMap::new();

        if let Some(counters_by_limit) = self.limits_for_namespace.read().unwrap().get(namespace) {
            for (limit, value) in counters_by_limit {
                res.insert(
                    Counter::new(limit.clone(), HashMap::default()),
                    value.clone(),
                );
            }
        }

        for (counter, value) in self.qualified_counters.iter() {
            if counter.namespace() == namespace {
                res.insert(counter.deref().clone(), value.deref().clone());
            }
        }

        res
    }

    fn delete_counters_of_limit(&self, limit: &Limit) {
        if let Some(counters_by_limit) = self
            .limits_for_namespace
            .write()
            .unwrap()
            .get_mut(limit.namespace())
        {
            counters_by_limit.remove(limit);
        }
    }

    fn counter_is_within_limits(counter: &Counter, current_val: Option<&u64>, delta: u64) -> bool {
        match current_val {
            Some(current_val) => current_val + delta <= counter.max_value(),
            None => counter.max_value() >= delta,
        }
    }

    fn increment_counter(
        &self,
        key: Counter,
        counter: &CrCounterValue<String>,
        delta: u64,
        when: SystemTime,
    ) {
        counter.inc_at(delta, Duration::from_secs(key.seconds()), when);
        let sender = self.sender.clone();
        let counter = counter.clone();
        tokio::spawn(async move {
            let (expiry, values) = counter.into_inner();
            let message = CounterValueMessage {
                counter_key: key.into(),
                expiry: expiry.duration_since(UNIX_EPOCH).unwrap().as_secs(),
                values,
            };
            sender.send(message).await
        });
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct CounterValueMessage {
    counter_key: CounterKey,
    expiry: u64,
    values: BTreeMap<String, u64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CounterKey {
    namespace: Namespace,
    seconds: u64,
    conditions: HashSet<String>,
    variables: HashSet<String>,
    vars: HashMap<String, String>,
}

impl From<Counter> for CounterKey {
    fn from(value: Counter) -> Self {
        Self {
            namespace: value.namespace().clone(),
            seconds: value.seconds(),
            variables: value.limit().variables(),
            conditions: value.limit().conditions(),
            vars: value.set_variables().clone(),
        }
    }
}

impl From<CounterKey> for Counter {
    fn from(value: CounterKey) -> Self {
        Self::new(
            Limit::new(
                value.namespace,
                0,
                value.seconds,
                value.conditions,
                value.vars.keys(),
            ),
            value.vars,
        )
    }
}
