use std::collections::VecDeque;

use crate::{
    TransferId,
    rcot::{RCOTReceiver, RCOTReceiverOutput},
    softspoken::{
        CSP, Check, Corrections, Extend, ReceiverConfig, ReceiverError, SSP, check, fold,
        fold::TILE_TARGET_BLOCKS, ggm,
    },
};

use mpz_common::future::{MaybeDone, Sender, new_output};
use mpz_core::{Block, aes::FixedKeyAes};

use rand::Rng as _;
use rand_core::RngCore;
use zerocopy::{FromBytes, IntoBytes};

#[cfg(feature = "rayon")]
use rayon::prelude::*;

#[allow(clippy::too_many_arguments)]
fn fold_block(
    aes: &FixedKeyAes,
    leaves: &[[u8; 16]],
    slab: &mut [u8],
    us_row: &mut [u8],
    src: &mut [[u8; 16]],
    scratch: &mut [[u8; 16]],
    u_tile: &mut [u8],
    k: usize,
    stride: usize,
    col_base: usize,
    ctr_base: u64,
    m: usize,
    tile_blocks: usize,
    choices: &[u8],
) {
    let q = 1 << k;
    let mut tc = 0;
    while tc < m {
        let tw = tile_blocks.min(m - tc);
        let tb = tw * 16;
        let ctr = ctr_base + tc as u64;
        fold::stretch(aes, leaves, tw, ctr, src, scratch);
        let col = (col_base + tc) * 16;
        fold::fold_emit(
            &mut scratch.as_mut_bytes()[..q * tb],
            tb,
            k,
            slab,
            stride,
            col,
            &mut u_tile[..tb],
        );
        let ch = &choices[(col_base + tc) * 16..(col_base + tc) * 16 + tb];
        let dst = &mut us_row[tc * 16..tc * 16 + tb];
        dst.copy_from_slice(&u_tile[..tb]);
        fold::xor_into(dst, ch);
        tc += tw;
    }
}

#[derive(Debug)]
struct Queued {
    count: usize,
    sender: Sender<RCOTReceiverOutput<bool, Block>>,
}

#[derive(Debug, Default)]
/// SoftSpoken correlated OT receiver.
///
/// The type parameter tracks the protocol state; see [`state`].
pub struct Receiver<T: state::State = state::Initialized> {
    config: ReceiverConfig,
    alloc: usize,
    transfer_id: TransferId,
    queue: VecDeque<Queued>,
    state: T,
}

impl<T> Receiver<T>
where
    T: state::State,
{
    /// Returns the receiver's configuration.
    pub fn config(&self) -> &ReceiverConfig {
        &self.config
    }
}

impl Receiver {
    /// Creates a new receiver with the given configuration.
    pub fn new(config: ReceiverConfig) -> Self {
        Receiver {
            config,
            alloc: 0,
            transfer_id: TransferId::default(),
            queue: VecDeque::default(),
            state: state::Initialized {},
        }
    }

    /// Loads the base OT `seeds`, advancing to the [`Setup`](state::Setup)
    /// state.
    pub fn setup(self, seeds: [[Block; 2]; CSP]) -> Receiver<state::Setup> {
        Receiver {
            config: self.config,
            alloc: self.alloc,
            transfer_id: self.transfer_id,
            queue: self.queue,
            state: state::Setup {
                seeds: seeds
                    .iter()
                    .map(|[a, b]| [a.to_bytes(), b.to_bytes()])
                    .collect(),
            },
        }
    }
}

impl Receiver<state::Setup> {
    /// Produces the [`Corrections`] for the sender, advancing to the
    /// [`Extension`](state::Extension) state.
    pub fn corrections(self) -> (Receiver<state::Extension>, Corrections) {
        let k = self.config.k();
        let n_blocks = self.config.n_blocks();
        let q = self.config.leaves();

        let mut rng = rand::rng();
        let s: [u8; 16] = rng.random();
        let hasher = FixedKeyAes::new(s);

        let seeds = self.state.seeds;
        let aes = ggm::expander();
        let mut leaf_seeds = vec![[0u8; 16]; n_blocks * q];
        let mut corrections = vec![[0u8; 16]; 2 * CSP];
        let mut parents = vec![[0u8; 16]; q / 2];
        for b in 0..n_blocks {
            let root: [u8; 16] = rng.random();
            ggm::build_full(
                &aes,
                root,
                &seeds[b * k..b * k + k],
                &mut leaf_seeds[b * q..(b + 1) * q],
                &mut corrections[2 * k * b..2 * k * (b + 1)],
                &mut parents,
            );
        }

        let receiver = Receiver {
            config: self.config,
            alloc: self.alloc,
            transfer_id: self.transfer_id,
            queue: self.queue,
            state: state::Extension {
                hasher,
                leaf_seeds,
                mac: Vec::default(),
                round_choices: Vec::default(),
                out_choices: Vec::default(),
                round_rb: 0,
                round_remaining: 0,
                col_filled: 0,
                prg_ctr: 0,
                output_len: 0,
            },
        };

        (receiver, Corrections { corrections, s })
    }
}

