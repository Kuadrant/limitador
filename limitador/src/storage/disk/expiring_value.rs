use crate::storage::StorageErr;
use sled::IVec;
use std::array::TryFromSliceError;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub struct ExpiringValue {
    value: i64,
    expiry: SystemTime,
}

impl ExpiringValue {
    pub fn new(value: i64, expiry: SystemTime) -> Self {
        Self { value, expiry }
    }

    pub fn value_at(&self, when: SystemTime) -> i64 {
        if self.expiry <= when {
            return 0;
        }
        self.value
    }

    pub fn value(&self) -> i64 {
        self.value_at(SystemTime::now())
    }

    pub fn update(self, delta: i64, ttl: u64) -> Self {
        let now = SystemTime::now();

        let expiry = if self.expiry <= now {
            now + Duration::from_secs(ttl)
        } else {
            self.expiry
        };

        let value = self.value_at(now) + delta;
        Self { value, expiry }
    }

    pub fn ttl(&self) -> Duration {
        self.expiry
            .duration_since(SystemTime::now())
            .unwrap_or(Duration::ZERO)
    }
}

impl TryFrom<&[u8]> for ExpiringValue {
    type Error = TryFromSliceError;

    fn try_from(raw: &[u8]) -> Result<Self, Self::Error> {
        let raw_val: [u8; 8] = raw[0..8].try_into()?;
        let raw_exp: [u8; 8] = raw[8..16].try_into()?;

        let val = i64::from_be_bytes(raw_val);
        let exp = u64::from_be_bytes(raw_exp);

        Ok(Self {
            value: val,
            expiry: UNIX_EPOCH + Duration::from_secs(exp),
        })
    }
}

impl From<ExpiringValue> for IVec {
    fn from(value: ExpiringValue) -> Self {
        let val: [u8; 8] = value.value.to_be_bytes();
        let exp: [u8; 8] = value
            .expiry
            .duration_since(UNIX_EPOCH)
            .expect("Can't expire before Epoch")
            .as_secs()
            .to_be_bytes();
        IVec::from([val, exp].concat())
    }
}

impl From<TryFromSliceError> for StorageErr {
    fn from(_: TryFromSliceError) -> Self {
        Self {
            msg: "Corrupted byte sequence while reading 8 bytes for 64-bit integer".to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ExpiringValue;
    use sled::IVec;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    #[test]
    fn returns_value_when_valid() {
        let now = SystemTime::now();
        let val = ExpiringValue::new(42, now);
        assert_eq!(val.value_at(now - Duration::from_secs(1)), 42);
    }

    #[test]
    fn returns_default_when_expired() {
        let now = SystemTime::now();
        let val = ExpiringValue::new(42, now - Duration::from_secs(1));
        assert_eq!(val.value_at(now), 0);
    }

    #[test]
    fn returns_default_on_expiry() {
        let now = SystemTime::now();
        let val = ExpiringValue::new(42, now);
        assert_eq!(val.value_at(now), 0);
    }

    #[test]
    fn updates_when_valid() {
        let now = SystemTime::now();
        let val = ExpiringValue::new(42, now + Duration::from_secs(1)).update(3, 10);
        assert_eq!(val.value_at(now - Duration::from_secs(1)), 45);
    }

    #[test]
    fn updates_when_expired() {
        let now = SystemTime::now();
        let val = ExpiringValue::new(42, now);
        assert_eq!(val.ttl(), Duration::ZERO);
        let val = val.update(3, 10);
        assert_eq!(val.value_at(now - Duration::from_secs(1)), 3);
    }

    #[test]
    fn from_into_ivec() {
        let now = SystemTime::now();
        let val = ExpiringValue::new(42, now);
        let raw: IVec = val.into();
        let back: ExpiringValue = raw.as_ref().try_into().unwrap();

        assert_eq!(back.value, 42);
        assert_eq!(
            back.expiry.duration_since(UNIX_EPOCH).unwrap().as_secs(),
            now.duration_since(UNIX_EPOCH).unwrap().as_secs()
        );
    }
}
