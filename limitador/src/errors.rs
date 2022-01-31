use crate::storage::StorageErr;
use thiserror::Error;

#[derive(Error, Debug, Eq, PartialEq)]
pub enum LimitadorError {
    #[error("error while accessing the limits storage: {0:?}")]
    Storage(String),
}

impl From<StorageErr> for LimitadorError {
    fn from(e: StorageErr) -> Self {
        Self::Storage(e.msg().to_owned())
    }
}
