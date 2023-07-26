//! Core types and utilities for MPC protocols
#![deny(missing_docs, unreachable_pub, unused_must_use)]
#![deny(clippy::all)]

pub mod aes;
mod block;
pub mod cointoss;
pub mod commit;
pub mod hash;
pub mod serialize;
pub mod utils;
pub mod value;

pub use block::{Block, BlockSerialize};

/// A protocol with a message type.
pub trait ProtocolMessage {
    /// The type of message used in the protocol.
    type Msg: Send + Sync + std::fmt::Debug + 'static;
}
