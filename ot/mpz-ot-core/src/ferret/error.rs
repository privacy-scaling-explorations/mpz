//! Errors that can occur when using the Ferret protocol.

/// Errors that can occur when using the Ferret sender.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum SenderError {
    #[error("invalid input: expected {0}")]
    InvalidInput(String),
}

/// Errors that can occur when using the Ferret receiver.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum ReceiverError {
    #[error("invalid input: expected {0}")]
    InvalidInput(String),
}
