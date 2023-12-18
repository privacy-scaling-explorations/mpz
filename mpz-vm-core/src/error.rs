use std::{error::Error, fmt::Display};

use crate::value::ValueError;

/// An error that can occur while executing a data instruction.
#[derive(Debug, thiserror::Error)]
#[error("data instruction error: kind {kind}, {err}")]
pub struct DataInstructionError {
    kind: DataInstructionErrorKind,
    err: Box<dyn Error + Send + Sync>,
}

impl DataInstructionError {
    /// Creates a new data instruction error.
    pub fn new<E>(kind: DataInstructionErrorKind, err: E) -> Self
    where
        E: Into<Box<dyn Error + Send + Sync>>,
    {
        Self {
            kind,
            err: err.into(),
        }
    }

    /// Returns the corresponding [`DataInstructionErrorKind`] for this error.
    pub fn kind(&self) -> DataInstructionErrorKind {
        self.kind
    }

    /// Returns the inner error.
    pub fn into_inner(self) -> Box<dyn Error + Send + Sync> {
        self.err
    }
}

/// A data instruction error kind.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum DataInstructionErrorKind {
    /// An error occurred in the executor.
    Executor,
    /// An error occurred while operating on values.
    Value,
}

impl Display for DataInstructionErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DataInstructionErrorKind::Executor => write!(f, "executor error"),
            DataInstructionErrorKind::Value => write!(f, "value error"),
        }
    }
}

impl From<ValueError> for DataInstructionError {
    fn from(err: ValueError) -> Self {
        Self::new(DataInstructionErrorKind::Value, err)
    }
}

/// An error that can occur when manipulating the call stack.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CallStackError {
    /// The call stack is empty.
    #[error("stack empty")]
    StackEmpty,
    /// Attempted to jump out of bounds of the current call frame.
    #[error("instruction out of bounds")]
    InstrOutOfBounds,
}

/// A control flow error.
#[derive(Debug, thiserror::Error)]
#[error("control flow error: kind {kind}, {err}")]
pub struct ControlFlowError {
    kind: ControlFlowErrorKind,
    err: Box<dyn Error + Send + Sync>,
}

impl ControlFlowError {
    /// Creates a new control flow error.   
    pub fn new<E>(kind: ControlFlowErrorKind, err: E) -> Self
    where
        E: Into<Box<dyn Error + Send + Sync>>,
    {
        Self {
            kind,
            err: err.into(),
        }
    }

    /// Returns the corresponding [`ControlFlowErrorKind`] for this error.
    pub fn kind(&self) -> ControlFlowErrorKind {
        self.kind
    }

    /// Returns the inner error.
    pub fn into_inner(self) -> Box<dyn Error + Send + Sync> {
        self.err
    }
}

/// A control flow error kind.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum ControlFlowErrorKind {
    /// An error occurred in the executor.
    Executor,
    /// An error occurred while manipulating the call stack.
    CallStack,
}

impl Display for ControlFlowErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ControlFlowErrorKind::Executor => write!(f, "executor error"),
            ControlFlowErrorKind::CallStack => write!(f, "call stack error"),
        }
    }
}

impl From<CallStackError> for ControlFlowError {
    fn from(err: CallStackError) -> Self {
        Self::new(ControlFlowErrorKind::CallStack, err)
    }
}

/// An error that can occur while executing a memory instruction.
#[derive(Debug, thiserror::Error)]
#[error("memory instruction error: kind {kind}, {err}")]
pub struct MemoryInstructionError {
    kind: MemoryInstructionErrorKind,
    err: Box<dyn Error + Send + Sync>,
}

impl MemoryInstructionError {
    /// Creates a new memory instruction error.
    pub fn new<E>(kind: MemoryInstructionErrorKind, err: E) -> Self
    where
        E: Into<Box<dyn Error + Send + Sync>>,
    {
        Self {
            kind,
            err: err.into(),
        }
    }

    /// Returns the corresponding [`MemoryInstructionErrorKind`] for this error.
    pub fn kind(&self) -> MemoryInstructionErrorKind {
        self.kind
    }

    /// Returns the inner error.
    pub fn into_inner(self) -> Box<dyn Error + Send + Sync> {
        self.err
    }
}

/// A memory instruction error kind.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum MemoryInstructionErrorKind {
    /// An error occurred in the executor.
    Executor,
}

impl Display for MemoryInstructionErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemoryInstructionErrorKind::Executor => write!(f, "executor error"),
        }
    }
}

/// An error that can occur while executing instructions.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ExecutorError {
    /// A data instruction error.
    #[error("data error: {source}")]
    Data {
        #[from]
        source: DataInstructionError,
    },
    /// A control flow error.
    #[error("control flow error: {source}")]
    ControlFlow {
        #[from]
        source: ControlFlowError,
    },
    /// A memory instruction error.
    #[error("memory error: {source}")]
    Memory {
        #[from]
        source: MemoryInstructionError,
    },
}
