use crate::chou_orlandi::{
    hash_point,
    msgs::{ReceiverPayload, ReceiverReveal, SenderPayload, SenderSetup},
    ReceiverConfig, ReceiverError,
};

use itybity::{BitIterable, FromBitIterator, ToBits};
use mpz_core::Block;

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
    /// # Committed Receiver
    ///
    /// ## ⚠️ Warning ⚠️
    ///
    /// If the receiver is committed, the receiver's RNG seed must be unbiased such as generated by
    /// a secure coin toss protocol with the sender.
    ///
    /// Use the [`new_with_seed`] method to provide a seed.
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
    /// # Committed Receiver
    ///
    /// ## ⚠️ Warning ⚠️
    ///
    /// If the receiver is committed, the receiver's RNG seed must be unbiased such as generated by
    /// a secure coin toss protocol with the sender.
    ///
    /// # Arguments
    ///
    /// * `config` - The receiver's configuration
    /// * `seed` - The RNG seed used to generate the receiver's keys
    pub fn new_with_seed(config: ReceiverConfig, seed: [u8; 32]) -> Self {
        Self {
            config,
            state: state::Initialized {
                rng: ChaCha20Rng::from_seed(seed),
            },
        }
    }

    /// Returns the receiver's configuration.
    pub fn config(&self) -> &ReceiverConfig {
        &self.config
    }

    /// Sets up the receiver.
    ///
    /// # Arguments
    ///
    /// * `sender_setup` - The sender's setup message
    pub fn setup(self, sender_setup: SenderSetup) -> Receiver<state::Setup> {
        let state::Initialized { rng } = self.state;

        Receiver {
            config: self.config,
            state: state::Setup {
                rng,
                sender_base_table: RistrettoBasepointTable::create(&sender_setup.public_key),
                counter: 0,
                choice_log: Vec::default(),
                decryption_keys: Vec::default(),
            },
        }
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
            choice_log.extend(choices.iter_lsb0());
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

    /// Reveals the receiver's choices to the Sender
    pub fn reveal_choices(self) -> Result<ReceiverReveal, ReceiverError> {
        let state::Setup { choice_log, .. } = self.state;

        Ok(ReceiverReveal {
            choices: Vec::<u8>::from_lsb0_iter(choice_log),
        })
    }
}

/// Computes the blinded choices `B` and the decryption keys for the OT receiver.
///
/// # Arguments
///
/// * `base_table` - A Ristretto basepoint table from the sender's public key
/// * `receiver_private_keys` - The private keys of the OT receiver
/// * `choices` - The choices of the OT receiver
/// * `offset` - The number of decryption keys that have already been computed
///              (used for the key derivation tweak)
fn compute_decryption_keys<T: BitIterable + Sync>(
    base_table: &RistrettoBasepointTable,
    receiver_private_keys: &[Scalar],
    choices: &[T],
    offset: usize,
) -> (Vec<RistrettoPoint>, Vec<(bool, Block)>) {
    let zero = &Scalar::ZERO * base_table;
    // a is A in [ref1]
    let a = &Scalar::ONE * base_table;

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
            a + b * RISTRETTO_BASEPOINT_TABLE
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
        /// RNG used to generate the receiver's keys
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
        /// RNG used to generate the receiver's keys
        pub(super) rng: ChaCha20Rng,
        /// Sender's public key (precomputed table)
        pub(super) sender_base_table: RistrettoBasepointTable,
        /// Counts how many decryption keys we've computed so far
        pub(super) counter: usize,
        /// Log of the receiver's choice bits
        pub(super) choice_log: Vec<bool>,

        /// The decryption key for each OT, with the corresponding choice bit
        pub(super) decryption_keys: Vec<(bool, Block)>,
    }

    impl State for Setup {}

    opaque_debug::implement!(Setup);
}
