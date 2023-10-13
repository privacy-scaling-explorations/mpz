//! SPCOT receiver
use crate::ferret::{spcot::error::ReceiverError, CSP};
use mpz_core::{aes::FIXED_KEY_AES, ggm_tree::GgmTree, hash::Hash, prg::Prg, Block};

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
        /// Receiver's mask bits.
        pub maskbits: Vec<bool>,
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
