use crate::{
    cointoss::{
        msgs::{ReceiverPayload, SenderCommitments, SenderPayload},
        CointossError,
    },
    commit::HashCommit,
    Block,
};

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
