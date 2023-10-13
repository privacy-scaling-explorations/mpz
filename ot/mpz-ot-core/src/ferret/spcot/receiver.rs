//! SPCOT receiver
use crate::ferret::{spcot::error::ReceiverError, CSP};
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
        extend: ExtendReceiverFromCOT,
    ) -> Result<MaskBits, ReceiverError> {
        todo!()
    }

    ///
    pub fn extend(&mut self, h: usize, extend: ExtendFromSender) -> Result<(), ReceiverError> {
        todo!()
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
