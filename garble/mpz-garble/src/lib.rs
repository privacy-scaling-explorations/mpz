//! This crate provides an implementation of garbled circuit protocols to facilitate MPC.

#![deny(missing_docs, unreachable_pub, unused_must_use)]
#![deny(clippy::all)]
#![forbid(unsafe_code)]

use std::sync::Arc;

use async_trait::async_trait;

use config::Visibility;
use mpz_circuits::{
    types::{StaticValueType, Value, ValueType},
    Circuit,
};
pub use mpz_core::value::{ValueId, ValueRef};

pub mod config;
pub(crate) mod evaluator;
pub(crate) mod generator;
pub(crate) mod internal_circuits;
pub mod ot;
pub mod protocol;
pub(crate) mod registry;
mod threadpool;

pub use evaluator::{Evaluator, EvaluatorConfig, EvaluatorConfigBuilder, EvaluatorError};
pub use generator::{Generator, GeneratorConfig, GeneratorConfigBuilder, GeneratorError};
pub use registry::ValueRegistry;
pub use threadpool::ThreadPool;

use utils::id::NestedId;

/// Errors that can occur when using an implementation of [`Vm`].
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum VmError {
    #[error(transparent)]
    IOError(#[from] std::io::Error),
    #[error(transparent)]
    MuxerError(#[from] utils_aio::mux::MuxerError),
    #[error(transparent)]
    ProtocolError(#[from] Box<dyn std::error::Error + Send + Sync>),
    #[error(transparent)]
    MemoryError(#[from] MemoryError),
    #[error(transparent)]
    ExecutionError(#[from] ExecutionError),
    #[error(transparent)]
    ProveError(#[from] ProveError),
    #[error(transparent)]
    VerifyError(#[from] VerifyError),
    #[error(transparent)]
    DecodeError(#[from] DecodeError),
    #[error("thread already exists: {0}")]
    ThreadAlreadyExists(String),
    #[error("vm is shutdown")]
    Shutdown,
}

/// Errors that can occur when interacting with memory.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum MemoryError {
    #[error("duplicate value id: {0:?}")]
    DuplicateValueId(ValueId),
    #[error("value is not defined: {0:?}")]
    Undefined(ValueId),
    #[error("invalid reference: {0:?}")]
    InvalidReference(ValueRef),
    #[error("value is already assigned: {0:?}")]
    AlreadyAssigned(ValueId),
    #[error("value is not assigned: {0:?}")]
    Unassigned(ValueId),
    #[error("duplicate value: {0:?}")]
    DuplicateValue(ValueRef),
    #[error(transparent)]
    TypeError(#[from] mpz_circuits::types::TypeError),
    #[error("invalid value type {1:?} for {0:?}")]
    InvalidType(ValueId, mpz_circuits::types::ValueType),
}

/// Errors that can occur when executing a circuit.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum ExecutionError {
    #[error(transparent)]
    IOError(#[from] std::io::Error),
    #[error(transparent)]
    ProtocolError(#[from] Box<dyn std::error::Error + Send + Sync>),
}

/// Errors that can occur when proving the output of a circuit.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum ProveError {
    #[error(transparent)]
    IOError(#[from] std::io::Error),
    #[error(transparent)]
    ProtocolError(#[from] Box<dyn std::error::Error + Send + Sync>),
}

/// Errors that can occur when verifying the output of a circuit.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum VerifyError {
    #[error(transparent)]
    IOError(#[from] std::io::Error),
    #[error(transparent)]
    ProtocolError(#[from] Box<dyn std::error::Error + Send + Sync>),
    #[error("invalid proof")]
    InvalidProof,
}

/// Errors that can occur when decoding values.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum DecodeError {
    #[error(transparent)]
    IOError(#[from] std::io::Error),
    #[error(transparent)]
    ProtocolError(#[from] Box<dyn std::error::Error + Send + Sync>),
}

/// This trait provides an abstraction of MPC, modeling it as a multi-threaded virtual machine.
#[async_trait]
pub trait Vm {
    /// The type of thread.
    type Thread: Thread + Send + 'static;

    /// Creates a new thread.
    async fn new_thread(&mut self, id: &str) -> Result<Self::Thread, VmError>;

    /// Creates a new thread pool.
    async fn new_thread_pool(
        &mut self,
        id: &str,
        thread_count: usize,
    ) -> Result<ThreadPool<Self::Thread>, VmError> {
        let mut id = NestedId::new(id).append_counter();
        let mut threads = Vec::with_capacity(thread_count);
        for _ in 0..thread_count {
            threads.push(
                self.new_thread(&id.increment_in_place().to_string())
                    .await?,
            );
        }
        Ok(ThreadPool::new(threads))
    }
}

/// This trait provides an abstraction of a thread in an MPC virtual machine.
pub trait Thread: Memory {}

/// This trait provides methods for interacting with values in memory.
pub trait Memory {
    /// Defines a new input value, returning a reference to it.
    fn new_input<T: StaticValueType>(
        &self,
        id: &str,
        visibility: Visibility,
    ) -> Result<ValueRef, MemoryError>;

    /// Defines a new public input value, returning a reference to it.
    fn new_public_input<T: StaticValueType>(&self, id: &str) -> Result<ValueRef, MemoryError> {
        self.new_input::<T>(id, Visibility::Public)
    }

    /// Defines a new private input value, returning a reference to it.
    fn new_private_input<T: StaticValueType>(&self, id: &str) -> Result<ValueRef, MemoryError> {
        self.new_input::<T>(id, Visibility::Private)
    }

    /// Defines a new blind input value, returning a reference to it.
    fn new_blind_input<T: StaticValueType>(&self, id: &str) -> Result<ValueRef, MemoryError> {
        self.new_input::<T>(id, Visibility::Blind)
    }

    /// Defines a new array input value, returning a reference to it.
    fn new_input_array<T: StaticValueType>(
        &self,
        id: &str,
        visibility: Visibility,
        len: usize,
    ) -> Result<ValueRef, MemoryError>
    where
        Vec<T>: Into<Value>;

    /// Defines a new public array input value, returning a reference to it.
    fn new_public_input_array<T: StaticValueType>(
        &self,
        id: &str,
        len: usize,
    ) -> Result<ValueRef, MemoryError>
    where
        Vec<T>: Into<Value>,
    {
        self.new_input_array::<T>(id, Visibility::Public, len)
    }

    /// Defines a new private array input value, returning a reference to it.
    fn new_private_input_array<T: StaticValueType>(
        &self,
        id: &str,
        len: usize,
    ) -> Result<ValueRef, MemoryError>
    where
        Vec<T>: Into<Value>,
    {
        self.new_input_array::<T>(id, Visibility::Private, len)
    }

    /// Defines a new blind array input value, returning a reference to it.
    fn new_blind_input_array<T: StaticValueType>(
        &self,
        id: &str,
        len: usize,
    ) -> Result<ValueRef, MemoryError>
    where
        Vec<T>: Into<Value>,
    {
        self.new_input_array::<T>(id, Visibility::Blind, len)
    }

    /// Defines a new output value, returning a reference to it.
    fn new_output<T: StaticValueType>(&self, id: &str) -> Result<ValueRef, MemoryError>;

    /// Defines a new array output value, returning a reference to it.
    fn new_output_array<T: StaticValueType>(
        &self,
        id: &str,
        len: usize,
    ) -> Result<ValueRef, MemoryError>
    where
        Vec<T>: Into<Value>;

    /// Assigns a value
    fn assign<T: StaticValueType>(&self, value_ref: &ValueRef, value: T)
        -> Result<(), MemoryError>;

    /// Returns a value if it exists.
    fn get_value(&self, id: &str) -> Option<ValueRef>;

    /// Returns the type of a value if it exists.
    fn get_value_type(&self, id: &str) -> Option<ValueType>;
}

/// This trait provides methods for executing a circuit.
#[async_trait]
pub trait Execute {
    /// Executes a circuit with the provided inputs, assigning to the provided output values
    async fn execute(
        &mut self,
        circ: Arc<Circuit>,
        inputs: &[ValueRef],
        outputs: &[ValueRef],
    ) -> Result<(), ExecutionError>;
}

/// This trait provides methods for proving the output of a circuit.
#[async_trait]
pub trait Prove {
    /// Proves the output of the circuit with the provided inputs, assigning to the provided output values
    async fn prove(
        &mut self,
        circ: Arc<Circuit>,
        inputs: &[ValueRef],
        outputs: &[ValueRef],
    ) -> Result<(), ProveError>;
}

/// This trait provides methods for verifying the output of a circuit.
#[async_trait]
pub trait Verify {
    /// Verifies the output of the circuit with the provided inputs, assigning to the provided output values
    async fn verify(
        &mut self,
        circ: Arc<Circuit>,
        inputs: &[ValueRef],
        outputs: &[ValueRef],
        expected_outputs: &[Value],
    ) -> Result<(), VerifyError>;
}

/// This trait provides methods for decoding values.
#[async_trait]
pub trait Decode {
    /// Decodes the provided values, returning the plaintext values to all parties.
    async fn decode(&mut self, values: &[ValueRef]) -> Result<Vec<Value>, DecodeError>;
}

/// This trait provides methods for decoding values with different privacy configurations.
#[async_trait]
pub trait DecodePrivate {
    /// Decodes the provided values, returning the plaintext values to only this party.
    async fn decode_private(&mut self, values: &[ValueRef]) -> Result<Vec<Value>, DecodeError>;

    /// Decodes the provided values, returning the plaintext values to the other party(s).
    async fn decode_blind(&mut self, values: &[ValueRef]) -> Result<(), DecodeError>;

    /// Decodes the provided values, returning additive shares of plaintext values to all parties.
    async fn decode_shared(&mut self, values: &[ValueRef]) -> Result<Vec<Value>, DecodeError>;
}
