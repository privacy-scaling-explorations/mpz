//! Errors that can occur when using the MPCOT protocol.

use crate::ferret::utils::BucketError;
/// Errors that can occur when using the MPCOT sender.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum SenderError {
    #[error("invalid input: expected {0}")]
    InvalidInput(String),
    #[error(transparent)]
    BucketError(#[from] BucketError),
}
