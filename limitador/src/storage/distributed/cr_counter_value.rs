use crate::storage::atomic_expiring_value::AtomicExpiryTime;
use std::collections::btree_map::Entry;
use std::collections::BTreeMap;
use std::ops::Not;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;
use std::time::{Duration, SystemTime};

struct CrCounterValue<A: Ord> {
    ourselves: A,
    value: AtomicU64,
    others: RwLock<BTreeMap<A, u64>>,
    expiry: AtomicExpiryTime,
}

impl<A: Ord> CrCounterValue<A> {
    pub fn new(actor: A, time_window: Duration) -> Self {
        Self {
            ourselves: actor,
            value: Default::default(),
            others: RwLock::default(),
            expiry: AtomicExpiryTime::from_now(time_window),
        }
    }

    pub fn read(&self) -> u64 {
        self.read_at(SystemTime::now())
    }

    pub fn read_at(&self, when: SystemTime) -> u64 {
        if self.expiry.expired_at(when) {
            0
        } else {
            let guard = self.others.read().unwrap();
            let others: u64 = guard.values().sum();
            others + self.value.load(Ordering::Relaxed)
        }
    }

    pub fn inc(&self, increment: u64, time_window: Duration) {
        self.inc_at(increment, time_window, SystemTime::now())
    }

    pub fn inc_at(&self, increment: u64, time_window: Duration, when: SystemTime) {
        if self
            .expiry
            .update_if_expired(time_window.as_micros() as u64, when)
        {
            self.value.store(increment, Ordering::SeqCst);
        } else {
            self.value.fetch_add(increment, Ordering::SeqCst);
        }
    }

    pub fn inc_actor(&self, actor: A, increment: u64, time_window: Duration) {
        self.inc_actor_at(actor, increment, time_window, SystemTime::now());
    }

    pub fn inc_actor_at(&self, actor: A, increment: u64, time_window: Duration, when: SystemTime) {
        if actor == self.ourselves {
            self.inc_at(increment, time_window, when);
        } else {
            let mut guard = self.others.write().unwrap();
            if self
                .expiry
                .update_if_expired(time_window.as_micros() as u64, when)
            {
                guard.insert(actor, increment);
            } else {
                *guard.entry(actor).or_insert(0) += increment;
            }
        }
    }

    pub fn merge(&self, other: Self) {
        self.merge_at(other, SystemTime::now());
    }

    pub fn merge_at(&self, other: Self, when: SystemTime) {
        if self.expiry.expired_at(when).not() && other.expiry.expired_at(when).not() {
            let (expiry, other_values) = other.into_inner();
            let _ = self.expiry.merge_at(AtomicExpiryTime::new(expiry), when);
            let ourselves = self.value.load(Ordering::SeqCst);
            let mut others = self.others.write().unwrap();
            for (actor, other_value) in other_values {
                if actor == self.ourselves {
                    if other_value > ourselves {
                        self.value
                            .fetch_add(other_value - ourselves, Ordering::SeqCst);
                    }
                } else {
                    match others.entry(actor) {
                        Entry::Vacant(entry) => {
                            entry.insert(other_value);
                        }
                        Entry::Occupied(mut known) => {
                            let local = known.get_mut();
                            if other_value > *local {
                                *local = other_value;
                            }
                        }
                    }
                }
            }
        }
    }

    fn into_inner(self) -> (SystemTime, BTreeMap<A, u64>) {
        let Self {
            ourselves,
            value,
            others,
            expiry,
        } = self;
        let mut map = others.into_inner().unwrap();
        map.insert(ourselves, value.into_inner());
        (expiry.into_inner(), map)
    }
}

#[cfg(test)]
mod tests {
    use crate::storage::distributed::cr_counter_value::CrCounterValue;
    use std::time::{Duration, SystemTime};

    #[test]
    fn local_increments_are_readable() {
        let window = Duration::from_secs(1);
        let a = CrCounterValue::new('A', window);
        a.inc(3, window);
        assert_eq!(3, a.read());
        a.inc(2, window);
        assert_eq!(5, a.read());
    }

    #[test]
    fn local_increments_expire() {
        let window = Duration::from_secs(1);
        let a = CrCounterValue::new('A', window);
        let now = SystemTime::now();
        a.inc_at(3, window, now);
        assert_eq!(3, a.read());
        a.inc_at(2, window, now + window);
        assert_eq!(2, a.read());
    }

    #[test]
    fn other_increments_are_readable() {
        let window = Duration::from_secs(1);
        let a = CrCounterValue::new('A', window);
        a.inc_actor('B', 3, window);
        assert_eq!(3, a.read());
        a.inc_actor('B', 2, window);
        assert_eq!(5, a.read());
    }

    #[test]
    fn other_increments_expire() {
        let window = Duration::from_secs(1);
        let a = CrCounterValue::new('A', window);
        let now = SystemTime::now();
        a.inc_actor_at('B', 3, window, now);
        assert_eq!(3, a.read());
        a.inc_actor_at('B', 2, window, now + window);
        assert_eq!(2, a.read());
    }

    #[test]
    fn merges() {
        let window = Duration::from_secs(1);
        {
            let a = CrCounterValue::new('A', window);
            let b = CrCounterValue::new('B', window);
            a.inc(3, window);
            b.inc(2, window);
            a.merge(b);
            assert_eq!(a.read(), 5);
        }

        {
            let a = CrCounterValue::new('A', window);
            let b = CrCounterValue::new('B', window);
            a.inc(3, window);
            b.inc(2, window);
            b.merge(a);
            assert_eq!(b.read(), 5);
        }

        {
            let a = CrCounterValue::new('A', window);
            let b = CrCounterValue::new('B', window);
            a.inc(3, window);
            b.inc(2, window);
            b.inc_actor('A', 2, window); // older value!
            b.merge(a); // merges the 3
            assert_eq!(b.read(), 5);
        }

        {
            let a = CrCounterValue::new('A', window);
            let b = CrCounterValue::new('B', window);
            a.inc(3, window);
            b.inc(2, window);
            b.inc_actor('A', 5, window); // newer value!
            b.merge(a); // ignores the 3 and keeps its own 5 for a
            assert_eq!(b.read(), 7);
        }
    }
}
