use std::{collections::VecDeque, mem};

use crate::{
    TransferId,
    rcot::{RCOTSender, RCOTSenderOutput},
    softspoken::{
        CSP, Check, Corrections, Extend, SSP, SenderConfig, SenderError, check, fold,
        fold::TILE_TARGET_BLOCKS, ggm,
    },
};

use itybity::ToBits;
use mpz_common::future::{MaybeDone, Sender as OutputSender, new_output};
use mpz_core::{Block, aes::FixedKeyAes};
use mpz_fields::{ExtensionField, gf2::Gf2};

use rand::{Rng as _, rng};

use zerocopy::{FromBytes, IntoBytes};

#[cfg(feature = "rayon")]
use rayon::prelude::*;

#[allow(clippy::too_many_arguments)]
fn fold_block(
    aes: &FixedKeyAes,
    leaves: &[[u8; 16]],
    slab: &mut [u8],
    us_row: &[u8],
    src: &mut [[u8; 16]],
    scratch: &mut [[u8; 16]],
    u_tile: &mut [u8],
    t_b: &mut [u8],
    missing: usize,
    delta_bits: &[bool],
    k: usize,
    stride: usize,
    col_base: usize,
    ctr_base: u64,
    m: usize,
    tile_blocks: usize,
) {
    let q = 1 << k;
    let mut tc = 0;
    while tc < m {
        let tw = tile_blocks.min(m - tc);
        let tb = tw * 16;
        let ctr = ctr_base + tc as u64;
        fold::stretch(aes, leaves, tw, ctr, src, scratch);
        scratch[missing * tw..(missing + 1) * tw].fill([0u8; 16]);

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

        t_b[..tb].copy_from_slice(&u_tile[..tb]);
        fold::xor_into(&mut t_b[..tb], &us_row[tc * 16..tc * 16 + tb]);
        for i in 0..k {
            let mask = [0u8.wrapping_sub(delta_bits[i] as u8); 16];
            fold::xor_masked_into(
                &mut slab[i * stride + col..i * stride + col + tb],
                &t_b[..tb],
                mask,
            );
        }
        tc += tw;
    }
}

#[derive(Debug)]
struct Queued {
    count: usize,
    sender: OutputSender<RCOTSenderOutput<Block>>,
}

#[derive(Debug)]
/// SoftSpoken correlated OT sender.
///
/// The type parameter tracks the protocol state; see [`state`].
pub struct Sender<T: state::State = state::Initialized> {
    config: SenderConfig,
    alloc: usize,
    queue: VecDeque<Queued>,
    transfer_id: TransferId,
    delta: Block,
    state: T,
}

impl<T> Sender<T>
where
    T: state::State,
{
    /// Returns the sender's configuration.
    pub fn config(&self) -> &SenderConfig {
        &self.config
    }
}

impl Sender<state::Initialized> {
    /// Creates a new sender with the given configuration and COT correlation
    /// `delta`.
    pub fn new(config: SenderConfig, delta: Block) -> Self {
        Sender {
            config,
            alloc: 0,
            transfer_id: TransferId::default(),
            queue: VecDeque::default(),
            delta,
            state: state::Initialized::default(),
        }
    }

    /// Loads the base OT `seeds`, advancing to the [`Setup`](state::Setup)
    /// state.
    pub fn setup(self, seeds: [Block; CSP]) -> Sender<state::Setup> {
        Sender {
            config: self.config,
            alloc: self.alloc,
            transfer_id: self.transfer_id,
            queue: self.queue,
            delta: self.delta,
            state: state::Setup {
                singles: seeds.iter().map(|s| s.to_bytes()).collect(),
            },
        }
    }
}

impl Sender<state::Setup> {
    /// Applies the receiver's [`Corrections`], advancing to the
    /// [`Extension`](state::Extension) state.
    pub fn corrections(self, corrections: Corrections) -> Sender<state::Extension> {
        let k = self.config.k();
        let n_blocks = self.config.n_blocks();
        let q = self.config.leaves();

        let Corrections { corrections, s } = corrections;
        let hasher = FixedKeyAes::new(s);
        let delta_bits: Vec<bool> = self.delta.iter_lsb0().collect();
        let singles = self.state.singles;
        let aes = ggm::expander();
        let mut leaf_seeds = vec![[0u8; 16]; n_blocks * q];
        let mut missing = vec![0usize; n_blocks];
        let mut parents = vec![[0u8; 16]; q / 2];
        for b in 0..n_blocks {
            let mut idx = 0;
            for i in 0..k {
                if delta_bits[b * k + i] {
                    idx |= 1 << i;
                }
            }
            ggm::build_punctured(
                &aes,
                idx,
                &singles[b * k..b * k + k],
                &corrections[2 * k * b..2 * k * (b + 1)],
                &mut leaf_seeds[b * q..(b + 1) * q],
                &mut parents,
            );
            missing[b] = idx;
        }

        Sender {
            config: self.config,
            alloc: self.alloc,
            transfer_id: self.transfer_id,
            queue: self.queue,
            delta: self.delta,
            state: state::Extension {
                hasher,
                leaf_seeds,
                missing,
                mac: Vec::default(),
                round_rb: 0,
                round_remaining: 0,
                col_filled: 0,
                prg_ctr: 0,
                output_len: 0,
                chi: None,
            },
        }
    }
}

