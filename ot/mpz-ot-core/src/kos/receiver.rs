use std::collections::HashMap;

use crate::{
    kos::{
        error::ReceiverVerifyError,
        msgs::{Check, Extend, SenderPayload},
        ReceiverConfig, ReceiverError, Rng, RngSeed, CSP, SSP,
    },
    msgs::Derandomize,
};

use itybity::{FromBitIterator, ToBits};
use mpz_core::{aes::FIXED_KEY_AES, Block};

use blake3::Hasher;
use rand::{thread_rng, Rng as _, SeedableRng};
use rand_chacha::ChaCha20Rng;
use rand_core::RngCore;
use utils::bits::ToBitsIter;

#[cfg(feature = "rayon")]
use rayon::prelude::*;

#[derive(Debug, Default)]
struct PayloadRecord {
    counter: usize,
    choices: Vec<u8>,
    ts: Vec<Block>,
    keys: Vec<Block>,
    ciphertext_digest: [u8; 32],
}

#[derive(Debug, Default)]
struct Tape {
    records: HashMap<usize, PayloadRecord>,
}

/// KOS15 receiver.
#[derive(Debug, Default)]
pub struct Receiver<T: state::State = state::Initialized> {
    config: ReceiverConfig,
    state: T,
    /// Protocol tape
    tape: Option<Tape>,
}

impl Receiver {
    /// Creates a new Sender
    ///
    /// # Arguments
    ///
    /// * `config` - The Sender's configuration
    pub fn new(config: ReceiverConfig) -> Self {
        let tape = if config.sender_commit() {
            Some(Tape::default())
        } else {
            None
        };

        Receiver {
            config,
            state: state::Initialized::default(),
            tape,
        }
    }

    /// Complete the base setup phase of the protocol.
    ///
    /// # Arguments
    ///
    /// * `seeds` - The receiver's rng seeds
    pub fn base_setup(self, seeds: [[Block; 2]; CSP]) -> Receiver<state::Extension> {
        let rngs = seeds
            .iter()
            .map(|seeds| {
                seeds.map(|seed| {
                    let mut seed_ = RngSeed::default();
                    seed_
                        .iter_mut()
                        .zip(seed.to_bytes().into_iter().cycle())
                        .for_each(|(s, c)| *s = c);
                    Rng::from_seed(seed_)
                })
            })
            .collect();

        Receiver {
            config: self.config,
            state: state::Extension {
                rngs,
                ts: Vec::default(),
                keys: Vec::default(),
                choices: Vec::default(),
                key_counter: 0,
                ot_counter: 0,
                payload_counter: 0,
                unchecked_ts: Vec::default(),
                unchecked_choices: Vec::default(),
            },
            tape: self.tape,
        }
    }
}

impl Receiver<state::Extension> {
    /// Returns the number of payloads that have been received.
    pub fn payload_count(&self) -> usize {
        self.state.payload_counter
    }

    /// The number of remaining OTs which can be consumed.
    pub fn remaining(&self) -> usize {
        self.state.keys.len()
    }

    /// Perform the IKNP OT extension.
    ///
    /// # Sacrificial OTs
    ///
    /// Performing the consistency check sacrifices 256 OTs for the consistency check, so be sure to
    /// extend enough OTs to compensate for this.
    ///
    /// # Streaming
    ///
    /// Extension can be performed in a streaming fashion by calling this method multiple times, sending
    /// the `Extend` messages to the sender in-between calls.
    ///
    /// The freshly extended OTs are not available until after the consistency check has been
    /// performed. See [`Receiver::check`].
    ///
    /// # Arguments
    ///
    /// * `choices` - The receiver's choices
    pub fn extend(&mut self, count: usize) -> Extend {
        // Round up the OTs to extend to the nearest multiple of 64 (matrix transpose optimization).
        let count = (count + 63) & !63;

        const NROWS: usize = CSP;
        let row_width = count / 8;

        let mut rng = thread_rng();
        let choices = (0..row_width)
            .flat_map(|_| rng.gen::<u8>().into_lsb0_iter())
            .collect::<Vec<_>>();

        let choice_vector = Vec::<u8>::from_lsb0_iter(choices.iter().copied());

        let mut ts = vec![0u8; NROWS * row_width];
        let mut us = vec![0u8; NROWS * row_width];
        cfg_if::cfg_if! {
            if #[cfg(feature = "rayon")] {
                let iter = self.state.rngs
                    .par_iter_mut()
                    .zip(ts.par_chunks_exact_mut(row_width))
                    .zip(us.par_chunks_exact_mut(row_width));
            } else {
                let iter = self.state.rngs
                    .iter_mut()
                    .zip(ts.chunks_exact_mut(row_width))
                    .zip(uss.chunks_exact_mut(row_width));
            }
        }

