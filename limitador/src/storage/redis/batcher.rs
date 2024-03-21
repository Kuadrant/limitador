use crate::counter::Counter;
use std::collections::HashMap;
use std::mem;

pub struct Batcher {
    accumulated_counter_updates: HashMap<Counter, i64>,
}

impl Batcher {
    pub fn new() -> Self {
        Self {
            accumulated_counter_updates: HashMap::new(),
        }
    }

    pub fn add_counter(&mut self, counter: &Counter, delta: i64) {
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

    pub fn flush(&mut self) -> HashMap<Counter, i64> {
        mem::take(&mut self.accumulated_counter_updates)
    }
}
