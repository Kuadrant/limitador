use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug)]
pub(crate) struct AtomicExpiringValue {
    value: AtomicU64,
    expiry: AtomicExpiryTime,
}

impl AtomicExpiringValue {
    pub fn new(value: u64, expiry: SystemTime) -> Self {
        Self {
            value: AtomicU64::new(value),
            expiry: AtomicExpiryTime::new(expiry),
        }
    }

    pub fn value_at(&self, when: SystemTime) -> u64 {
        if self.expiry.expired_at(when) {
            return 0;
        }
        self.value.load(Ordering::SeqCst)
    }

    pub fn value(&self) -> u64 {
        self.value_at(SystemTime::now())
    }

    #[allow(dead_code)]
    pub fn add_and_set_expiry(&self, delta: u64, expire_at: SystemTime) -> u64 {
        self.expiry.update(expire_at);
        self.value.fetch_add(delta, Ordering::SeqCst) + delta
    }

    pub fn update(&self, delta: u64, ttl: u64, when: SystemTime) -> u64 {
        if self.expiry.update_if_expired(ttl, when) {
            self.value.store(delta, Ordering::SeqCst);
            return delta;
        }
        self.value.fetch_add(delta, Ordering::SeqCst) + delta
    }

    pub fn ttl(&self) -> Duration {
        self.expiry.duration()
    }
}

#[derive(Debug)]
pub struct AtomicExpiryTime {
    expiry: AtomicU64, // in microseconds
}

impl AtomicExpiryTime {
    pub fn new(when: SystemTime) -> Self {
        let expiry = Self::since_epoch(when);
        Self {
            expiry: AtomicU64::new(expiry),
        }
    }

    #[allow(dead_code)]
    pub fn from_now(ttl: Duration) -> Self {
        Self::new(SystemTime::now() + ttl)
    }

    fn since_epoch(when: SystemTime) -> u64 {
        when.duration_since(UNIX_EPOCH)
            .expect("SystemTime before UNIX EPOCH!")
            .as_micros() as u64
    }

    pub fn duration(&self) -> Duration {
        let expiry =
            SystemTime::UNIX_EPOCH + Duration::from_micros(self.expiry.load(Ordering::SeqCst));
        expiry
            .duration_since(SystemTime::now())
            .unwrap_or(Duration::ZERO)
    }

    pub fn expired_at(&self, when: SystemTime) -> bool {
        let when = Self::since_epoch(when);
        self.expiry.load(Ordering::SeqCst) <= when
    }

    #[allow(dead_code)]
    pub fn update(&self, expiry: SystemTime) {
        self.expiry
            .store(Self::since_epoch(expiry), Ordering::SeqCst);
    }

    pub fn update_if_expired(&self, ttl: u64, when: SystemTime) -> bool {
        let ttl_micros = ttl * 1_000_000;
        let when_micros = Self::since_epoch(when);
        let expiry = self.expiry.load(Ordering::SeqCst);
        if expiry <= when_micros {
            let new_expiry = when_micros + ttl_micros;
            return self
                .expiry
                .compare_exchange(expiry, new_expiry, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok();
        }
        false
    }

    #[allow(dead_code)]
    pub fn merge(&self, other: Self) {
        let mut other = other;
        loop {
            let now = SystemTime::now();
            other = match self.merge_at(other, now) {
                Ok(_) => return,
                Err(other) => other,
            };
        }
    }

    pub fn merge_at(&self, other: Self, when: SystemTime) -> Result<(), Self> {
        let other_exp = other.expiry.load(Ordering::SeqCst);
        let expiry = self.expiry.load(Ordering::SeqCst);
        if other_exp < expiry && other_exp > Self::since_epoch(when) {
            // if our expiry changed, some thread observed the time window as elapsed...
            // `other` can't be in the future anymore! Safely ignoring the failure scenario
            return match self.expiry.compare_exchange(
                expiry,
                other_exp,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => Ok(()),
                Err(_) => Err(other),
            };
        }
        Ok(())
    }

    pub fn into_inner(self) -> SystemTime {
        self.expires_at()
    }

    pub fn expires_at(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_micros(self.expiry.load(Ordering::SeqCst))
    }
}

impl Clone for AtomicExpiryTime {
    fn clone(&self) -> Self {
        Self {
            expiry: AtomicU64::new(self.expiry.load(Ordering::SeqCst)),
        }
    }
}

impl Default for AtomicExpiringValue {
    fn default() -> Self {
        AtomicExpiringValue {
            value: AtomicU64::new(0),
            expiry: AtomicExpiryTime::new(UNIX_EPOCH),
        }
    }
}

impl Clone for AtomicExpiringValue {
    fn clone(&self) -> Self {
        AtomicExpiringValue {
            value: AtomicU64::new(self.value.load(Ordering::SeqCst)),
            expiry: self.expiry.clone(),
        }
    }
}

impl From<SystemTime> for AtomicExpiryTime {
    fn from(value: SystemTime) -> Self {
        AtomicExpiryTime::new(value)
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
        assert!([2u64, 3u64].contains(&atomic_expiring_value.value.load(Ordering::SeqCst)));
    }

    #[test]
    fn size_of_struct() {
        // This is ugly, but we don't have access to `AtomicExpiringValue` in the server,
        // so this is hardcoded in main.rs
        assert_eq!(16, std::mem::size_of::<AtomicExpiringValue>());
    }
}
