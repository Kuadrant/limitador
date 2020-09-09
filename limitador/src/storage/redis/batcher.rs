use crate::counter::Counter;
use crate::storage::redis::redis_sync::RedisStorage;
use crate::storage::Storage;
use std::collections::HashMap;
use std::sync::Mutex;

pub struct Batcher {
    accumulated_counter_updates: Mutex<HashMap<Counter, i64>>,
    redis_storage: RedisStorage,
}

impl Batcher {
    pub fn new(redis_storage: RedisStorage) -> Batcher {
        Batcher {
            accumulated_counter_updates: Mutex::new(HashMap::new()),
            redis_storage,
        }
    }

    pub fn add_counter(&self, counter: &Counter, delta: i64) {
        let mut accumulated_counter_updates = self.accumulated_counter_updates.lock().unwrap();

        match accumulated_counter_updates.get_mut(counter) {
            Some(val) => {
                *val += delta;
            }
            None => {
                accumulated_counter_updates.insert(counter.clone(), delta);
            }
        }
    }

    pub fn flush(&self) {
        let mut accumulated_counter_updates = self.accumulated_counter_updates.lock().unwrap();

        for (counter, delta) in accumulated_counter_updates.iter() {
            self.redis_storage.update_counter(counter, *delta).unwrap();
        }
        accumulated_counter_updates.clear();
    }
}
