use mpz_ot_core::kos::msgs::Message;

use crate::OTError;

/// A KOS sender error.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum SenderError {
    #[error(transparent)]
    IOError(#[from] std::io::Error),
    #[error(transparent)]
    CoreError(#[from] mpz_ot_core::kos::SenderError),
    #[error(transparent)]
    BaseOTError(#[from] crate::OTError),
    #[error(transparent)]
    CointossError(#[from] mpz_core::cointoss::CointossError),
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

impl<BaseMsg> From<enum_try_as_inner::Error<Message<BaseMsg>>> for SenderError {
    fn from(value: enum_try_as_inner::Error<Message<BaseMsg>>) -> Self {
        SenderError::from(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            value.to_string(),
        ))
    }
}

/// A KOS receiver error.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum ReceiverError {
    #[error(transparent)]
    IOError(#[from] std::io::Error),
    #[error(transparent)]
    CoreError(#[from] mpz_ot_core::kos::ReceiverError),
    #[error(transparent)]
    BaseOTError(#[from] crate::OTError),
    #[error(transparent)]
    CointossError(#[from] mpz_core::cointoss::CointossError),
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

impl<BaseMsg> From<enum_try_as_inner::Error<Message<BaseMsg>>> for ReceiverError {
    fn from(value: enum_try_as_inner::Error<Message<BaseMsg>>) -> Self {
        ReceiverError::from(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            value.to_string(),
        ))
    }
}
