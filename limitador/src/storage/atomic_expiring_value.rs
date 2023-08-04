use std::sync::atomic::Ordering;
use std::sync::atomic::{AtomicI64, AtomicU64};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug)]
pub(crate) struct AtomicExpiringValue {
    value: AtomicI64,
    expiry: AtomicU64, // in microseconds
}

impl AtomicExpiringValue {
    pub fn new(value: i64, expiry: SystemTime) -> Self {
        let expiry = Self::get_duration_micros(expiry);
        Self {
            value: AtomicI64::new(value),
            expiry: AtomicU64::new(expiry),
        }
    }

    pub fn value_at(&self, when: SystemTime) -> i64 {
        let when = Self::get_duration_micros(when);
        let expiry = self.expiry.load(Ordering::SeqCst);
        if expiry <= when {
            return 0;
        }
        self.value.load(Ordering::SeqCst)
    }

    pub fn value(&self) -> i64 {
        self.value_at(SystemTime::now())
    }

    pub fn update(&self, delta: i64, ttl: u64, when: SystemTime) -> i64 {
        let ttl_micros = ttl * 1_000_000;
        let when_micros = Self::get_duration_micros(when);

        let expiry = self.expiry.load(Ordering::SeqCst);
        if expiry <= when_micros {
            let new_expiry = when_micros + ttl_micros;
            if self
                .expiry
                .compare_exchange(expiry, new_expiry, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                self.value.store(delta, Ordering::SeqCst);
            }
            return delta;
        }
        self.value.fetch_add(delta, Ordering::SeqCst) + delta
    }

    pub fn ttl(&self) -> Duration {
        let expiry =
            SystemTime::UNIX_EPOCH + Duration::from_micros(self.expiry.load(Ordering::SeqCst));
        expiry
            .duration_since(SystemTime::now())
            .unwrap_or(Duration::ZERO)
    }

    fn get_duration_micros(when: SystemTime) -> u64 {
        when.duration_since(UNIX_EPOCH)
            .expect("SystemTime before UNIX EPOCH!")
            .as_micros() as u64
    }
}

impl Default for AtomicExpiringValue {
    fn default() -> Self {
        AtomicExpiringValue {
            value: AtomicI64::new(0),
            expiry: AtomicU64::new(0),
        }
    }
}

impl Clone for AtomicExpiringValue {
    fn clone(&self) -> Self {
        AtomicExpiringValue {
            value: AtomicI64::new(self.value.load(Ordering::SeqCst)),
            expiry: AtomicU64::new(self.expiry.load(Ordering::SeqCst)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::{Duration, SystemTime};

    #[test]
    fn returns_value_when_valid() {
        let now = SystemTime::now();
        let val = AtomicExpiringValue::new(42, now);
        assert_eq!(val.value_at(now - Duration::from_secs(1)), 42);
    }

    #[test]
    fn returns_default_when_expired() {
        let now = SystemTime::now();
        let val = AtomicExpiringValue::new(42, now - Duration::from_secs(1));
        assert_eq!(val.value_at(now), 0);
    }

    #[test]
    fn returns_default_on_expiry() {
        let now = SystemTime::now();
        let val = AtomicExpiringValue::new(42, now);
        assert_eq!(val.value_at(now), 0);
    }

    #[test]
    fn updates_when_valid() {
        let now = SystemTime::now();
        let val = AtomicExpiringValue::new(42, now + Duration::from_secs(1));
        val.update(3, 10, now);
        assert_eq!(val.value_at(now - Duration::from_secs(1)), 45);
    }

    #[test]
    fn updates_when_expired() {
        let now = SystemTime::now();
        let val = AtomicExpiringValue::new(42, now);
        assert_eq!(val.ttl(), Duration::ZERO);
        val.update(3, 10, now);
        assert_eq!(val.value_at(now - Duration::from_secs(1)), 3);
    }

    #[test]
    fn test_overlapping_updates() {
        let now = SystemTime::now();
        let atomic_expiring_value = AtomicExpiringValue::new(42, now + Duration::from_secs(10));

        thread::scope(|s| {
            s.spawn(|| {
                atomic_expiring_value.update(1, 1, now);
            });
            s.spawn(|| {
                atomic_expiring_value.update(2, 1, now + Duration::from_secs(11));
            });
        });
        assert!([2i64, 3i64].contains(&atomic_expiring_value.value.load(Ordering::SeqCst)));
    }

    #[test]
    fn size_of_struct() {
        // This is ugly, but we don't have access to `AtomicExpiringValue` in the server,
        // so this is hardcoded in main.rs
        assert_eq!(16, std::mem::size_of::<AtomicExpiringValue>());
    }
}
