use std::fmt::{Debug, Display};

pub trait Value: Debug + Clone + Send + Sync + IsZero + IsTrue {}

#[derive(Debug, thiserror::Error)]
#[error("value error: kind {kind}, {msg}")]
pub struct ValueError {
    kind: ValueErrorKind,
    msg: String,
}

impl ValueError {
    /// Creates a new value error.
    pub fn new(kind: ValueErrorKind, msg: impl Into<String>) -> Self {
        Self {
            kind,
            msg: msg.into(),
        }
    }

    /// Returns the corresponding [`ValueErrorKind`] for this error.
    pub fn kind(&self) -> ValueErrorKind {
        self.kind
    }

    /// Returns the inner error.
    pub fn into_inner(self) -> String {
        self.msg
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum ValueErrorKind {
    /// Attempted to perform an operation on an unsupported type.
    UnsupportedOperation,
}

impl Display for ValueErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ValueErrorKind::UnsupportedOperation => write!(f, "unsupported operation"),
        }
    }
}

pub trait IsZero {
    fn is_zero(&self) -> Result<bool, ValueError>;
}

pub trait IsTrue {
    fn is_true(&self) -> Result<bool, ValueError>;
}
