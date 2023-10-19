//! SPCOT receiver
use crate::ferret::{spcot::error::ReceiverError, CSP};
use itybity::ToBits;
use mpz_core::{aes::FIXED_KEY_AES, ggm_tree::GgmTree, hash::Hash, prg::Prg, utils::blake3, Block};
use rand_core::SeedableRng;

use super::msgs::{CheckFromReceiver, CheckFromSender, ExtendFromSender, MaskBits};

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
                chis: Vec::default(),
                cot_counter: 0,
                exec_counter: 0,
                prg: Prg::from_seed(seed),
            },
        }
    }
}

impl Receiver<state::Extension> {
    /// Performs the mask bit step in extension.
    ///
    /// See step 4 in Figure 6.
    ///
    /// # Arguments
    ///
    /// * `h` - The depth of the GGM tree.
    /// * `alpha` - The chosen position.
    /// * `rs` - The message from COT ideal functionality for the receiver. Only the random bits are used.
    pub fn extend_mask_bits(
        &mut self,
        h: usize,
        alpha: usize,
        rs: &[bool],
    ) -> Result<MaskBits, ReceiverError> {
        if alpha > (1 << h) {
            return Err(ReceiverError::InvalidInput(
                "the input pos should be no more than 2^h".to_string(),
            ));
        }

        if rs.len() != h {
            return Err(ReceiverError::InvalidLength(
                "the length of b should be h".to_string(),
            ));
        }

        // Step 4 in Figure 6
        let mut alpha_vec = alpha.to_msb0_vec();
        alpha_vec.drain(0..alpha_vec.len() - h);

        let bs: Vec<bool> = alpha_vec
            .iter()
            // Computes alpha_i XOR r_i XOR 1.
            .zip(rs.iter())
            .map(|(alpha, r)| alpha == r)
            .collect();

        Ok(MaskBits { bs })
    }

    /// Performs the GGM reconstruction step in extension. This function can be called multiple times before checking.
    ///
    /// See step 5 in Figure 6.
    ///
    /// # Arguments
    ///
    /// * `h` - The depth of the GGM tree.
    /// * `alpha` - The chosen position.
    /// * `ts` - The message from COT ideal functionality for the receiver. Only the chosen blocks are used.
    /// * `extendfr` - The message sent from the sender.
    pub fn extend(
        &mut self,
        h: usize,
        alpha: usize,
        ts: &[Block],
        extendfs: ExtendFromSender,
    ) -> Result<(), ReceiverError> {
        if alpha > (1 << h) {
            return Err(ReceiverError::InvalidInput(
                "the input pos should be no more than 2^h".to_string(),
            ));
        }

        let ExtendFromSender { ms, sum } = extendfs;
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

        let mut alpha_bar_vec = alpha.to_msb0_vec();
        alpha_bar_vec.drain(0..alpha_bar_vec.len() - h);

        alpha_bar_vec.iter_mut().for_each(|a| *a = !*a);

        // Setp 5 in Figure 6.
        let k: Vec<Block> = ms
            .into_iter()
            .zip(ts)
            .zip(alpha_bar_vec.iter())
            .enumerate()
            .map(|(i, (([m0, m1], &t), &b))| {
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

        // Reconstructs GGM tree except `ws[alpha]`.
        let ggm_tree = GgmTree::new(h);
        let mut tree = vec![Block::ZERO; 1 << h];
        ggm_tree.reconstruct(&mut tree, &k, &alpha_bar_vec);

        self.state.ws.extend_from_slice(&tree);

        // Sets `ws[alpha]`.
        self.state.ws[alpha] = self.state.ws.iter().fold(sum, |acc, &x| acc ^ x);

        Ok(())
    }

    /// Performs the decomposition and bit-mask steps in check.
    ///
    /// See step 7 in Figure 6.
    ///
    /// # Arguments
    ///
    /// * `h` - The depth of the GGM tree.
    /// * `alpha` - The chosen position.
    /// * `x_star` - The message from COT ideal functionality for the receiver. Only the random bits are used.
    pub fn check_pre(
        &mut self,
        h: usize,
        alpha: usize,
        x_star: &[bool],
    ) -> Result<CheckFromReceiver, ReceiverError> {
        if alpha > (1 << h) {
            return Err(ReceiverError::InvalidInput(
                "the input pos should be no more than 2^h".to_string(),
            ));
        }

        if x_star.len() != CSP {
            return Err(ReceiverError::InvalidLength(
                "the length of x* should be 128".to_string(),
            ));
        }

        let chis_seed = self.state.prg.random_block();
        self.state.chis = vec![Block::ZERO; 1 << h];
        Prg::from_seed(chis_seed).random_blocks(&mut self.state.chis);

        let x_prime: Vec<bool> = self.state.chis[alpha]
            .to_lsb0_vec()
            .into_iter()
            .zip(x_star)
            .map(|(x, &x_star)| x != x_star)
            .collect();

        Ok(CheckFromReceiver { chis_seed, x_prime })
    }

    /// Performs the final consistency check.
    ///
    /// See step 9 in Figure 6.
    ///
    /// # Arguments
    ///
    /// * `z_star` - The message from COT ideal functionality for the receiver. Only the chosen blocks are used.
    /// * `check` - The hashed value sent from the Sender.
    pub fn check(
        &mut self,
        z_star: &[Block],
        check: CheckFromSender,
    ) -> Result<Vec<Block>, ReceiverError> {
        let CheckFromSender { hashed_v } = check;

        if z_star.len() != CSP {
            return Err(ReceiverError::InvalidLength(
                "the length of z* should be 128".to_string(),
            ));
        }

        // Computes the base X^i
        let base: Vec<Block> = (0..CSP).map(|x| bytemuck::cast((1_u128) << x)).collect();

        // Computes Z.
        let mut w = Block::inn_prdt_red(&z_star, &base);

        // Computes W.
        w ^= Block::inn_prdt_red(&self.state.chis, &self.state.ws);

        // Computes H'(W)
        let hashed_w = Hash::from(blake3(&w.to_bytes()));

        if hashed_v != hashed_w {
            return Err(ReceiverError::ConsistencyCheckFailed);
        }

        self.state.exec_counter += 1;
        self.state.cot_counter += self.state.ws.len();
        Ok(self.state.ws.clone())
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
        /// Receiver's random challenges chis.
        pub chis: Vec<Block>,

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