        iter.for_each(|((rngs, t), u)| {
            // Figure 3, step 2.
            rngs[0].fill_bytes(t);
            rngs[1].fill_bytes(u);

            // Figure 3, step 3.
            // Computing `u = t_0 + t_1 + x`.
            u.iter_mut()
                .zip(t)
                .zip(&choice_vector)
                .for_each(|((u, t), r)| {
                    *u ^= *t ^ r;
                });
        });

        matrix_transpose::transpose_bits(&mut ts, NROWS).expect("matrix is rectangular");

        self.state.unchecked_ts.extend(
            ts.chunks_exact(NROWS / 8)
                .map(|t| Block::try_from(t).unwrap()),
        );
        self.state.unchecked_choices.extend(choices);

        Extend { count, us }
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
    /// coin-toss protocol **after** the receiver has sent their setup message, ie
    /// after they have already committed to their choice vectors.
    ///
    /// # Arguments
    ///
    /// * `chi_seed` - The seed used to generate the consistency check weights.
    pub fn check(&mut self, chi_seed: Block) -> Check {
        let mut seed = RngSeed::default();
        seed.iter_mut()
            .zip(chi_seed.to_bytes().into_iter().cycle())
            .for_each(|(s, c)| *s = c);

        let mut rng = Rng::from_seed(seed);

        let mut unchecked_ts = std::mem::take(&mut self.state.unchecked_ts);
        let mut unchecked_choices = std::mem::take(&mut self.state.unchecked_choices);

        // Figure 7, "Check correlation", point 1.
        // Sample random weights for the consistency check.
        let chis = (0..unchecked_ts.len())
            .map(|_| Block::random(&mut rng))
            .collect::<Vec<_>>();

        // Figure 7, "Check correlation", point 2.
        // Compute the random linear combinations.
        cfg_if::cfg_if! {
            if #[cfg(feature = "rayon")] {
                let (x, t0, t1) = unchecked_choices.par_iter()
                    .zip(&unchecked_ts)
                    .zip(chis)
                    .map(|((c, t), chi)| {
                        let x = if *c { chi } else { Block::ZERO };
                        let (t0, t1) = t.clmul(chi);
                        (x, t0, t1)
                    })
                    .reduce(
                        || (Block::ZERO, Block::ZERO, Block::ZERO),
                        |(_x, _t0, _t1), (x, t0, t1)| {
                            (_x ^ x, _t0 ^ t0, _t1 ^ t1)
                        },
                    );
            } else {
                use itybity::ToBits;

                let (x, t0, t1) = unchecked_choices.iter()
                    .zip(&unchecked_ts)
                    .zip(chis)
                    .map(|((c, t), chi)| {
                        let x = if *c { chi } else { Block::ZERO };
                        let (t0, t1) = t.clmul(chi);
                        (x, t0, t1)
                    })
                    .reduce(|(_x, _t0, _t1), (x, t0, t1)| {
                        (_x ^ x, _t0 ^ t0, _t1 ^ t1)
                    }).unwrap();
            }
        }

        // Strip off the rows sacrificed for the consistency check.
        let nrows = unchecked_ts.len() - (CSP + SSP);
        unchecked_ts.truncate(nrows);
        unchecked_choices.truncate(nrows);

