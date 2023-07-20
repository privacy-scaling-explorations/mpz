use crate::chou_orlandi::{
    hash_point,
    msgs::{ReceiverPayload, ReceiverReveal, ReceiverSetup, SenderPayload, SenderSetup},
    Receiver, ReceiverConfig, SenderConfig, SenderError, SenderVerifyError,
};

use itybity::IntoBitIterator;
use mpz_core::{hash::Hash, Block};

use curve25519_dalek::{
    constants::RISTRETTO_BASEPOINT_TABLE, ristretto::RistrettoPoint, scalar::Scalar,
};
use rand::SeedableRng;
use rand_chacha::ChaCha20Rng;

#[cfg(feature = "rayon")]
use rayon::prelude::{IndexedParallelIterator, IntoParallelRefIterator, ParallelIterator};

/// A tape used to record all the blinded choices made by the receiver, which
/// can later be used to perform a consistency check.
#[derive(Debug, Default)]
struct Tape {
    receiver_choices: Vec<RistrettoPoint>,
}

/// A [CO15](https://eprint.iacr.org/2015/267.pdf) sender.
#[derive(Debug, Default)]
pub struct Sender<T: state::State = state::Initialized> {
    config: SenderConfig,
    /// Current state
    state: T,
    /// Protocol tape
    tape: Option<Tape>,
}

impl Sender {
    /// Creates a new Sender
    ///
    /// # Arguments
    ///
    /// * `config` - The Sender's configuration
    pub fn new(config: SenderConfig) -> Self {
        let tape = if config.receiver_commit() {
            Some(Tape::default())
        } else {
            None
        };

        Sender {
            config,
            state: state::Initialized::default(),
            tape,
        }
    }

    /// Creates a new Sender with the provided RNG seed
    ///
    /// # Arguments
    ///
    /// * `config` - The Sender's configuration
    /// * `seed` - The RNG seed
    pub fn new_with_seed(config: SenderConfig, seed: [u8; 32]) -> Self {
        let mut rng = ChaCha20Rng::from_seed(seed);

        let private_key = Scalar::random(&mut rng);
        let public_key = &private_key * RISTRETTO_BASEPOINT_TABLE;
        let state = state::Initialized {
            private_key,
            public_key,
        };

        let tape = if config.receiver_commit() {
            Some(Tape::default())
        } else {
            None
        };

        Sender {
            config,
            state,
            tape,
        }
    }

    /// Returns the setup message to be sent to the receiver.
    pub fn setup(self) -> (SenderSetup, Sender<state::ReceiveSetup>) {
        let state::Initialized {
            private_key,
            public_key,
        } = self.state;

        (
            SenderSetup { public_key },
            Sender {
                config: self.config,
                state: state::ReceiveSetup {
                    private_key,
                    public_key,
                },
                tape: self.tape,
            },
        )
    }
}

impl Sender<state::ReceiveSetup> {
    /// Receives the receiver's setup message.
    ///
    /// # Arguments
    ///
    /// * `receiver_setup` - The receiver's setup message.
    pub fn receive_setup(
        self,
        receiver_setup: ReceiverSetup,
    ) -> Result<Sender<state::Setup>, SenderError> {
        let state::ReceiveSetup {
            private_key,
            public_key,
        } = self.state;

        let ReceiverSetup { commitment } = receiver_setup;

        if self.config.receiver_commit() && commitment.is_none() {
            return Err(SenderError::NoCommitment);
        }

        Ok(Sender {
            config: self.config,
            state: state::Setup {
                private_key,
                public_key,
                counter: 0,
                receiver_commitment: commitment,
            },
            tape: self.tape,
        })
    }
}

impl Sender<state::Setup> {
    /// Obliviously sends `inputs` to the receiver.
    ///
    /// # Arguments
    ///
    /// * `inputs` - The inputs to be obliviously sent to the receiver.
    /// * `receiver_payload` - The receiver's choice payload.
    pub fn send(
        &mut self,
        inputs: &[[Block; 2]],
        receiver_payload: ReceiverPayload,
    ) -> Result<SenderPayload, SenderError> {
        let state::Setup {
            private_key,
            public_key,
            counter,
            ..
        } = &mut self.state;

        let ReceiverPayload { blinded_choices } = receiver_payload;

        // Check that the number of inputs matches the number of choices
        if inputs.len() != blinded_choices.len() {
            return Err(SenderError::CountMismatch(
                blinded_choices.len(),
                inputs.len(),
            ));
        }

        if let Some(tape) = self.tape.as_mut() {
            // Record the receiver's choices
            tape.receiver_choices.extend_from_slice(&blinded_choices);
        }

        let mut payload =
            compute_encryption_keys(private_key, public_key, &blinded_choices, *counter);

        *counter += inputs.len();

        // Encrypt the inputs
        for (input, payload) in inputs.iter().zip(payload.iter_mut()) {
            payload[0] = input[0] ^ payload[0];
            payload[1] = input[1] ^ payload[1];
        }

        Ok(SenderPayload { payload })
    }

