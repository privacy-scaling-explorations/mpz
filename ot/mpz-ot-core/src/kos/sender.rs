use crate::{
    kos::{
        msgs::{Check, Extend, SenderPayload},
        Rng, RngSeed, SenderConfig, SenderError, CSP, SSP,
    },
    msgs::Derandomize,
};

use itybity::IntoBitIterator;
use mpz_core::{aes::FIXED_KEY_AES, Block};

use rand::{Rng as _, SeedableRng};
use rand_chacha::ChaCha20Rng;
use rand_core::RngCore;

cfg_if::cfg_if! {
    if #[cfg(feature = "rayon")] {
        use itybity::ToParallelBits;
        use rayon::prelude::*;
    } else {
        use itybity::ToBits;
    }
}

/// KOS15 sender.
#[derive(Debug, Default)]
pub struct Sender<T: state::State = state::Initialized> {
    config: SenderConfig,
    state: T,
}

/// Returned by the `Sender::keys` method, used in cases where the sender
/// wishes to reserve a set of keys for use later, while still being able to process
/// other payloads.
///
/// A set of keys which can be used to encrypt a payload.
pub struct Keys {
    keys: Vec<[Block; 2]>,
}

impl Keys {
    /// Encrypts the provided messages using the keys, performing Beaver derandomization.
    ///
    /// # Arguments
    ///
    /// * `msgs` - The messages to encrypt
    /// * `derandomize` - The receiver's derandomized choices.
    pub fn send(
        self,
        msgs: &[[Block; 2]],
        derandomize: Derandomize,
    ) -> Result<SenderPayload, SenderError> {
        let Derandomize { flip } = derandomize;

        let flip = flip.into_lsb0_vec();

        if msgs.len() > flip.len() {
            return Err(SenderError::CountMismatch(flip.len(), msgs.len()));
        }

        if msgs.len() > self.keys.len() {
            return Err(SenderError::InsufficientSetup(msgs.len(), self.keys.len()));
        }

        // Encrypt the chosen messages using the generated keys from ROT.
        let ciphertexts = self
            .keys
            .into_iter()
            .zip(msgs)
            .zip(flip)
            .flat_map(|(([k0, k1], [m0, m1]), flip)| {
                // Use Beaver derandomization to correct the receiver's choices
                // from the extension phase.
                if flip {
                    [k1 ^ *m0, k0 ^ *m1]
                } else {
                    [k0 ^ *m0, k1 ^ *m1]
                }
            })
            .collect();

        Ok(SenderPayload { ciphertexts })
    }
}

impl<T> Sender<T>
where
    T: state::State,
{
    /// Returns the Sender's configuration
    pub fn config(&self) -> &SenderConfig {
        &self.config
    }
}

impl Sender {
    /// Creates a new Sender
    ///
    /// # Arguments
    ///
    /// * `config` - The Sender's configuration
    pub fn new(config: SenderConfig) -> Self {
        Sender {
            config,
            state: state::Initialized::default(),
        }
    }

    /// Complete the base setup phase of the protocol.
    ///
    /// # Arguments
    ///
    /// * `delta` - The sender's base OT choices
    /// * `seeds` - The receiver's rng seeds received via base OT
    pub fn base_setup(self, delta: Block, seeds: [Block; CSP]) -> Sender<state::Extension> {
        let rngs = seeds
            .iter()
            .map(|seed| {
                let mut seed_ = RngSeed::default();
                seed_
                    .iter_mut()
                    .zip(seed.to_bytes().into_iter().cycle())
                    .for_each(|(s, c)| *s = c);
                Rng::from_seed(seed_)
            })
            .collect();

        Sender {
            config: self.config,
            state: state::Extension {
                delta,
                rngs,
                keys: Vec::default(),
                counter: 0,
                unchecked_qs: Vec::default(),
                unchecked_keys: Vec::default(),
            },
        }
    }
}

impl Sender<state::Extension> {
    /// The number of remaining OTs which can be consumed.
    pub fn remaining(&self) -> usize {
        self.state.keys.len()
    }

