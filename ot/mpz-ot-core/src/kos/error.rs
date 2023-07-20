/// Errors that can occur when using the KOS15 sender.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum SenderError {
    #[error("invalid state: expected {0}")]
    InvalidState(String),
    #[error("count mismatch: receiver expected {0}, got {1}")]
    CountMismatch(usize, usize),
    #[error("consistency check failed")]
    ConsistencyCheckFailed,
    #[error("not enough OTs are setup: expected {0}, actual {1}")]
    InsufficientSetup(usize, usize),
}

/// Errors that can occur when using the KOS15 receiver.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum ReceiverError {
    #[error("invalid state: expected {0}")]
    InvalidState(String),
    #[error("count mismatch: receiver expected {0} but sender sent {1}")]
    CountMismatch(usize, usize),
    #[error("invalid payload")]
    InvalidPayload,
    #[error(transparent)]
    ReceiverVerifyError(#[from] ReceiverVerifyError),
}

/// Errors that can occur during verification of the sender's messages.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum ReceiverVerifyError {
    #[error("tape was not recorded")]
    TapeNotRecorded,
    #[error("invalid payload index")]
    InvalidPayloadIndex,
    #[error("payload inconsistent")]
    InconsistentPayload,
}
