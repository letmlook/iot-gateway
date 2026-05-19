//! Flow errors.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum FlowError {
    #[error("validation: {0}")]
    Validation(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("already exists: {0}")]
    AlreadyExists(String),
    #[error("execution: {0}")]
    Execution(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}
