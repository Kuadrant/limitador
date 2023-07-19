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
        let expiry = expiry
            .duration_since(UNIX_EPOCH)
            .expect("SystemTime before UNIX EPOCH!")
            .as_micros() as u64;
        Self {
            value: AtomicI64::new(value),
            expiry: AtomicU64::new(expiry),
        }
    }

    pub fn value_at(&self, when: SystemTime) -> i64 {
        let expiry = self.expiry.load(Ordering::SeqCst);
        let when = when
            .duration_since(UNIX_EPOCH)
            .expect("SystemTime before UNIX EPOCH!")
            .as_micros() as u64;
        if expiry <= when {
            return 0;
        }
        self.value.load(Ordering::SeqCst)
    }

    pub fn value(&self) -> i64 {
        self.value_at(SystemTime::now())
    }

    pub fn update(&self, delta: i64, ttl: u64, when: SystemTime) {
        let new = self.value_at(when) + delta;
        self.value.swap(new, Ordering::SeqCst);

        let ttl_micros = ttl * 1_000_000;
        let when_micros = when
            .duration_since(UNIX_EPOCH)
            .expect("SystemTime before UNIX EPOCH!")
            .as_micros() as u64;
        let expiry = self.expiry.load(Ordering::SeqCst);
        if expiry <= when_micros {
            self.expiry.swap(when_micros + ttl_micros, Ordering::SeqCst);
        }
    }

    pub fn ttl(&self) -> Duration {
        let expiry =
            SystemTime::UNIX_EPOCH + Duration::from_micros(self.expiry.load(Ordering::SeqCst));
        expiry
            .duration_since(SystemTime::now())
            .unwrap_or(Duration::ZERO)
    }
}

impl Default for AtomicExpiringValue {
    fn default() -> Self {
        AtomicExpiringValue {
            value: AtomicI64::new(0),
            expiry: AtomicU64::new(
                UNIX_EPOCH
                    .duration_since(UNIX_EPOCH)
                    .expect("SystemTime before UNIX EPOCH")
                    .as_micros() as u64,
            ),
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
    use super::AtomicExpiringValue;
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
}
