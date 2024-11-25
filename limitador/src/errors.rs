use crate::storage::StorageErr;
use std::error::Error;
use std::fmt::{Display, Formatter};

#[derive(Debug)]
pub enum LimitadorError {
    Storage(StorageErr),
}

impl Display for LimitadorError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            LimitadorError::Storage(err) => {
                write!(f, "error while accessing the limits storage: {err:?}")
            }
        }
    }
}

impl Error for LimitadorError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            LimitadorError::Storage(err) => Some(err),
        }
    }
}

impl From<StorageErr> for LimitadorError {
    fn from(e: StorageErr) -> Self {
        Self::Storage(e)
    }
}
