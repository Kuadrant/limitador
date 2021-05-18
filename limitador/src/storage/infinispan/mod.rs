mod counters;
mod dist_lock;
mod infinispan_storage;
mod response;
mod sets;

use crate::storage::StorageErr;
use infinispan::errors::InfinispanError;
pub use infinispan_storage::InfinispanStorage;

impl From<reqwest::Error> for StorageErr {
    fn from(e: reqwest::Error) -> Self {
        StorageErr { msg: e.to_string() }
    }
}

impl From<InfinispanError> for StorageErr {
    fn from(e: InfinispanError) -> Self {
        StorageErr { msg: e.to_string() }
    }
}
