use std::fmt::{Debug, Display};

pub trait Value: Debug + Clone + Send + Sync + IsZero + IsTrue {}

#[derive(Debug, thiserror::Error)]
#[error("value error: kind {kind}, {msg}")]
pub struct ValueError {
    kind: ValueErrorKind,
    msg: String,
}

#[derive(Debug)]
pub enum ValueErrorKind {}

impl Display for ValueErrorKind {
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            _ => Ok(()),
        }
    }
}

pub trait IsZero {
    fn is_zero(&self) -> Result<bool, ValueError>;
}

pub trait IsTrue {
    fn is_true(&self) -> Result<bool, ValueError>;
}