impl Receiver<state::Extension> {
    /// Returns `true` if the receiver has work to [`extend`](Self::extend).
    pub fn wants_extend(&self) -> bool {
        self.state.round_remaining != 0 || (self.state.round_rb == 0 && self.alloc != 0)
    }

    /// Returns `true` if the receiver is ready to run the consistency
    /// [`check`](Self::check).
    pub fn wants_check(&self) -> bool {
        self.state.round_rb != 0 && self.state.round_remaining == 0
    }

    /// Produces the next [`Extend`] message for the sender.
    ///
    /// # Errors
    ///
    /// Returns an error if the receiver is not in a state to extend.
    pub fn extend(&mut self) -> Result<Extend, ReceiverError> {
        let k = self.config.k();
        let n_blocks = self.config.n_blocks();
        let q = self.config.leaves();

        if self.state.round_rb == 0 {
            if self.alloc == 0 {
                return Err(ReceiverError::InvalidState("nothing to extend".to_string()));
            }
            let round_total = (self.alloc + SSP).next_multiple_of(SSP);
            let round_rb = round_total / 8;
            self.state.round_rb = round_rb;
            self.state.round_remaining = round_total;
            self.state.col_filled = 0;
            self.alloc = 0;

            let needed = self.state.output_len * 16 + CSP * round_rb;
            if self.state.mac.len() < needed {
                self.state.mac.resize(needed, 0);
            }
            self.state.round_choices.resize(round_rb, 0);
        }

        let count = self
            .config
            .batch_size()
            .min(self.state.round_remaining)
            .next_multiple_of(SSP);
        let rb = count / 8;
        let m = count / CSP;

        let round_rb = self.state.round_rb;
        let col_filled = self.state.col_filled;
        let ctr_base = self.state.prg_ctr;
        let tile_blocks = (TILE_TARGET_BLOCKS / q).max(1).min(m);

        rand::rng()
            .fill_bytes(&mut self.state.round_choices[col_filled * 16..col_filled * 16 + rb]);

        let mut us = vec![0u8; n_blocks * rb];

        let work_start = self.state.output_len * 16;

        let hasher = &self.state.hasher;
        let leaf_seeds = &self.state.leaf_seeds;
        let work = &mut self.state.mac[work_start..work_start + CSP * round_rb];
        let choices = &self.state.round_choices;
        let make_buf = || {
            (
                vec![[0u8; 16]; q * tile_blocks],
                vec![[0u8; 16]; q * tile_blocks],
                vec![0u8; tile_blocks * 16],
            )
        };

        cfg_if::cfg_if! {
            if #[cfg(feature = "rayon")] {
                leaf_seeds
                    .par_chunks(q)
                    .zip(work.par_chunks_mut(k * round_rb))
                    .zip(us.par_chunks_mut(rb))
                    .for_each_init(make_buf, |(src, scratch, u_tile), ((leaves, slab), us_row)| {
                        fold_block(
                            hasher, leaves, slab, us_row, src, scratch, u_tile, k, round_rb,
                            col_filled, ctr_base, m, tile_blocks, choices,
                        );
                    });
            } else {
                let (mut src, mut scratch, mut u_tile) = make_buf();
                leaf_seeds
                    .chunks(q)
                    .zip(work.chunks_mut(k * round_rb))
                    .zip(us.chunks_mut(rb))
                    .for_each(|((leaves, slab), us_row)| {
                        fold_block(
                            hasher, leaves, slab, us_row, &mut src, &mut scratch, &mut u_tile, k,
                            round_rb, col_filled, ctr_base, m, tile_blocks, choices,
                        );
                    });
            }
        }

        self.state.col_filled = col_filled + m;
        self.state.prg_ctr = ctr_base + m as u64;
        self.state.round_remaining -= count;

