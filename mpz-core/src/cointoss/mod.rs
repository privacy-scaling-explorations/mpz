//! A simple 2-party coin-toss protocol.
//!
//! # Example
//!
//! ```
//! use rand::{thread_rng, Rng};
//! use mpz_core::cointoss::{Sender, Receiver};
//! # use mpz_core::cointoss::CointossError;
//! use mpz_core::Block;
//!
//! # fn main() -> Result<(), CointossError> {
//! let sender_seeds = (0..8).map(|_| Block::random(&mut thread_rng())).collect();
//! let receiver_seeds = (0..8).map(|_| Block::random(&mut thread_rng())).collect();
//!
//! let sender = Sender::new(sender_seeds);
//! let receiver = Receiver::new(receiver_seeds);
//!
//! let (sender, commitments) = sender.send();
//! let (receiver, receiver_payload) = receiver.reveal(commitments)?;
//! let (sender_output, sender_payload) = sender.finalize(receiver_payload)?;
//! let receiver_output = receiver.finalize(sender_payload)?;
//!
//! assert_eq!(sender_output, receiver_output);
//! # Ok(())
//! # }
//! ```

pub mod msgs;
mod receiver;
mod sender;

pub use receiver::{receiver_state, Receiver};
pub use sender::{sender_state, Sender};

/// A coin-toss error.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum CointossError {
    #[error(transparent)]
    CommitmentError(#[from] crate::commit::CommitmentError),
    #[error("count mismatch, expected {expected}, got {actual}")]
    CountMismatch { expected: usize, actual: usize },
}
