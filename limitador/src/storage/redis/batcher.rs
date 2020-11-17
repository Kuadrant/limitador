use crate::counter::Counter;
use crate::storage::redis::AsyncRedisStorage;
use crate::storage::AsyncStorage;
use std::collections::HashMap;
use tokio::sync::Mutex;

pub struct Batcher {
    accumulated_counter_updates: Mutex<HashMap<Counter, i64>>,
    redis_storage: AsyncRedisStorage,
}

impl Batcher {
    pub fn new(redis_storage: AsyncRedisStorage) -> Batcher {
        Batcher {
            accumulated_counter_updates: Mutex::new(HashMap::new()),
            redis_storage,
        }
    }

    pub async fn add_counter(&self, counter: &Counter, delta: i64) {
        let mut accumulated_counter_updates = self.accumulated_counter_updates.lock().await;

        match accumulated_counter_updates.get_mut(counter) {
            Some(val) => {
                *val += delta;
            }
            None => {
                accumulated_counter_updates.insert(counter.clone(), delta);
            }
        }
    }

    pub async fn flush(&self) {
        let mut accumulated_counter_updates = self.accumulated_counter_updates.lock().await;

        for (counter, delta) in accumulated_counter_updates.iter() {
            self.redis_storage
                .update_counter(counter, *delta)
                .await
                .unwrap();
        }
        accumulated_counter_updates.clear();
    }
}