        Ok(Extend { count, us })
    }

    /// Answers the sender's challenge `chi_seed`, producing the [`Check`]
    /// message and finalizing the extended OTs, which then become available to
    /// receive.
    ///
    /// # Errors
    ///
    /// Returns an error if the receiver is not ready to check.
    pub fn check(&mut self, chi_seed: Block) -> Result<Check, ReceiverError> {
        if !self.wants_check() {
            return Err(ReceiverError::InvalidState(
                "receiver not ready to check".to_string(),
            ));
        }

        let round_rb = self.state.round_rb;
        let work_start = self.state.output_len * 16;
        let round_out = round_rb * 8 - SSP;

        let (check_t, check_x) = check::check_fold(
            chi_seed,
            &self.state.mac[work_start..work_start + CSP * round_rb],
            round_rb,
            Some(&self.state.round_choices[..round_rb]),
        );
        let check_x = check_x.expect("choices were provided");

        {
            let choices = &self.state.round_choices;
            let out = &mut self.state.out_choices;
            out.extend((0..round_out).map(|i| (choices[i / 8] >> (i % 8)) & 1 == 1));
        }

        let work = &mut self.state.mac[work_start..work_start + CSP * round_rb];
        matrix_transpose::transpose_bits(work, CSP).expect("matrix is rectangular");
        self.state.output_len += round_out;
        self.state.round_rb = 0;
        self.state.col_filled = 0;

        self.resolve_queue();

        Ok(Check {
            x: check_x,
            t: check_t,
        })
    }

    fn resolve_queue(&mut self) {
        while let Some(front) = self.queue.front() {
            if front.count > self.state.output_len {
                break;
            }
            let Queued { count, sender } = self.queue.pop_front().expect("front exists");
            let (choices, msgs) = self.take_output(count);
            sender.send(RCOTReceiverOutput {
                id: self.transfer_id.next(),
                choices,
                msgs,
            });
        }
    }

    fn take_output(&mut self, count: usize) -> (Vec<bool>, Vec<Block>) {
        self.state.output_len -= count;
        let start = self.state.output_len;
        let msgs = <[Block]>::ref_from_bytes(&self.state.mac[start * 16..(start + count) * 16])
            .expect("multiple of Block size")
            .to_vec();
        let choices = self.state.out_choices[start..start + count].to_vec();
        self.state.out_choices.truncate(start);
        (choices, msgs)
    }
}

impl RCOTReceiver<bool, Block> for Receiver<state::Initialized> {
    type Error = ReceiverError;
    type Future = MaybeDone<RCOTReceiverOutput<bool, Block>>;

    fn alloc(&mut self, count: usize) -> Result<(), Self::Error> {
        self.alloc += count;

        Ok(())
    }

    fn available(&self) -> usize {
        0
    }

    fn try_recv_rcot(
        &mut self,
        _count: usize,
    ) -> Result<RCOTReceiverOutput<bool, Block>, Self::Error> {
        Err(ReceiverError::InvalidState(
            "receiver has not been set up yet".to_string(),
        ))
    }

    fn queue_recv_rcot(&mut self, count: usize) -> Result<Self::Future, Self::Error> {
        let (sender, recv) = new_output();

        self.queue.push_back(Queued { count, sender });

        Ok(recv)
    }
}

impl RCOTReceiver<bool, Block> for Receiver<state::Extension> {
    type Error = ReceiverError;
    type Future = MaybeDone<RCOTReceiverOutput<bool, Block>>;

    fn alloc(&mut self, count: usize) -> Result<(), Self::Error> {
        self.alloc += count;

        Ok(())
    }

    fn available(&self) -> usize {
        self.state.output_len
    }

    fn try_recv_rcot(
        &mut self,
        count: usize,
    ) -> Result<RCOTReceiverOutput<bool, Block>, Self::Error> {
        if self.available() < count {
            return Err(ReceiverError::InsufficientSetup {
                expected: count,
                actual: self.available(),
            });
        }

        let (choices, msgs) = self.take_output(count);

        Ok(RCOTReceiverOutput {
            id: self.transfer_id.next(),
            choices,
            msgs,
        })
    }

    fn queue_recv_rcot(&mut self, count: usize) -> Result<Self::Future, Self::Error> {
        if self.available() >= count {
            let output = self.try_recv_rcot(count)?;
            let (sender, recv) = new_output();
            sender.send(output);

            Ok(recv)
        } else {
            let (sender, recv) = new_output();

            self.queue.push_back(Queued { count, sender });

            Ok(recv)
        }
    }
}

/// Typestates for the [`Receiver`].
pub mod state {
    mod sealed {
        pub trait Sealed {}

        impl Sealed for super::Initialized {}
        impl Sealed for super::Setup {}
        impl Sealed for super::Extension {}
    }

    /// A receiver protocol state. This trait is sealed.
    pub trait State: sealed::Sealed {}

    /// The initial state, before base OT setup.
    #[derive(Default)]
    pub struct Initialized {}

    impl State for Initialized {}

    opaque_debug::implement!(Initialized);

    /// The state after base OT setup, before producing the corrections.
    pub struct Setup {
        pub(super) seeds: Vec<[[u8; 16]; 2]>,
    }

    impl State for Setup {}

    opaque_debug::implement!(Setup);

    /// The extension state, in which OTs are generated and checked.
    pub struct Extension {
        pub(super) hasher: mpz_core::aes::FixedKeyAes,
        pub(super) leaf_seeds: Vec<[u8; 16]>,
        pub(super) mac: Vec<u8>,
        pub(super) round_choices: Vec<u8>,
        pub(super) out_choices: Vec<bool>,
        pub(super) round_rb: usize,
        pub(super) round_remaining: usize,
        pub(super) col_filled: usize,
        pub(super) prg_ctr: u64,
        pub(super) output_len: usize,
    }

    impl State for Extension {}

    opaque_debug::implement!(Extension);
}
