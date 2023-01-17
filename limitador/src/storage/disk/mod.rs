use crate::storage::StorageErr;

mod expiring_value;
mod sled_storage;

pub use sled_storage::SledStorage as DiskStorage;

impl From<sled::Error> for StorageErr {
    fn from(error: sled::Error) -> Self {
        Self {
            msg: format!("Underlying storage error: {error}"),
        }
    }
}

pub enum OptimizeFor {
    Space,
    Throughput,
}
