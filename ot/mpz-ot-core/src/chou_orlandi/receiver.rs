use crate::chou_orlandi::{
    hash_point,
    msgs::{ReceiverPayload, ReceiverReveal, ReceiverSetup, SenderPayload, SenderSetup},
    ReceiverConfig, ReceiverError,
};

use itybity::{BitIterable, FromBitIterator, ToBits};
use mpz_core::{
    commit::{Decommitment, HashCommit},
    Block,
};

use curve25519_dalek::{
    constants::RISTRETTO_BASEPOINT_TABLE,
    ristretto::{RistrettoBasepointTable, RistrettoPoint},
    scalar::Scalar,
};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

#[cfg(feature = "rayon")]
use rayon::prelude::{IndexedParallelIterator, IntoParallelIterator, ParallelIterator};

/// A [CO15](https://eprint.iacr.org/2015/267.pdf) receiver.
#[derive(Debug, Default)]
pub struct Receiver<T = state::Initialized> {
    /// The receiver's configuration
    config: ReceiverConfig,
    /// The current state of the protocol
    state: T,
}

impl Receiver {
    /// Creates a new receiver.
    ///
    /// # Arguments
    ///
    /// * `config` - The receiver's configuration
    pub fn new(config: ReceiverConfig) -> Self {
        Self {
            config,
            state: state::Initialized::default(),
        }
    }

    /// Creates a new receiver with the provided RNG seed.
    ///
    /// # Arguments
    ///
    /// * `config` - The receiver's configuration
    pub fn new_with_seed(config: ReceiverConfig, seed: [u8; 32]) -> Self {
        Self {
            config,
            state: state::Initialized {
                rng: ChaCha20Rng::from_seed(seed),
            },
        }
    }

    /// Sets up the receiver.
    ///
    /// # Arguments
    ///
    /// * `sender_setup` - The sender's setup message
    pub fn setup(self, sender_setup: SenderSetup) -> (ReceiverSetup, Receiver<state::Setup>) {
        let state::Initialized { rng } = self.state;

        // Commit to RNG seed if configured.
        let (decommitment, commitment) = if self.config.receiver_commit() {
            let (decommitment, commitment) = rng.get_seed().hash_commit();
            (Some(decommitment), Some(commitment))
        } else {
            (None, None)
        };

        (
            ReceiverSetup { commitment },
            Receiver {
                config: self.config,
                state: state::Setup {
                    rng,
                    sender_base_table: RistrettoBasepointTable::create(&sender_setup.public_key),
                    counter: 0,
                    choice_log: Vec::default(),
                    decryption_keys: Vec::default(),
                    decommitment,
                },
            },
        )
    }
}

impl Receiver<state::Setup> {
    /// Computes the decryption keys, returning the Receiver's payload to be sent to the Sender.
    ///
    /// # Arguments
    ///
    /// * `choices` - The receiver's choices
    pub fn receive_random<T: BitIterable + Sync>(&mut self, choices: &[T]) -> ReceiverPayload {
        let state::Setup {
            rng,
            sender_base_table,
            counter,
            choice_log,
            decryption_keys: cached_decryption_keys,
            ..
        } = &mut self.state;

        let private_keys = choices
            .iter_lsb0()
            .map(|_| Scalar::random(rng))
            .collect::<Vec<_>>();

        let (blinded_choices, decryption_keys) =
            compute_decryption_keys(sender_base_table, &private_keys, choices, *counter);

        *counter += blinded_choices.len();
        cached_decryption_keys.extend(decryption_keys);

        // If configured, log the choices
        if self.config.receiver_commit() {
            choice_log.extend(Vec::<u8>::from_lsb0_iter(choices.iter_lsb0()));
        }

        ReceiverPayload { blinded_choices }
    }

