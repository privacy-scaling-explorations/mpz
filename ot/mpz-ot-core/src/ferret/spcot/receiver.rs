//! SPCOT receiver
use crate::ferret::{spcot::error::ReceiverError, CSP};
use itybity::ToBits;
use mpz_core::{aes::FIXED_KEY_AES, ggm_tree::GgmTree, hash::Hash, prg::Prg, Block};
use rand_core::SeedableRng;

use super::msgs::{
    CheckFromReceiver, CheckFromSender, CheckReceiverFromCOT, ExtendFromSender,
    ExtendReceiverFromCOT, MaskBits,
};

/// SPCOT receiver.
#[derive(Debug, Default)]
pub struct Receiver<T: state::State = state::Initialized> {
    state: T,
}

impl Receiver {
    /// Creates a new Receiver.
    pub fn new() -> Self {
        Receiver {
            state: state::Initialized::default(),
        }
    }

    /// Completes the setup phase of the protocol.
    ///
    /// See step 1 in Figure 6.
    ///
    /// # Arguments
    ///
    /// * `seed` - The random seed to generate PRG.
    pub fn setup(self, seed: Block) -> Receiver<state::Extension> {
        Receiver {
            state: state::Extension {
                ws: Vec::default(),
                cot_counter: 0,
                exec_counter: 0,
                prg: Prg::from_seed(seed),
            },
        }
    }
}

impl Receiver<state::Extension> {
    ///
    pub fn extend_mask_bits(
        &mut self,
        h: usize,
        alpha: usize,
        extend: ExtendReceiverFromCOT,
    ) -> Result<MaskBits, ReceiverError> {
        if alpha > (1 << h) {
            return Err(ReceiverError::InvalidInput(
                "the input pos should be no more than 2^h".to_string(),
            ));
        }

        let ExtendReceiverFromCOT { rs, ts: _ } = extend;

        if rs.len() != h {
            return Err(ReceiverError::InvalidLength(
                "the length of b should be h".to_string(),
            ));
        }

        // Step 4 in Figure 6
        let bs: Vec<bool> = alpha.to_msb0_vec()[0..h]
            .iter()
            // Computes alpha_i XOR r_i.
            .zip(rs.iter())
            .map(|(alpha, r)| if alpha == r { false } else { true })
            // Computes alpha_i XOR r_i XOR 1
            .map(|b| if b { false } else { true })
            .collect();

        Ok(MaskBits { bs })
    }

    ///
    pub fn extend(
        &mut self,
        h: usize,
        alpha: usize,
        extendfc: ExtendReceiverFromCOT,
        extendfr: ExtendFromSender,
    ) -> Result<(), ReceiverError> {
        if alpha > (1 << h) {
            return Err(ReceiverError::InvalidInput(
                "the input pos should be no more than 2^h".to_string(),
            ));
        }

        let ExtendReceiverFromCOT { rs: _, ts } = extendfc;
        let ExtendFromSender { ms, sum } = extendfr;
        if ts.len() != h {
            return Err(ReceiverError::InvalidLength(
                "the length of t should be h".to_string(),
            ));
        }

        if ms.len() != h {
            return Err(ReceiverError::InvalidLength(
                "the length of M should be h".to_string(),
            ));
        }

        let comp_alpha_vec = alpha.to_msb0_vec()[0..h].to_vec();

        // Setp 5 in Figure 6.
        let k: Vec<Block> = ms
            .into_iter()
            .zip(ts)
            .zip(comp_alpha_vec.iter())
            .enumerate()
            .map(|(i, (([m0, m1], t), b))| {
                let tweak: Block = bytemuck::cast([i, self.state.exec_counter]);
                if !b {
                    // H(t, i|ell) ^ M0
                    FIXED_KEY_AES.tccr(tweak, t) ^ m0
                } else {
                    // H(t, i|ell) ^ M1
                    FIXED_KEY_AES.tccr(tweak, t) ^ m1
                }
            })
            .collect();

        // Reconstruct GGM tree except `ws[alpha]`.
        let ggm_tree = GgmTree::new(h);
        self.state.ws = vec![Block::ZERO; 1 << h];
        ggm_tree.reconstruct(&mut self.state.ws, &k, &comp_alpha_vec);

        // Set `ws[alpha]`.
        self.state.ws[alpha] = self.state.ws.iter().fold(sum, |acc, &x| acc ^ x);

        Ok(())
    }

    ///
    pub fn check_pre(
        &mut self,
        check: CheckReceiverFromCOT,
    ) -> Result<CheckFromReceiver, ReceiverError> {
        todo!()
    }

    ///
    pub fn check(&mut self, check: CheckFromSender) -> Result<(), ReceiverError> {
        todo!()
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
    #[allow(missing_docs)]
    pub struct Initialized {}

    impl State for Initialized {}

    opaque_debug::implement!(Initialized);

    /// The receiver's state after the setup phase.
    ///
    /// In this state the receiver performs COT extension and outputs random choice bits (potentially multiple times).
    pub struct Extension {
        /// Receiver's output blocks.
        pub ws: Vec<Block>,

        /// Current COT counter
        pub cot_counter: usize,
        /// Current execution counter
        pub exec_counter: usize,

        /// A PRG to generate random strings.
        pub prg: Prg,
    }

    impl State for Extension {}

    opaque_debug::implement!(Extension);
}
