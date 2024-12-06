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
