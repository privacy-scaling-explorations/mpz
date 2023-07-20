use crate::{
    cointoss::{
        msgs::{ReceiverPayload, SenderCommitments, SenderPayload},
        CointossError,
    },
    hash::Hash,
    Block,
};

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