    /// Perform the IKNP OT extension.
    ///
    /// # Sacrificial OTs
    ///
    /// Performing the consistency check sacrifices 256 OTs, so be sure to extend enough to
    /// compensate for this.
    ///
    /// # Streaming
    ///
    /// Extension can be performed in a streaming fashion by processing an extension in batches via
    /// multiple calls to this method.
    ///
    /// The freshly extended OTs are not available until after the consistency check has been
    /// performed. See [`Sender::check`].
    ///
    /// # Arguments
    ///
    /// * `count` - The number of additional OTs to extend
    /// * `extend` - The receiver's setup message
    pub fn extend(&mut self, count: usize, extend: Extend) -> Result<(), SenderError> {
        // Round up the OTs to extend to the nearest multiple of 64 (matrix transpose optimization).
        let count = (count + 63) & !63;

        const NROWS: usize = CSP;
        let row_width = count / 8;

        let Extend {
            us,
            count: receiver_count,
        } = extend;

        // Make sure the number of OTs to extend matches the receiver's setup message.
        if receiver_count != count {
            return Err(SenderError::CountMismatch(receiver_count, count));
        }

        if us.len() != NROWS * row_width {
            return Err(SenderError::InvalidExtend);
        }

        let mut qs = vec![0u8; NROWS * row_width];
        cfg_if::cfg_if! {
            if #[cfg(feature = "rayon")] {
                let iter = self.state.delta
                    .par_iter_lsb0()
                    .zip(self.state.rngs.par_iter_mut())
                    .zip(qs.par_chunks_exact_mut(row_width))
                    .zip(us.par_chunks_exact(row_width));
            } else {
                let iter = self.state.delta
                    .iter_lsb0()
                    .zip(self.state.rngs.iter_mut())
                    .zip(qs.chunks_exact_mut(row_width))
                    .zip(us.chunks_exact(row_width));
            }
        }

        let zero = vec![0u8; row_width];
        iter.for_each(|(((b, rng), q), u)| {
            rng.fill_bytes(q);
            // If `b` is true, xor `u` into `q`, otherwise xor 0 into `q` (constant time).
            let u = if b { u } else { &zero };
            q.iter_mut().zip(u).for_each(|(q, u)| *q ^= u);
        });

        matrix_transpose::transpose_bits(&mut qs, NROWS).expect("matrix is rectangular");

