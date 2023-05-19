use crate::counter::Counter;
use crate::limit::Limit;
use crate::storage::disk::expiring_value::ExpiringValue;
use crate::storage::disk::OptimizeFor;
use crate::storage::keys::bin::{
    key_for_counter, partial_counter_from_counter_key, prefix_for_namespace,
};
use crate::storage::{Authorization, CounterStorage, StorageErr};
use rocksdb::{
    CompactionDecision, DBCompressionType, DBWithThreadMode, IteratorMode, MultiThreaded, Options,
    DB,
};
use std::collections::{BTreeSet, HashSet};
use std::time::{Duration, SystemTime};

pub struct RocksDbStorage {
    db: DBWithThreadMode<MultiThreaded>,
}

impl CounterStorage for RocksDbStorage {
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
            let slice: &[u8] = key.as_ref();
            let (val, ttl) = match self.db.get(slice)? {
                None => (0, Duration::from_secs(counter.limit().seconds())),
                Some(raw) => {
                    let slice: &[u8] = raw.as_ref();
                    let value: ExpiringValue = slice.try_into()?;
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
            for entry in self.db.prefix_iterator(prefix_for_namespace(ns)) {
                let (key, value) = entry?;
                let mut counter = partial_counter_from_counter_key(key.as_ref());
                if counter.namespace().as_ref() != ns {
                    break;
                }
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
            self.db.delete(key_for_counter(counter))?;
        }
        Ok(())
    }

    fn clear(&self) -> Result<(), StorageErr> {
        for entry in self.db.iterator(IteratorMode::Start) {
            self.db.delete(entry?.0)?
        }
        Ok(())
    }
}

impl RocksDbStorage {
    pub fn open<P: AsRef<std::path::Path>>(path: P, mode: OptimizeFor) -> Result<Self, StorageErr> {
        let mut opts = Options::default();
        match mode {
            OptimizeFor::Space => {
                opts.set_compression_type(DBCompressionType::Bz2);
                opts.set_compaction_filter("ExpiredValueFilter", |_level, _key, value| {
                    if let Ok(value) = ExpiringValue::try_from(value) {
                        if value.value_at(SystemTime::now()) != 0 {
                            return CompactionDecision::Keep;
                        }
                    }
                    CompactionDecision::Remove
                });
            }
            OptimizeFor::Throughput => {
                opts.set_compression_type(DBCompressionType::None);
            }
        }
        opts.set_merge_operator_associative("ExpiringValueMerge", |_key, start, operands| {
            let now = SystemTime::now();
            let mut value: ExpiringValue = start
                .map(|raw: &[u8]| raw.try_into().unwrap_or_default())
                .unwrap_or_default();
            for op in operands {
                // ignore (corrupted?) values pending merges
                if let Ok(pending) = ExpiringValue::try_from(op) {
                    value = value.merge(pending, now);
                }
            }
            Some(Vec::from(value))
        });
        opts.create_if_missing(true);
        let db = DB::open(&opts, path).unwrap();
        Ok(Self { db })
    }

    fn insert_or_update(
        &self,
        key: &[u8],
        counter: &Counter,
        delta: i64,
    ) -> Result<ExpiringValue, StorageErr> {
        let now = SystemTime::now();
        let value = match self.db.get(key)? {
            None => ExpiringValue::default(),
            Some(raw) => {
                let slice: &[u8] = raw.as_ref();
                slice.try_into()?
            }
        };
        if value.value_at(now) + delta <= counter.max_value() {
            let expiring_value =
                ExpiringValue::new(delta, now + Duration::from_secs(counter.limit().seconds()));
            self.db
                .merge(key, <ExpiringValue as Into<Vec<u8>>>::into(expiring_value))?;
            return Ok(value.update(delta, counter.seconds(), now));
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::RocksDbStorage;
    use crate::counter::Counter;
    use crate::limit::Limit;
    use crate::storage::disk::OptimizeFor;
    use crate::storage::CounterStorage;
    use std::collections::HashMap;
    use std::fs;
    use std::time::Duration;
    use tempfile::TempDir;

    #[test]
    fn opens_db_on_disk() {
        let namespace = "test_namespace";
        let limit = Limit::new(namespace, 1, 2, vec!["req.method == 'GET'"], vec!["app_id"]);
        let counter = Counter::new(limit, HashMap::default());

        let tmp = TempDir::new().expect("We should have a dir!");
        {
            let storage = RocksDbStorage::open(tmp.path(), OptimizeFor::Space)
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
            let storage = RocksDbStorage::open(tmp.path(), OptimizeFor::Space)
                .expect("We should still have a storage");
            assert!(
                !storage.is_within_limits(&counter, 1).unwrap(),
                "Should be above threshold still!"
            );
        }
    }
}
