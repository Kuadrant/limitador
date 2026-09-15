use crate::limit::EvaluationError;
use crate::limit::ParseError;
use crate::storage::StorageErr;
use std::convert::Infallible;
use std::error::Error;
use std::fmt::{Display, Formatter};

#[derive(Debug)]
pub enum LimitadorError {
    StorageError(StorageErr),
    InterpreterError(EvaluationError),
}

impl Display for LimitadorError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            LimitadorError::StorageError(err) => {
                write!(f, "error while accessing the limits storage: {err:?}")
            }
            LimitadorError::InterpreterError(err) => {
                write!(f, "error parsing condition: {err:?}")
            }
        }
    }
}

impl Error for LimitadorError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            LimitadorError::StorageError(err) => Some(err),
            LimitadorError::InterpreterError(err) => Some(err),
        }
    }
}

impl LimitadorError {
    /// Whether retrying the same operation later could succeed, as opposed to a permanent
    /// condition (e.g. a storage backend that doesn't support the requested operation).
    pub fn is_transient(&self) -> bool {
        match self {
            LimitadorError::StorageError(err) => err.is_transient(),
            LimitadorError::InterpreterError(_) => false,
        }
    }
}

impl From<StorageErr> for LimitadorError {
    fn from(e: StorageErr) -> Self {
        Self::StorageError(e)
    }
}

impl From<EvaluationError> for LimitadorError {
    fn from(err: EvaluationError) -> Self {
        LimitadorError::InterpreterError(err)
    }
}

impl From<Infallible> for ParseError {
    fn from(value: Infallible) -> Self {
        unreachable!("unexpected infallible value: {:?}", value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_storage_error_is_not_transient() {
        let err: LimitadorError = StorageErr::unsupported("not supported by this backend").into();
        assert!(!err.is_transient());
    }
}
