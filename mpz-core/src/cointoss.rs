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

use serde::{Deserialize, Serialize};

use crate::{
    commit::{CommitmentError, Decommitment, HashCommit},
    hash::Hash,
    Block,
};

/// A coin-toss error.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum CointossError {
    #[error(transparent)]
    CommitmentError(#[from] CommitmentError),
    #[error("count mismatch, expected {expected}, got {actual}")]
    CountMismatch { expected: usize, actual: usize },
}

/// A coin-toss sender.
#[derive(Debug)]
pub struct Sender<S: sender_state::State = sender_state::Initialized> {
    state: S,
}

impl Sender {
    /// Create a new sender.
    pub fn new(seeds: Vec<Block>) -> Self {
        Self {
            state: sender_state::Initialized { seeds },
        }
    }

    /// Sends the coin-toss commitments.
    pub fn send(self) -> (Sender<sender_state::Committed>, SenderCommitments) {
        let sender_state::Initialized { seeds } = self.state;

        let (decommitments, commitments): (Vec<_>, Vec<_>) =
            seeds.iter().copied().map(|seed| seed.hash_commit()).unzip();

        (
            Sender {
                state: sender_state::Committed {
                    seeds,
                    decommitments,
                },
            },
            SenderCommitments { commitments },
        )
    }
}

impl Sender<sender_state::Committed> {
    /// Finalizes the coin-toss, returning the random seeds and the sender's payload.
    pub fn finalize(
        self,
        payload: ReceiverPayload,
    ) -> Result<(Vec<Block>, SenderPayload), CointossError> {
        let receiver_seeds = payload.seeds;
        let sender_seeds = self.state.seeds;

        if sender_seeds.len() != receiver_seeds.len() {
            return Err(CointossError::CountMismatch {
                expected: sender_seeds.len(),
                actual: receiver_seeds.len(),
            });
        }

        let seeds = sender_seeds
            .into_iter()
            .zip(receiver_seeds)
            .map(|(sender_seed, receiver_seed)| sender_seed ^ receiver_seed)
            .collect();

        Ok((
            seeds,
            SenderPayload {
                decommitments: self.state.decommitments,
            },
        ))
    }
}

/// Coin-toss sender state.
pub mod sender_state {
    use crate::commit::Decommitment;

    use super::*;

    mod sealed {
        use super::*;

        pub trait Sealed {}

        impl Sealed for Initialized {}
        impl Sealed for Committed {}
    }

    /// The sender's state.
    pub trait State: sealed::Sealed {}

    /// The sender's initial state.
    pub struct Initialized {
        pub(super) seeds: Vec<Block>,
    }

    impl State for Initialized {}

    opaque_debug::implement!(Initialized);

    /// The sender's committed state.
    pub struct Committed {
        pub(super) seeds: Vec<Block>,
        pub(super) decommitments: Vec<Decommitment<Block>>,
    }

    impl State for Committed {}

    opaque_debug::implement!(Committed);
}

/// A coin-toss receiver.
#[derive(Debug)]
pub struct Receiver<S: receiver_state::State = receiver_state::Initialized> {
    state: S,
}

impl Receiver {
    /// Create a new receiver.
    pub fn new(seeds: Vec<Block>) -> Self {
        Self {
            state: receiver_state::Initialized { seeds },
        }
    }

    /// Reveals the receiver's seeds after receiving the sender's commitments.
    pub fn reveal(
        self,
        sender_commitments: SenderCommitments,
    ) -> Result<(Receiver<receiver_state::Received>, ReceiverPayload), CointossError> {
        let receiver_state::Initialized { seeds } = self.state;

        if sender_commitments.commitments.len() != seeds.len() {
            return Err(CointossError::CountMismatch {
                expected: sender_commitments.commitments.len(),
                actual: seeds.len(),
            });
        }

        Ok((
            Receiver {
                state: receiver_state::Received {
                    seeds: seeds.clone(),
                    commitments: sender_commitments.commitments,
                },
            },
            ReceiverPayload { seeds },
        ))
    }
}

impl Receiver<receiver_state::Received> {
    /// Finalizes the coin-toss, returning the random seeds.
    pub fn finalize(self, payload: SenderPayload) -> Result<Vec<Block>, CointossError> {
        let decommitments = payload.decommitments;
        let receiver_seeds = self.state.seeds;
        let commitments = self.state.commitments;

        if decommitments.len() != receiver_seeds.len() {
            return Err(CointossError::CountMismatch {
                expected: decommitments.len(),
                actual: receiver_seeds.len(),
            });
        }

        if commitments.len() != receiver_seeds.len() {
            return Err(CointossError::CountMismatch {
                expected: commitments.len(),
                actual: receiver_seeds.len(),
            });
        }

        decommitments
            .into_iter()
            .zip(receiver_seeds)
            .zip(commitments)
            .map(|((decommitment, receiver_seed), commitment)| {
                decommitment.verify(&commitment)?;

                Ok(receiver_seed ^ decommitment.into_inner())
            })
            .collect::<Result<Vec<_>, CointossError>>()
    }
}

/// Coin-toss receiver state.
pub mod receiver_state {
    use super::*;

    mod sealed {
        use super::*;

        pub trait Sealed {}

        impl Sealed for Initialized {}
        impl Sealed for Received {}
    }

    /// The receiver's state.
    pub trait State: sealed::Sealed {}

    /// The receiver's initial state.
    pub struct Initialized {
        pub(super) seeds: Vec<Block>,
    }

    impl State for Initialized {}

    opaque_debug::implement!(Initialized);

    /// The receiver's state after receiving the sender's commitments.
    pub struct Received {
        pub(super) seeds: Vec<Block>,
        pub(super) commitments: Vec<Hash>,
    }

    impl State for Received {}

    opaque_debug::implement!(Received);
}

/// Coin-toss protocol messages.
pub mod msgs {
    use super::*;

    /// A coin-toss message.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[allow(missing_docs)]
    pub enum Message {
        SenderCommitments(SenderCommitments),
        SenderPayload(SenderPayload),
        ReceiverPayload(ReceiverPayload),
    }

    /// The coin-toss sender's commitments.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct SenderCommitments {
        /// The commitments to the random seeds.
        pub commitments: Vec<Hash>,
    }

    /// The coin-toss sender's payload.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct SenderPayload {
        /// The decommitments to the random seeds.
        pub decommitments: Vec<Decommitment<Block>>,
    }

    /// The coin-toss receiver's payload.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ReceiverPayload {
        /// The receiver's random seeds.
        pub seeds: Vec<Block>,
    }
}

use msgs::*;
