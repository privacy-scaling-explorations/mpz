//! SPCOT sender.
use crate::ferret::{spcot::error::SenderError, CSP};
use mpz_core::{aes::FIXED_KEY_AES, ggm_tree::GgmTree, hash::Hash, prg::Prg, Block};
use rand_core::SeedableRng;

use super::msgs::{
    CheckFromReceiver, CheckFromSender, CheckSenderFromCOT, ExtendFromSender, ExtendSenderFromCOT,
    MaskBits,
};

/// SPCOT sender.
#[derive(Debug, Default)]
pub struct Sender<T: state::State = state::Initialized> {
    state: T,
}

impl Sender {
    /// Creates a new Sender.
    pub fn new() -> Self {
        Sender {
            state: state::Initialized::default(),
        }
    }

    /// Complete the setup phase of the protocol.
    /// Step 1, Figure 6.
    ///
    /// # Arguments
    ///
    /// * `delta` - The sender's global secret.
    /// * `seed`  - The random seed to generate PRG.
    pub fn setup(self, delta: Block, seed: Block) -> Sender<state::Extension> {
        Sender {
            state: state::Extension {
                delta,
                vs: Vec::default(),
                cot_counter: 0,
                exec_counter: 0,
                prg: Prg::from_seed(seed),
            },
        }
    }
}

impl Sender<state::Extension> {
    /// Perform the SPCOT extension.
    ///
    /// See Step 1-5 in Figure 6.
    ///
    /// # Arguments
    ///
    /// * `h` - The depth of the GGM tree.
    /// * `qs`- The blocks received by calling the COT functionality.
    pub fn extend(
        &mut self,
        h: usize,
        extend: ExtendSenderFromCOT,
        mask: MaskBits,
    ) -> Result<ExtendFromSender, SenderError> {
        let ExtendSenderFromCOT { qs } = extend;
        let MaskBits { bs } = mask;

        if qs.len() != h {
            return Err(SenderError::InvalidLength(
                "the length of q should be 128".to_string(),
            ));
        }

        if bs.len() != h {
            return Err(SenderError::InvalidLength(
                "the length of b should h".to_string(),
            ));
        }

        // Step 3-4, Figure 6.

        // Generates a GGM tree with depth h and seed s.
        let s = self.state.prg.random_block();
        let ggm_tree = GgmTree::new(h);
        let mut k0 = vec![Block::ZERO; h];
        let mut k1 = vec![Block::ZERO; h];
        let mut tree = vec![Block::ZERO; 1 << h];
        ggm_tree.gen(s, &mut tree, &mut k0, &mut k1);

        // Computes M0 and M1.
        let mut ms: Vec<[Block; 2]> = qs
            .iter()
            .zip(bs.iter())
            .map(|(&q, &b)| {
                if b {
                    [q ^ self.state.delta, q]
                } else {
                    [q, q ^ self.state.delta]
                }
            })
            .collect();

        ms.iter_mut().enumerate().for_each(|(i, blks)| {
            let tweak: Block = bytemuck::cast([i, self.state.exec_counter]);
            let tweaks = [tweak, tweak];
            FIXED_KEY_AES.tccr_many(&tweaks, blks);
        });

        ms.iter_mut()
            .zip(k0.iter().zip(k1.iter()))
            .for_each(|([m0, m1], (k0, k1))| {
                *m0 ^= *k0;
                *m1 ^= *k1;
            });

        // Sets vs.
        self.state.vs = tree;

        // Computes the sum of the leaves and delta.
        let sum = self
            .state
            .vs
            .iter()
            .fold(self.state.delta, |acc, &x| acc ^ x);

        Ok(ExtendFromSender { ms, sum })
    }

    /// Performs the consistency check for the resulting COTs.
    ///
    /// See Step 6-9 in Figure 6.
    ///
    /// # Arguments
    ///
    /// * `h` - The depth of the GGM tree.
    /// * `checkfc` - The blocks received from the ideal functionality for the check.
    /// * `checkfr` - The blocks received from the receiver for the check.
    pub fn check(
        &mut self,
        h: usize,
        checkfc: CheckSenderFromCOT,
        checkfr: CheckFromReceiver,
    ) -> Result<CheckFromSender, SenderError> {
        let CheckSenderFromCOT { y_star } = checkfc;
        let CheckFromReceiver { chis, x_prime } = checkfr;

        if y_star.len() != CSP {
            return Err(SenderError::InvalidLength(
                "the length of y* should be 128".to_string(),
            ));
        }

        if chis.len() != 1 << h {
            return Err(SenderError::InvalidLength(
                "the length of chi should be 2^h ".to_string(),
            ));
        }

        if x_prime.len() != CSP {
            return Err(SenderError::InvalidLength(
                "the length of x' should be 128".to_string(),
            ));
        }

        // Step 8 in Figure 6.

        // Computes y = y^star + x' * Delta
        let y: Vec<Block> = y_star
            .iter()
            .zip(x_prime.iter())
            .map(|(&y, &x)| if x { y ^ self.state.delta } else { y })
            .collect();

        // Computes the base X^i
        let base: Vec<Block> = (0..CSP).map(|x| bytemuck::cast((1 << x) as u128)).collect();

        // Computes Y
        let mut v = Block::inn_prdt_red(&y, &base);

        // Computes V
        v ^= Block::inn_prdt_red(&chis, &self.state.vs);

        // Computes H'(V)
        let mut hasher = blake3::Hasher::new();
        hasher.update(&v.to_bytes());
        let hashed_v = Hash::from(*hasher.finalize().as_bytes());

        self.state.exec_counter += 1;
        self.state.cot_counter += self.state.vs.len();

        Ok(CheckFromSender { hashed_v })
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
    #[allow(missing_docs)]
    pub struct Initialized {}

    impl State for Initialized {}

    opaque_debug::implement!(Initialized);

    /// The sender's state after the setup phase.
    ///
    /// In this state the sender performs COT extension with random choice bits (potentially multiple times). Also in this state the sender responds to COT requests.
    pub struct Extension {
        /// Sender's global secret.
        pub delta: Block,
        /// Sender's output blocks.
        pub vs: Vec<Block>,

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