        cfg_if::cfg_if! {
            if #[cfg(feature = "rayon")] {
                let iter = unchecked_ts.par_iter().enumerate();
            } else {
                let iter = unchecked_ts.iter().enumerate();
            }
        }

        let cipher = &(*FIXED_KEY_AES);
        let keys = iter
            .map(|(j, t)| {
                let j = Block::from(((self.state.key_counter + j) as u128).to_be_bytes());
                cipher.tccr(j, *t)
            })
            .collect::<Vec<_>>();

        self.state.key_counter += keys.len();

        // Add to existing keys.
        self.state.keys.extend(keys);
        self.state.choices.extend(unchecked_choices);

        // If we're recording, we track `ts` too
        if self.tape.is_some() {
            self.state.ts.extend(unchecked_ts);
        }

        Check { x, t0, t1 }
    }

    /// Derandomize the receiver's choices from the setup phase.
    ///
    /// # Arguments
    ///
    /// * `choices` - The receiver's corrected choices.
    pub fn derandomize(&mut self, choices: &[bool]) -> Derandomize {
        let flip = Vec::<u8>::from_lsb0_iter(
            self.state
                .choices
                .iter()
                .zip(choices)
                .map(|(setup_choice, new_choice)| setup_choice ^ new_choice),
        );

        self.state.choices[..choices.len()].copy_from_slice(choices);

        Derandomize { flip }
    }

    /// Obliviously receive the sender's messages.
    ///
    /// # Arguments
    ///
    /// * `payload` - The sender's payload
    pub fn receive(&mut self, payload: SenderPayload) -> Result<Vec<Block>, ReceiverError> {
        let SenderPayload { ciphertexts } = payload;

        if ciphertexts.len() % 2 != 0 {
            return Err(ReceiverError::InvalidPayload);
        }

        let count = ciphertexts.len() / 2;

        if count > self.state.keys.len() {
            return Err(ReceiverError::CountMismatch(self.state.keys.len(), count));
        }

        let keys = Vec::from_iter(self.state.keys.drain(..count));
        let choices = Vec::from_iter(self.state.choices.drain(..count));

        if let Some(tape) = &mut self.tape {
            let ts = Vec::from_iter(self.state.ts.drain(..count));

            let mut hasher = Hasher::default();
            ciphertexts.iter().for_each(|ct| {
                hasher.update(&ct.to_bytes());
            });

            tape.records.insert(
                self.state.payload_counter,
                PayloadRecord {
                    counter: self.state.ot_counter,
                    choices: Vec::from_lsb0_iter(choices.iter().copied()),
                    ts,
                    keys: keys.clone(),
                    ciphertext_digest: hasher.finalize().into(),
                },
            );
        }

        let plaintexts = keys
            .into_iter()
            .zip(choices)
            .zip(ciphertexts.chunks(2))
            .map(|((key, c), ct)| if c { key ^ ct[1] } else { key ^ ct[0] })
            .collect();

        self.state.ot_counter += count;
        self.state.payload_counter += 1;

        Ok(plaintexts)
    }

    /// Checks the purported messages against the receiver's protocol tape, using the sender's
    /// base choices `delta`.
    ///
    /// # ⚠️ Warning ⚠️
    ///
    /// The authenticity of `delta` must be established outside the context of this function. This
    /// can be achieved using verifiable OT for the base choices.
    ///
    /// # Arguments
    ///
    /// * `index` - The index of the payload.
    /// * `delta` - The sender's base choices.
    /// * `purported_msgs` - The purported messages sent by the sender.
    pub fn verify(
        &self,
        index: usize,
        delta: Block,
        purported_msgs: &[[Block; 2]],
    ) -> Result<(), ReceiverError> {
        let Some(tape) = &self.tape else {
            return Err(ReceiverVerifyError::TapeNotRecorded)?;
        };

        let Some(record) = tape.records.get(&index) else {
            return Err(ReceiverVerifyError::InvalidPayloadIndex)?;
        };

        let PayloadRecord {
            counter,
            choices,
            ts,
            keys,
            ciphertext_digest,
        } = record;

        // Here we compute the complementary key to the one used earlier in the protocol.
        //
        // From this, we encrypt the purported messages and check that the ciphertext digests match.
        let cipher = &(*FIXED_KEY_AES);
        let mut hasher = Hasher::default();
        for (j, (((c, t), key), msgs)) in choices
            .iter_lsb0()
            .zip(ts)
            .zip(keys)
            .zip(purported_msgs)
            .enumerate()
        {
            let j = Block::new(((counter + j) as u128).to_be_bytes());
            let key_ = cipher.tccr(j, *t ^ delta);

            let (ct0, ct1) = if c {
                (msgs[0] ^ key_, msgs[1] ^ *key)
            } else {
                (msgs[0] ^ *key, msgs[1] ^ key_)
            };

            hasher.update(&ct0.to_bytes());
            hasher.update(&ct1.to_bytes());
        }

        let digest: [u8; 32] = hasher.finalize().into();

        if *ciphertext_digest != digest {
            return Err(ReceiverVerifyError::InconsistentPayload)?;
        }

        Ok(())
    }
}

/// The receiver's state.
pub mod state {
    use super::*;

    mod sealed {
        pub trait Sealed {}

        impl Sealed for super::Initialized {}
        impl Sealed for super::Extension {}
    }

    /// The receiver's state.
    pub trait State: sealed::Sealed {}

    /// The receiver's initial state.
    #[derive(Default)]
    pub struct Initialized {}

    impl State for Initialized {}

    opaque_debug::implement!(Initialized);

    /// The receiver's state after base setup
    pub struct Extension {
        /// Receiver's rngs
        pub(super) rngs: Vec<[ChaCha20Rng; 2]>,
        /// Receiver's ts
        pub(super) ts: Vec<Block>,
        /// Receiver's keys
        pub(super) keys: Vec<Block>,
        /// Receiver's random choices
        pub(super) choices: Vec<bool>,
        /// Keys computed so far
        pub(super) key_counter: usize,
        /// OTs received so far
        pub(super) ot_counter: usize,
        /// Payloads received so far
        pub(super) payload_counter: usize,

        /// Receiver's unchecked ts
        pub(super) unchecked_ts: Vec<Block>,
        /// Receiver's unchecked choices
        pub(super) unchecked_choices: Vec<bool>,
    }

    impl State for Extension {}

    opaque_debug::implement!(Extension);
}