    /// Returns the Receiver choices after verifying them against the tape.
    ///
    /// # Arguments
    ///
    /// * `receiver_reveal` - The receiver's private inputs.
    pub fn verify_choices(self, receiver_reveal: ReceiverReveal) -> Result<Vec<bool>, SenderError> {
        let state::Setup {
            public_key,
            receiver_commitment,
            ..
        } = self.state;

        let receiver_commitment = receiver_commitment.ok_or(SenderVerifyError::NoCommitment)?;

        let Some(tape) = &self.tape else {
            return Err(SenderVerifyError::TapeNotRecorded)?;
        };

        let ReceiverReveal {
            seed_decommit,
            choices,
        } = receiver_reveal;

        // Check that the receiver's decommitment is consistent
        seed_decommit
            .verify(&receiver_commitment)
            .map_err(SenderVerifyError::from)?;

        let receiver_seed = seed_decommit.into_inner();

        let choices = choices
            .into_iter_lsb0()
            .take(tape.receiver_choices.len())
            .collect::<Vec<bool>>();

        // Check that the number of choices matches
        if tape.receiver_choices.len() != choices.len() {
            return Err(SenderVerifyError::ChoiceCountMismatch(
                tape.receiver_choices.len(),
                choices.len(),
            ))?;
        }

        // Simulate the receiver
        let receiver = Receiver::new_with_seed(ReceiverConfig::default(), receiver_seed);

        let (_, mut receiver) = receiver.setup(SenderSetup { public_key });

        let ReceiverPayload { blinded_choices } = receiver.receive_random(&choices);

        // Check that the simulated receiver's choices match the ones recorded in the tape
        if blinded_choices != tape.receiver_choices {
            return Err(SenderVerifyError::InconsistentChoice)?;
        }

        Ok(choices)
    }
}

/// Computes the encryption keys for the sender.
///
/// # Arguments
///
/// * `private_key` - The sender's private key.
/// * `public_key` - The sender's public key.
/// * `receiver_payload` - The receiver's choice payload.
/// * `offset` - The number of OTs that have already been performed
///              (used for the key derivation tweak)
fn compute_encryption_keys(
    private_key: &Scalar,
    public_key: &RistrettoPoint,
    blinded_choices: &[RistrettoPoint],
    offset: usize,
) -> Vec<[Block; 2]> {
    // ys is A^a in [ref1]
    let ys = private_key * public_key;

    cfg_if::cfg_if! {
        if #[cfg(feature = "rayon")] {
            let iter = blinded_choices
                .par_iter()
                .enumerate();
        } else {
            let iter = blinded_choices
                .iter()
                .enumerate();
        }
    }

    iter.map(|(i, blinded_choice)| {
        // yr is B^a in [ref1]
        let yr = private_key * blinded_choice;
        let k0 = hash_point(&yr, (offset + i) as u128);
        // yr - ys == (B/A)^a in [ref1]
        let k1 = hash_point(&(yr - ys), (offset + i) as u128);

        [k0, k1]
    })
    .collect()
}

/// The sender's state.
pub mod state {
    use super::*;

    mod sealed {
        pub trait Sealed {}

        impl Sealed for super::Initialized {}
        impl Sealed for super::ReceiveSetup {}
        impl Sealed for super::Setup {}
    }

    /// The sender's state.
    pub trait State: sealed::Sealed {}

    /// The sender's initial state.
    pub struct Initialized {
        /// The private_key is random `a` in [ref1]
        pub(super) private_key: Scalar,
        // The public_key is `A == g^a` in [ref1]
        pub(super) public_key: RistrettoPoint,
    }

    impl State for Initialized {}

    opaque_debug::implement!(Initialized);

    impl Default for Initialized {
        fn default() -> Self {
            let mut rng = ChaCha20Rng::from_entropy();
            let private_key = Scalar::random(&mut rng);
            let public_key = &private_key * RISTRETTO_BASEPOINT_TABLE;
            Initialized {
                private_key,
                public_key,
            }
        }
    }

    /// The sender's state while waiting for the receiver's setup.
    pub struct ReceiveSetup {
        /// The private_key is random `a` in [ref1]
        pub(super) private_key: Scalar,
        // The public_key is `A == g^a` in [ref1]
        pub(super) public_key: RistrettoPoint,
    }

    impl State for ReceiveSetup {}

    opaque_debug::implement!(ReceiveSetup);

    /// The sender's state when setup is complete.
    pub struct Setup {
        /// The private_key is random `a` in [ref1]
        pub(super) private_key: Scalar,
        // The public_key is `A == g^a` in [ref1]
        pub(super) public_key: RistrettoPoint,
        /// Number of OTs sent so far
        pub(super) counter: usize,
        /// The receiver's commitment to their RNG seed
        pub(super) receiver_commitment: Option<Hash>,
    }

    impl State for Setup {}

    opaque_debug::implement!(Setup);
}