impl Sender<state::Extension> {
    /// Returns `true` if the sender has work to [`extend`](Self::extend).
    pub fn wants_extend(&self) -> bool {
        self.state.round_remaining != 0 || (self.state.round_rb == 0 && self.alloc != 0)
    }

    /// Returns `true` if the sender is ready to run the consistency
    /// [`check`](Self::check).
    pub fn wants_check(&self) -> bool {
        self.state.round_rb != 0 && self.state.round_remaining == 0
    }

    /// Processes one [`Extend`] message from the receiver.
    ///
    /// # Errors
    ///
    /// Returns an error if the message does not match the sender's expected
    /// state.
    pub fn extend(&mut self, extend: Extend) -> Result<(), SenderError> {
        if self.state.round_rb == 0 {
            if self.alloc == 0 {
                return Err(SenderError::InvalidState("nothing to extend".to_string()));
            }
            let round_total = (self.alloc + SSP).next_multiple_of(SSP);
            self.state.round_rb = round_total / 8;
            self.state.round_remaining = round_total;
            self.state.col_filled = 0;
            self.alloc = 0;

            let needed = self.state.output_len * 16 + CSP * self.state.round_rb;
            if self.state.mac.len() < needed {
                self.state.mac.resize(needed, 0);
            }
        }

        let Extend { count, us } = extend;

        let k = self.config.k();
        let n_blocks = self.config.n_blocks();
        let q = self.config.leaves();

        let expected_count = self
            .config
            .batch_size()
            .min(self.state.round_remaining)
            .next_multiple_of(SSP);
        if count != expected_count {
            return Err(SenderError::CountMismatch {
                expected: expected_count,
                actual: count,
            });
        }

        let rb = count / 8;
        let m = count / CSP;

        if us.len() != n_blocks * rb {
            return Err(SenderError::InvalidExtend);
        }

        let delta_bits: Vec<bool> = self.delta.iter_lsb0().collect();

        let round_rb = self.state.round_rb;
        let col_filled = self.state.col_filled;
        let ctr_base = self.state.prg_ctr;
        let tile_blocks = (TILE_TARGET_BLOCKS / q).max(1).min(m);

        let work_start = self.state.output_len * 16;

        let hasher = &self.state.hasher;
        let leaf_seeds = &self.state.leaf_seeds;
        let missing = &self.state.missing;
        let work = &mut self.state.mac[work_start..work_start + CSP * round_rb];
        let delta_bits = &delta_bits;
        let make_buf = || {
            (
                vec![[0u8; 16]; q * tile_blocks],
                vec![[0u8; 16]; q * tile_blocks],
                vec![0u8; tile_blocks * 16],
                vec![0u8; tile_blocks * 16],
            )
        };

        cfg_if::cfg_if! {
            if #[cfg(feature = "rayon")] {
                leaf_seeds
                    .par_chunks(q)
                    .zip(work.par_chunks_mut(k * round_rb))
                    .zip(us.par_chunks(rb))
                    .enumerate()
                    .for_each_init(make_buf, |(src, scratch, u_tile, t_b), (b, ((leaves, slab), us_row))| {
                        fold_block(
                            hasher, leaves, slab, us_row, src, scratch, u_tile, t_b, missing[b],
                            &delta_bits[b * k..b * k + k], k, round_rb, col_filled, ctr_base, m,
                            tile_blocks,
                        );
                    });
            } else {
                let (mut src, mut scratch, mut u_tile, mut t_b) = make_buf();
                leaf_seeds
                    .chunks(q)
                    .zip(work.chunks_mut(k * round_rb))
                    .zip(us.chunks(rb))
                    .enumerate()
                    .for_each(|(b, ((leaves, slab), us_row))| {
                        fold_block(
                            hasher, leaves, slab, us_row, &mut src, &mut scratch, &mut u_tile,
                            &mut t_b, missing[b], &delta_bits[b * k..b * k + k], k, round_rb,
                            col_filled, ctr_base, m, tile_blocks,
                        );
                    });
            }
        }

        self.state.col_filled = col_filled + m;
        self.state.prg_ctr = ctr_base + m as u64;
        self.state.round_remaining -= count;

