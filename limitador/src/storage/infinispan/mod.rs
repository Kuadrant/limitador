mod counters;
mod dist_lock;
mod infinispan_storage;
mod response;
mod sets;

use crate::storage::StorageErr;
pub use counters::Consistency;
use infinispan::errors::InfinispanError;
pub use infinispan_storage::InfinispanStorage;
pub use infinispan_storage::InfinispanStorageBuilder;

impl From<reqwest::Error> for StorageErr {
    fn from(e: reqwest::Error) -> Self {
        Self { msg: e.to_string() }
    }
}

impl From<InfinispanError> for StorageErr {
    fn from(e: InfinispanError) -> Self {
        Self { msg: e.to_string() }
    }
}

pub const DEFAULT_INFINISPAN_LIMITS_CACHE_NAME: &str = "limitador";
pub const DEFAULT_INFINISPAN_CONSISTENCY: Consistency = Consistency::Strong;
