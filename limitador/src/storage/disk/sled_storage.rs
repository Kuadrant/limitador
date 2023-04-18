use crate::counter::Counter;
use crate::limit::Limit;
use crate::storage::disk::expiring_value::ExpiringValue;
use crate::storage::disk::OptimizeFor;
use crate::storage::keys::{
    key_for_counter, partial_counter_from_counter_key, prefix_for_namespace,
};
use crate::storage::{Authorization, CounterStorage, StorageErr};
use sled::{Config, Db, IVec, Mode};
use std::collections::{BTreeSet, HashSet};
use std::time::{Duration, SystemTime};

pub struct SledStorage {
    db: Db,
}

impl CounterStorage for SledStorage {
    fn is_within_limits(&self, counter: &Counter, delta: i64) -> Result<bool, StorageErr> {
        let key = key_for_counter(counter);
        let value = self.insert_or_update(&key, counter, 0)?;
        Ok(counter.max_value() >= value.value() + delta)
    }

    fn update_counter(&self, counter: &Counter, delta: i64) -> Result<(), StorageErr> {
        let key = key_for_counter(counter);
        self.insert_or_update(&key, counter, delta)?;
        Ok(())
    }

    fn check_and_update(
        &self,
        counters: &mut Vec<Counter>,
        delta: i64,
        load_counters: bool,
    ) -> Result<Authorization, StorageErr> {
        let mut keys: Vec<Vec<u8>> = Vec::with_capacity(counters.len());

        for counter in &mut *counters {
            let key = key_for_counter(counter);
            let (val, ttl) = match self.db.get(&key)? {
                None => (0, Duration::from_secs(counter.limit().seconds())),
                Some(raw) => {
                    let value: ExpiringValue = raw.as_ref().try_into()?;
                    (value.value(), value.ttl())
                }
            };

            if load_counters {
                counter.set_expires_in(ttl);
                counter.set_remaining(counter.max_value() - val - delta);
            }

            if counter.max_value() < val + delta {
                return Ok(Authorization::Limited(
                    counter.limit().name().map(|n| n.to_string()),
                ));
            }

            keys.push(key);
        }

        for (idx, counter) in counters.iter_mut().enumerate() {
            self.insert_or_update(&keys[idx], counter, delta)?;
        }

        Ok(Authorization::Ok)
    }

    fn get_counters(&self, limits: &HashSet<Limit>) -> Result<HashSet<Counter>, StorageErr> {
        let mut counters = HashSet::default();
        let namepaces: BTreeSet<&str> = limits.iter().map(|l| l.namespace().as_ref()).collect();
        for ns in namepaces {
            for entry in self.db.range(prefix_for_namespace(ns)..) {
                let (key, value) = entry?;
                let raw = String::from_utf8_lossy(key.as_ref());
                if !raw.starts_with(&prefix_for_namespace(ns)) {
                    break;
                }
                let mut counter = partial_counter_from_counter_key(raw.as_bytes(), ns);
                let value: ExpiringValue = value.as_ref().try_into()?;
                for limit in limits {
                    if limit == counter.limit() {
                        counter.update_to_limit(limit);
                        let ttl = value.ttl();
                        counter.set_expires_in(ttl);
                        counter.set_remaining(limit.max_value() - value.value());
                        break;
                    }
                }
                if counter.expires_in().expect("Duration needs to be set") > Duration::ZERO {
                    counters.insert(counter);
                }
            }
        }
        Ok(counters)
    }

    fn delete_counters(&self, limits: HashSet<Limit>) -> Result<(), StorageErr> {
        let counters = self.get_counters(&limits)?;
        for counter in &counters {
            self.db.remove(key_for_counter(counter))?;
        }
        Ok(())
    }

    fn clear(&self) -> Result<(), StorageErr> {
        Ok(self.db.clear()?)
    }
}

impl From<OptimizeFor> for Mode {
    fn from(value: OptimizeFor) -> Self {
        match value {
            OptimizeFor::Space => Mode::LowSpace,
            OptimizeFor::Throughput => Mode::HighThroughput,
        }
    }
}

impl SledStorage {
    pub fn open<P: AsRef<std::path::Path>>(path: P, mode: OptimizeFor) -> Result<Self, StorageErr> {
        let db = Config::new().mode(mode.into()).path(path).open()?;
        Ok(Self { db })
    }

    fn insert_or_update(
        &self,
        key: &[u8],
        counter: &Counter,
        delta: i64,
    ) -> Result<ExpiringValue, StorageErr> {
        Ok(self.db.update_and_fetch(key, |prev| {
            let updated_value = match prev {
                Some(raw) => {
                    let value: ExpiringValue = match TryInto::<ExpiringValue>::try_into(raw) {
                        Ok(val) => val.update(delta, counter.seconds()),
                        Err(_) => ExpiringValue::new(
                            delta,
                            SystemTime::now() + Duration::from_secs(counter.limit().seconds()),
                        ),
                    };
                    value
                }
                None => ExpiringValue::new(
                    delta,
                    SystemTime::now() + Duration::from_secs(counter.limit().seconds()),
                ),
            };
            Some::<IVec>(updated_value.into())
        })?)
        .map(|option| {
            option
                .expect("we always have a counter now!")
                .as_ref()
                .try_into()
                .expect("This has to work!")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::SledStorage;
    use crate::counter::Counter;
    use crate::limit::Limit;
    use crate::storage::disk::OptimizeFor;
    use crate::storage::CounterStorage;
    use std::collections::HashMap;
    use std::fs;
    use std::time::Duration;
    use tempdir::TempDir;

    #[test]
    fn opens_db_on_disk() {
        let namespace = "test_namespace";
        let limit = Limit::new(namespace, 1, 2, vec!["req.method == 'GET'"], vec!["app_id"]);
        let counter = Counter::new(limit, HashMap::default());

        let tmp = TempDir::new("limitador-disk-tests").expect("We should have a dir!");
        {
            let storage = SledStorage::open(tmp.path(), OptimizeFor::Space)
                .expect("We should have a storage");
            let mut files = fs::read_dir(tmp.as_ref()).expect("Couldn't access data dir");
            assert!(files.next().is_some());

            assert!(
                storage.is_within_limits(&counter, 1).unwrap(),
                "Should be a fresh value"
            );
            assert!(
                storage.is_within_limits(&counter, 1).unwrap(),
                "Should be from the store, yet still below threshold"
            );
            std::thread::sleep(Duration::from_secs(2));
            assert!(
                storage.update_counter(&counter, 1).is_ok(),
                "Should have written the counter to disk"
            );
            assert!(
                !storage.is_within_limits(&counter, 1).unwrap(),
                "Should now be above threshold!"
            );
        }

        {
            let storage = SledStorage::open(tmp.path(), OptimizeFor::Space)
                .expect("We should still have a storage");
            assert!(
                !storage.is_within_limits(&counter, 1).unwrap(),
                "Should be above threshold still!"
            );
        }
    }
}
