use std::collections::btree_map::Entry;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;
use std::time::{Duration, SystemTime};

use crate::storage::atomic_expiring_value::AtomicExpiryTime;

#[derive(Debug)]
pub struct CrCounterValue<A: Ord> {
    ourselves: A,
    max_value: u64,
    value: AtomicU64,
    others: RwLock<BTreeMap<A, u64>>,
    expiry: AtomicExpiryTime,
}

#[allow(dead_code)]
impl<A: Ord> CrCounterValue<A> {
    pub fn new(actor: A, max_value: u64, time_window: Duration) -> Self {
        Self {
            ourselves: actor,
            max_value,
            value: Default::default(),
            others: RwLock::default(),
            expiry: AtomicExpiryTime::new(SystemTime::now() + time_window),
        }
    }

    pub fn max_value(&self) -> u64 {
        self.max_value
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
        if self.expiry.update_if_expired(time_window, when) {
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
            if self.expiry.update_if_expired(time_window, when) {
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
        let (expiry, other_values) = other.into_inner();
        if expiry > when {
            let _ = self.expiry.merge_at(expiry.into(), when);
            if self.expiry.expired_at(when) {
                self.reset(expiry);
            }
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
                            if other_value > 0 {
                                entry.insert(other_value);
                            }
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

    pub fn ttl(&self) -> Duration {
        self.expiry.ttl()
    }

    pub fn expiry(&self) -> SystemTime {
        self.expiry.expires_at()
    }

    pub fn into_inner(self) -> (SystemTime, BTreeMap<A, u64>) {
        let Self {
            ourselves,
            max_value: _,
            value,
            others,
            expiry,
        } = self;
        let mut map = others.into_inner().unwrap();
        map.insert(ourselves, value.into_inner());
        (expiry.into_inner(), map)
    }

    pub fn local_values(&self) -> (SystemTime, &A, u64) {
        (
            self.expiry.clone().into_inner(),
            &self.ourselves,
            self.value.load(Ordering::Relaxed),
        )
    }

    fn reset(&self, expiry: SystemTime) {
        let mut guard = self.others.write().unwrap();
        self.expiry.update(expiry);
        self.value.store(0, Ordering::SeqCst);
        guard.clear()
    }
}

impl<A: Clone + Ord> Clone for CrCounterValue<A> {
    fn clone(&self) -> Self {
        Self {
            ourselves: self.ourselves.clone(),
            max_value: self.max_value,
            value: AtomicU64::new(self.value.load(Ordering::SeqCst)),
            others: RwLock::new(self.others.read().unwrap().clone()),
            expiry: self.expiry.clone(),
        }
    }
}

impl<A: Clone + Ord + Default> From<(SystemTime, BTreeMap<A, u64>)> for CrCounterValue<A> {
    fn from(value: (SystemTime, BTreeMap<A, u64>)) -> Self {
        Self {
            ourselves: A::default(),
            max_value: 0,
            value: Default::default(),
            others: RwLock::new(value.1),
            expiry: value.0.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use crate::storage::distributed::cr_counter_value::CrCounterValue;

    #[test]
    fn local_increments_are_readable() {
        let window = Duration::from_secs(1);
        let a = CrCounterValue::new('A', u64::MAX, window);
        a.inc(3, window);
        assert_eq!(3, a.read());
        a.inc(2, window);
        assert_eq!(5, a.read());
    }

    #[test]
    fn local_increments_expire() {
        let window = Duration::from_secs(1);
        let a = CrCounterValue::new('A', u64::MAX, window);
        let now = SystemTime::now();
        a.inc_at(3, window, now);
        assert_eq!(3, a.read());
        a.inc_at(2, window, now + window);
        assert_eq!(2, a.read());
    }

    #[test]
    fn other_increments_are_readable() {
        let window = Duration::from_secs(1);
        let a = CrCounterValue::new('A', u64::MAX, window);
        a.inc_actor('B', 3, window);
        assert_eq!(3, a.read());
        a.inc_actor('B', 2, window);
        assert_eq!(5, a.read());
    }

    #[test]
    fn other_increments_expire() {
        let window = Duration::from_secs(1);
        let a = CrCounterValue::new('A', u64::MAX, window);
        let now = SystemTime::now();
        a.inc_actor_at('B', 3, window, now);
        assert_eq!(3, a.read());
        a.inc_actor_at('B', 2, window, now + window);
        assert_eq!(2, a.read());
    }

    #[test]
    fn merges() {
        let window = Duration::from_secs(1);
        let a = CrCounterValue::new('A', u64::MAX, window);
        let b = CrCounterValue::new('B', u64::MAX, window);
        a.inc(3, window);
        b.inc(2, window);
        a.merge(b);
        assert_eq!(a.read(), 5);
    }

    #[test]
    fn merges_symetric() {
        let window = Duration::from_secs(1);
        let a = CrCounterValue::new('A', u64::MAX, window);
        let b = CrCounterValue::new('B', u64::MAX, window);
        a.inc(3, window);
        b.inc(2, window);
        b.merge(a);
        assert_eq!(b.read(), 5);
    }

    #[test]
    fn merges_overrides_with_larger_value() {
        let window = Duration::from_secs(1);
        let a = CrCounterValue::new('A', u64::MAX, window);
        let b = CrCounterValue::new('B', u64::MAX, window);
        a.inc(3, window);
        b.inc(2, window);
        b.inc_actor('A', 2, window); // older value!
        b.merge(a); // merges the 3
        assert_eq!(b.read(), 5);
    }

    #[test]
    fn merges_ignore_lesser_values() {
        let window = Duration::from_secs(1);
        let a = CrCounterValue::new('A', u64::MAX, window);
        let b = CrCounterValue::new('B', u64::MAX, window);
        a.inc(3, window);
        b.inc(2, window);
        b.inc_actor('A', 5, window); // newer value!
        b.merge(a); // ignores the 3 and keeps its own 5 for a
        assert_eq!(b.read(), 7);
    }

    #[test]
    fn merge_ignores_expired_sets() {
        let window = Duration::from_secs(1);
        let a = CrCounterValue::new('A', u64::MAX, Duration::ZERO);
        a.inc(3, Duration::ZERO);
        let b = CrCounterValue::new('B', u64::MAX, window);
        b.inc(2, window);
        b.merge(a);
        assert_eq!(b.read(), 2);
    }

    #[test]
    fn merge_ignores_expired_sets_symmetric() {
        let window = Duration::from_secs(1);
        let a = CrCounterValue::new('A', u64::MAX, Duration::ZERO);
        a.inc(3, Duration::ZERO);
        let b = CrCounterValue::new('B', u64::MAX, window);
        b.inc(2, window);
        a.merge(b);
        assert_eq!(a.read(), 2);
    }

    #[test]
    fn merge_uses_earliest_expiry() {
        let later = Duration::from_secs(1);
        let a = CrCounterValue::new('A', u64::MAX, later);
        let sooner = Duration::from_millis(200);
        let b = CrCounterValue::new('B', u64::MAX, sooner);
        a.inc(3, later);
        b.inc(2, later);
        a.merge(b);
        assert!(a.expiry.ttl() < sooner);
    }
}
