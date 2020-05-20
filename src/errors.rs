use crate::storage::StorageErr;
use thiserror::Error;

#[derive(Error, Debug, Eq, PartialEq)]
pub enum LimitadorError {
    #[error("error while accessing the limits storage: {0:?}")]
    Storage(String),
    #[error("missing namespace")]
    MissingNamespace,
}

impl From<StorageErr> for LimitadorError {
    fn from(e: StorageErr) -> Self {
        LimitadorError::Storage(e.msg().to_owned())
    }
}