        Ok(())
    }

    /// Samples and returns the challenge seed for the consistency check.
    ///
    /// Send it to the receiver, then pass its [`Check`] response to
    /// [`check`](Self::check).
    pub fn check_start(&mut self) -> Block {
        let chi = rng().random::<Block>();
        self.state.chi = Some(chi);
        chi
    }

    /// Verifies the receiver's [`Check`] and finalizes the extended OTs, which
    /// then become available to send.
    ///
    /// # Errors
    ///
    /// Returns [`SenderError::ConsistencyCheckFailed`] if verification fails.
    pub fn check(&mut self, receiver_check: Check) -> Result<(), SenderError> {
        if !self.wants_check() {
            return Err(SenderError::InvalidState("not ready to check".to_string()));
        }
        let chi_seed = mem::take(&mut self.state.chi).ok_or(SenderError::ChiNotSet)?;

        let round_rb = self.state.round_rb;
        let work_start = self.state.output_len * 16;
        let work = &self.state.mac[work_start..work_start + CSP * round_rb];

        let (check_q, _) = check::check_fold(chi_seed, work, round_rb, None);

        let Check { x, t } = receiver_check;

        let mut failed = false;
        for ((bit, t), q) in self.delta.iter_lsb0().zip(t).zip(check_q) {
            let xb = x.scale_by_subfield(Gf2(bit));
            failed |= q != t + xb;
        }
        if failed {
            return Err(SenderError::ConsistencyCheckFailed);
        }

        let work = &mut self.state.mac[work_start..work_start + CSP * round_rb];
        matrix_transpose::transpose_bits(work, CSP).expect("matrix is rectangular");
        self.state.output_len += round_rb * 8 - SSP;
        self.state.round_rb = 0;
        self.state.col_filled = 0;

        self.resolve_queue();

        Ok(())
    }

    fn resolve_queue(&mut self) {
        while let Some(front) = self.queue.front() {
            if front.count > self.state.output_len {
                break;
            }
            let Queued { count, sender } = self.queue.pop_front().expect("front exists");
            let keys = self.take_keys(count);
            sender.send(RCOTSenderOutput {
                id: self.transfer_id.next(),
                keys,
            });
        }
    }

    fn take_keys(&mut self, count: usize) -> Vec<Block> {
        self.state.output_len -= count;
        let start = self.state.output_len;
        <[Block]>::ref_from_bytes(&self.state.mac[start * 16..(start + count) * 16])
            .expect("multiple of Block size")
            .to_vec()
    }
}

impl RCOTSender<Block> for Sender<state::Initialized> {
    type Error = SenderError;
    type Future = MaybeDone<RCOTSenderOutput<Block>>;

    fn alloc(&mut self, count: usize) -> Result<(), Self::Error> {
        self.alloc += count;

        Ok(())
    }

    fn available(&self) -> usize {
        0
    }

    fn delta(&self) -> Block {
        self.delta
    }

    fn try_send_rcot(&mut self, _count: usize) -> Result<RCOTSenderOutput<Block>, Self::Error> {
        Err(SenderError::InvalidState(
            "sender has not been setup yet".to_string(),
        ))
    }

    fn queue_send_rcot(&mut self, count: usize) -> Result<Self::Future, Self::Error> {
        let (sender, recv) = new_output();

        self.queue.push_back(Queued { count, sender });

        Ok(recv)
    }
}

impl RCOTSender<Block> for Sender<state::Extension> {
    type Error = SenderError;
    type Future = MaybeDone<RCOTSenderOutput<Block>>;

    fn alloc(&mut self, count: usize) -> Result<(), Self::Error> {
        self.alloc += count;

        Ok(())
    }

    fn available(&self) -> usize {
        self.state.output_len
    }

    fn delta(&self) -> Block {
        self.delta
    }

    fn try_send_rcot(&mut self, count: usize) -> Result<RCOTSenderOutput<Block>, Self::Error> {
        if self.available() < count {
            return Err(SenderError::InsufficientSetup {
                expected: count,
                actual: self.available(),
            });
        }

        let keys = self.take_keys(count);

        Ok(RCOTSenderOutput {
            id: self.transfer_id.next(),
            keys,
        })
    }

    fn queue_send_rcot(&mut self, count: usize) -> Result<Self::Future, Self::Error> {
        if self.available() >= count {
            let output = self.try_send_rcot(count)?;
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

/// Typestates for the [`Sender`].
pub mod state {
    use super::*;

    mod sealed {
        pub trait Sealed {}

        impl Sealed for super::Initialized {}
        impl Sealed for super::Setup {}
        impl Sealed for super::Extension {}
    }

    /// A sender protocol state. This trait is sealed.
    pub trait State: sealed::Sealed {}

    /// The initial state, before base OT setup.
    #[derive(Default)]
    pub struct Initialized {}

    impl State for Initialized {}

    opaque_debug::implement!(Initialized);

    /// The state after base OT setup, awaiting the receiver's corrections.
    pub struct Setup {
        pub(super) singles: Vec<[u8; 16]>,
    }

    impl State for Setup {}

    opaque_debug::implement!(Setup);

    /// The extension state, in which OTs are generated and checked.
    pub struct Extension {
        pub(super) hasher: mpz_core::aes::FixedKeyAes,
        pub(super) leaf_seeds: Vec<[u8; 16]>,
        pub(super) missing: Vec<usize>,
        pub(super) mac: Vec<u8>,
        pub(super) round_rb: usize,
        pub(super) round_remaining: usize,
        pub(super) col_filled: usize,
        pub(super) prg_ctr: u64,
        pub(super) output_len: usize,
        pub(super) chi: Option<Block>,
    }

    impl State for Extension {}

    opaque_debug::implement!(Extension);
}