    /// Receives the encrypted payload from the Sender, returning the plaintext messages corresponding
    /// to the Receiver's choices.
    ///
    /// # Arguments
    ///
    /// * `payload` - The encrypted payload from the Sender
    pub fn receive(&mut self, payload: SenderPayload) -> Result<Vec<Block>, ReceiverError> {
        let state::Setup {
            decryption_keys, ..
        } = &mut self.state;

        let SenderPayload { payload } = payload;

        // Check that the number of ciphertexts does not exceed the number of pending keys
        if payload.len() > decryption_keys.len() {
            return Err(ReceiverError::CountMismatch(
                decryption_keys.len(),
                payload.len(),
            ));
        }

        // Drain the decryption keys and decrypt the ciphertexts
        Ok(decryption_keys
            .drain(..payload.len())
            .zip(payload)
            .map(
                |((c, key), [ct0, ct1])| {
                    if c {
                        key ^ ct1
                    } else {
                        key ^ ct0
                    }
                },
            )
            .collect::<Vec<Block>>())
    }

    /// Reveals the receiver's choices to the Sender by decommiting to the RNG seed,
    /// and sending the choice log.
    pub fn reveal_choices(self) -> Result<ReceiverReveal, ReceiverError> {
        let state::Setup {
            decommitment: seed_decommit,
            choice_log: choices,
            ..
        } = self.state;

        let Some(seed_decommit) = seed_decommit else {
            return Err(ReceiverError::NoReceiverCommit)
        };

        Ok(ReceiverReveal {
            seed_decommit,
            choices,
        })
    }
}

/// Computes the blinded choices `B` and the decryption keys for the OT receiver.
///
/// # Arguments
///
/// * `rng` - A crypto-secure RNG
/// * `base_table` - A Ristretto basepoint table from the sender's public key
/// * `receiver_private_keys` - The private keys of the OT receiver
/// * `choices` - The choices of the OT receiver
/// * `offset` - The number of OTs that have already been performed
///              (used for the key derivation tweak)
fn compute_decryption_keys<T: BitIterable + Sync>(
    base_table: &RistrettoBasepointTable,
    receiver_private_keys: &[Scalar],
    choices: &[T],
    offset: usize,
) -> (Vec<RistrettoPoint>, Vec<(bool, Block)>) {
    let zero = &Scalar::ZERO * base_table;
    let one = &Scalar::ONE * base_table;

    cfg_if::cfg_if! {
        if #[cfg(feature = "rayon")] {
            // itybity currently doesn't support `IndexedParallelIterator` for collections,
            // so we allocate instead.
            let temp = receiver_private_keys.iter().zip(choices.iter_lsb0()).collect::<Vec<_>>();
            let iter = temp.into_par_iter().enumerate();
        } else {
            let iter = receiver_private_keys.iter().zip(choices.iter_lsb0()).enumerate();
        }
    }

    iter.map(|(i, (b, c))| {
        // blinded_choice is B in [ref1]
        //
        // if c = 0: B = g ^ b
        // if c = 1: B = A * g ^ b
        //
        // when choice is 0, we add the zero element to keep constant time.
        let blinded_choice = if c {
            one + b * RISTRETTO_BASEPOINT_TABLE
        } else {
            zero + b * RISTRETTO_BASEPOINT_TABLE
        };

        let decryption_key = hash_point(&(b * base_table), (offset + i) as u128);

        (blinded_choice, (c, decryption_key))
    })
    .unzip()
}

/// The receiver's state.
pub mod state {
    use super::*;

    mod sealed {
        pub trait Sealed {}

        impl Sealed for super::Initialized {}
        impl Sealed for super::Setup {}
    }

    /// The receiver's state.
    pub trait State: sealed::Sealed {}

    /// The receiver's initial state.
    pub struct Initialized {
        pub(super) rng: ChaCha20Rng,
    }

    impl State for Initialized {}

    opaque_debug::implement!(Initialized);

    impl Default for Initialized {
        fn default() -> Self {
            Self {
                rng: ChaCha20Rng::from_entropy(),
            }
        }
    }

    /// The receiver's state after setup.
    pub struct Setup {
        pub(super) rng: ChaCha20Rng,
        pub(super) sender_base_table: RistrettoBasepointTable,
        pub(super) counter: usize,
        pub(super) choice_log: Vec<u8>,

        /// The decryption key for each OT, with the corresponding choice bit
        pub(super) decryption_keys: Vec<(bool, Block)>,

        pub(super) decommitment: Option<Decommitment<[u8; 32]>>,
    }

    impl State for Setup {}

    opaque_debug::implement!(Setup);
}
