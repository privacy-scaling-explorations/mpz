use mpz_ot_core::chou_orlandi::msgs::Message;

use crate::OTError;

/// A Chou-Orlandi sender error.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum SenderError {
    #[error(transparent)]
    IOError(#[from] std::io::Error),
    #[error(transparent)]
    CoreError(#[from] mpz_ot_core::chou_orlandi::SenderError),
    #[error("invalid state: expected {0}")]
    StateError(String),
}

impl From<SenderError> for OTError {
    fn from(err: SenderError) -> Self {
        match err {
            SenderError::IOError(e) => e.into(),
            e => OTError::SenderError(Box::new(e)),
        }
    }
}

impl From<enum_try_as_inner::Error<Message>> for SenderError {
    fn from(value: enum_try_as_inner::Error<Message>) -> Self {
        SenderError::from(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            value.to_string(),
        ))
    }
}

/// A Chou-Orlandi receiver error.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum ReceiverError {
    #[error(transparent)]
    IOError(#[from] std::io::Error),
    #[error(transparent)]
    CoreError(#[from] mpz_ot_core::chou_orlandi::ReceiverError),
    #[error("invalid state: expected {0}")]
    StateError(String),
}

impl From<ReceiverError> for OTError {
    fn from(err: ReceiverError) -> Self {
        match err {
            ReceiverError::IOError(e) => e.into(),
            e => OTError::ReceiverError(Box::new(e)),
        }
    }
}

impl From<enum_try_as_inner::Error<Message>> for ReceiverError {
    fn from(value: enum_try_as_inner::Error<Message>) -> Self {
        ReceiverError::from(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            value.to_string(),
        ))
    }
}
