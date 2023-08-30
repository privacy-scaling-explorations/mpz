use crate::{
    actor::kos::msgs::Message,
    kos::{ReceiverError, SenderError},
};

/// Errors that can occur in the KOS Sender Actor.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
#[non_exhaustive]
pub enum SenderActorError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    SenderError(#[from] SenderError),
    #[error("actor channel error: {0}")]
    Channel(String),
    #[error("{0}")]
    Other(String),
}

impl From<mpz_ot_core::kos::SenderError> for SenderActorError {
    fn from(err: mpz_ot_core::kos::SenderError) -> Self {
        SenderActorError::SenderError(err.into())
    }
}

impl From<crate::OTError> for SenderActorError {
    fn from(err: crate::OTError) -> Self {
        match err {
            crate::OTError::IOError(err) => err.into(),
            err => SenderActorError::Other(err.to_string()),
        }
    }
}

impl From<enum_try_as_inner::Error<crate::kos::SenderState>> for SenderActorError {
    fn from(value: enum_try_as_inner::Error<crate::kos::SenderState>) -> Self {
        SenderError::StateError(value.to_string()).into()
    }
}

impl From<futures::channel::oneshot::Canceled> for SenderActorError {
    fn from(err: futures::channel::oneshot::Canceled) -> Self {
        SenderActorError::Channel(err.to_string())
    }
}

impl<T> From<futures::channel::mpsc::TrySendError<T>> for SenderActorError {
    fn from(err: futures::channel::mpsc::TrySendError<T>) -> Self {
        SenderActorError::Channel(err.to_string())
    }
}

impl<T> From<enum_try_as_inner::Error<Message<T>>> for SenderActorError {
    fn from(value: enum_try_as_inner::Error<Message<T>>) -> Self {
        SenderActorError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            value.to_string(),
        ))
    }
}

impl<T> From<futures::channel::mpsc::TrySendError<T>> for SenderError {
    fn from(err: futures::channel::mpsc::TrySendError<T>) -> Self {
        SenderError::Other(format!("actor channel error: {}", err))
    }
}

impl From<futures::channel::oneshot::Canceled> for SenderError {
    fn from(err: futures::channel::oneshot::Canceled) -> Self {
        SenderError::Other(format!("actor channel canceled: {}", err))
    }
}

/// Errors that can occur in the KOS Receiver Actor.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
#[non_exhaustive]
pub enum ReceiverActorError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    ReceiverError(#[from] ReceiverError),
    #[error("received unexpected transfer id: {0}")]
    UnexpectedTransferId(String),
    #[error("actor channel error: {0}")]
    Channel(String),
    #[error("{0}")]
    Other(String),
}

impl From<mpz_ot_core::kos::ReceiverError> for ReceiverActorError {
    fn from(err: mpz_ot_core::kos::ReceiverError) -> Self {
        ReceiverActorError::ReceiverError(err.into())
    }
}

impl From<crate::OTError> for ReceiverActorError {
    fn from(err: crate::OTError) -> Self {
        match err {
            crate::OTError::IOError(err) => err.into(),
            err => ReceiverActorError::Other(err.to_string()),
        }
    }
}

impl From<enum_try_as_inner::Error<crate::kos::ReceiverState>> for ReceiverActorError {
    fn from(value: enum_try_as_inner::Error<crate::kos::ReceiverState>) -> Self {
        ReceiverError::StateError(value.to_string()).into()
    }
}

impl From<futures::channel::oneshot::Canceled> for ReceiverActorError {
    fn from(err: futures::channel::oneshot::Canceled) -> Self {
        ReceiverActorError::Channel(err.to_string())
    }
}

impl<T> From<futures::channel::mpsc::TrySendError<T>> for ReceiverActorError {
    fn from(err: futures::channel::mpsc::TrySendError<T>) -> Self {
        ReceiverActorError::Channel(err.to_string())
    }
}

impl<T> From<enum_try_as_inner::Error<Message<T>>> for ReceiverActorError {
    fn from(value: enum_try_as_inner::Error<Message<T>>) -> Self {
        ReceiverActorError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            value.to_string(),
        ))
    }
}

impl<T> From<futures::channel::mpsc::TrySendError<T>> for ReceiverError {
    fn from(err: futures::channel::mpsc::TrySendError<T>) -> Self {
        ReceiverError::Other(format!("actor channel error: {}", err))
    }
}

impl From<futures::channel::oneshot::Canceled> for ReceiverError {
    fn from(err: futures::channel::oneshot::Canceled) -> Self {
        ReceiverError::Other(format!("actor channel canceled: {}", err))
    }
}
