use crate::counter::Counter;
use crate::storage::redis::AsyncRedisStorage;
use crate::storage::AsyncCounterStorage;
use std::collections::HashMap;

pub struct Batcher {
    accumulated_counter_updates: HashMap<Counter, i64>,
    redis_storage: AsyncRedisStorage,
}

impl Batcher {
    pub fn new(redis_storage: AsyncRedisStorage) -> Self {
        Self {
            accumulated_counter_updates: HashMap::new(),
            redis_storage,
        }
    }

    pub async fn add_counter(&mut self, counter: &Counter, delta: i64) {
        match self.accumulated_counter_updates.get_mut(counter) {
            Some(val) => {
                *val += delta;
            }
            None => {
                self.accumulated_counter_updates
                    .insert(counter.clone(), delta);
            }
        }
    }

    pub async fn flush(&mut self) {
        for (counter, delta) in self.accumulated_counter_updates.iter() {
            self.redis_storage
                .update_counter(counter, *delta)
                .await
                .unwrap();
        }
        self.accumulated_counter_updates.clear();
    }
}
