//! Errors that can occur when using the SPCOT.

/// Errors that can occur when using the SPCOT sender.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum SenderError {
    #[error("invalid state: expected {0}")]
    InvalidLength(String),
}

/// Errors that can occur when using the SPCOT receiver.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum ReceiverError {}
