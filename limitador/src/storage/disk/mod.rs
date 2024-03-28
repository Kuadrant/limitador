use crate::storage::StorageErr;
use rocksdb::ErrorKind;

mod expiring_value;
mod rocksdb_storage;

pub use rocksdb_storage::RocksDbStorage as DiskStorage;

impl From<rocksdb::Error> for StorageErr {
    fn from(error: rocksdb::Error) -> Self {
        Self {
            msg: format!("Underlying storage error: {error}"),
            transient: error.kind() == ErrorKind::TimedOut || error.kind() == ErrorKind::TryAgain,
        }
    }
}

#[derive(PartialEq, Eq, Debug)]
pub enum OptimizeFor {
    Space,
    Throughput,
}