        cfg_if::cfg_if! {
            if #[cfg(feature = "rayon")] {
                let iter = qs.par_chunks_exact(NROWS / 8).enumerate();
            } else {
                let iter = qs.chunks_exact(NROWS / 8).enumerate();
            }
        }

        let cipher = &(*FIXED_KEY_AES);
        let (qs, keys): (Vec<_>, Vec<_>) = iter
            .map(|(j, q)| {
                let q: Block = q.try_into().unwrap();
                let j = Block::new(((self.state.counter + j) as u128).to_be_bytes());

                let k0 = cipher.tccr(j, q);
                let k1 = cipher.tccr(j, q ^ self.state.delta);

                (q, [k0, k1])
            })
            .unzip();

        self.state.counter += count;

        self.state.unchecked_qs.extend(qs);
        self.state.unchecked_keys.extend(keys);

        Ok(())
    }

    /// Checks the consistency of the receiver's choice vectors for all outstanding OTs.
    ///
    /// See section 3.1 of the paper for more details.
    ///
    /// # Sacrificial OTs
    ///
    /// Performing this check sacrifices 256 OTs for the consistency check, so be sure to
    /// extend enough OTs to compensate for this.
    ///
    /// # ⚠️ Warning ⚠️
    ///
    /// The provided seed must be unbiased! It should be generated using a secure
    /// coin-toss protocol **after** the receiver has sent their extension message, ie
    /// after they have already committed to their choice vectors.
    ///
    /// # Arguments
    ///
    /// * `chi_seed` - The seed used to generate the consistency check weights.
    /// * `receiver_check` - The receiver's consistency check message.
    pub fn check(&mut self, chi_seed: Block, receiver_check: Check) -> Result<(), SenderError> {
        let mut seed = RngSeed::default();
        seed.iter_mut()
            .zip(chi_seed.to_bytes().into_iter().cycle())
            .for_each(|(s, c)| *s = c);

        let mut rng = Rng::from_seed(seed);

        let unchecked_qs = std::mem::take(&mut self.state.unchecked_qs);
        let mut unchecked_keys = std::mem::take(&mut self.state.unchecked_keys);

        // Figure 7, "Check correlation", point 1.
        // Sample random weights for the consistency check.
        let chis = (0..unchecked_qs.len())
            .map(|_| rng.gen())
            .collect::<Vec<_>>();

        // Figure 7, "Check correlation", point 3.
        // Compute the random linear combinations.
        cfg_if::cfg_if! {
            if #[cfg(feature = "rayon")] {
                let check = unchecked_qs.into_par_iter()
                    .zip(chis)
                    .map(|(q, chi)| q.clmul(chi))
                    .reduce(
                        || (Block::ZERO, Block::ZERO),
                        |(_a, _b), (a, b)| (a ^ _a, b ^ _b),
                    );
            } else {
                let check = unchecked_qs.into_iter()
                    .zip(chis)
                    .map(|(q, chi)| q.clmul(chi))
                    .reduce(
                        |(_a, _b), (a, b)| (a ^ _a, b ^ _b),
                    ).unwrap();
            }
        }

        let Check { x, t0, t1 } = receiver_check;
        let tmp = x.clmul(self.state.delta);
        let check = (check.0 ^ tmp.0, check.1 ^ tmp.1);

        // Call the police!
        if check != (t0, t1) {
            return Err(SenderError::ConsistencyCheckFailed);
        }

        // Strip off the rows sacrificed for the consistency check.
        let nrows = unchecked_keys.len() - (CSP + SSP);
        unchecked_keys.truncate(nrows);

        // Add to existing qs.
        self.state.keys.extend(unchecked_keys);

        Ok(())
    }

    /// Reserves a set of keys which can be used to encrypt a payload later.
    ///
    /// # Arguments
    ///
    /// * `count` - The number of keys to reserve.
    pub fn keys(&mut self, count: usize) -> Result<Keys, SenderError> {
        if count > self.state.keys.len() {
            return Err(SenderError::InsufficientSetup(count, self.state.keys.len()));
        }

        Ok(Keys {
            keys: self.state.keys.drain(..count).collect(),
        })
    }

    /// Obliviously transfers the provided messages to the receiver, applying Beaver
    /// derandomization to correct the receiver's choices made during extension.
    ///
    /// # Arguments
    ///
    /// * `msgs` - The messages to obliviously transfer
    /// * `derandomize` - The receiver's derandomization choices
    pub fn send(
        &mut self,
        msgs: &[[Block; 2]],
        derandomize: Derandomize,
    ) -> Result<SenderPayload, SenderError> {
        self.keys(msgs.len())?.send(msgs, derandomize)
    }
}

/// The sender's state.
pub mod state {
    use super::*;

    mod sealed {
        pub trait Sealed {}

        impl Sealed for super::Initialized {}
        impl Sealed for super::Extension {}
    }

    /// The sender's state.
    pub trait State: sealed::Sealed {}

    /// The sender's initial state.
    #[derive(Default)]
    pub struct Initialized {}

    impl State for Initialized {}

    opaque_debug::implement!(Initialized);

    /// The sender's state after base setup
    pub struct Extension {
        /// Sender's base OT choices
        pub(super) delta: Block,
        /// Receiver's rngs seeded from base OT
        pub(super) rngs: Vec<ChaCha20Rng>,
        /// Sender's keys
        pub(super) keys: Vec<[Block; 2]>,
        /// Number of OTs sent so far
        pub(super) counter: usize,

        /// Sender's unchecked qs
        pub(super) unchecked_qs: Vec<Block>,
        /// Sender's unchecked keys
        pub(super) unchecked_keys: Vec<[Block; 2]>,
    }

    impl State for Extension {}

    opaque_debug::implement!(Extension);
}
