//! MPCOT receiver for regular indices. Regular indices means the indices are evenly distributed.

use mpz_core::Block;

use crate::ferret::mpcot::error::ReceiverError;

/// MPCOT receiver.
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
    pub fn setup(self) -> Receiver<state::Extension> {
        Receiver {
            state: state::Extension {
                counter: 0,
                queries_length: Vec::default(),
                queries_depth: Vec::default(),
            },
        }
    }
}

impl Receiver<state::Extension> {
    /// Performs the prepare procedure in MPCOT extension.
    /// Outputs the indices for SPCOT.
    ///
    /// # Arguments.
    ///
    /// * `alphas` - The queried indices.
    /// * `n` - The total number of indices.
    pub fn extend_pre(
        &mut self,
        alphas: &[u32],
        n: u32,
    ) -> Result<Vec<(usize, u32)>, ReceiverError> {
        let t = alphas.len() as u32;
        if t > n {
            return Err(ReceiverError::InvalidInput(
                "the length of alpha should not exceed n".to_string(),
            ));
        }

        // The range of each interval.
        let k = (n + t - 1) / t;

        self.state.queries_length = if n % t == 0 {
            vec![k as usize; t as usize]
        } else {
            let mut tmp = vec![k as usize; (t - 1) as usize];
            tmp.push((n % k) as usize);
            if tmp.iter().sum::<usize>() != n as usize {
                return Err(ReceiverError::InvalidInput(
                    "the input parameters (t,n) are not regular".to_string(),
                ));
            } else {
                tmp
            }
        };

        for len in self.state.queries_length.iter() {
            // pad `len` to power of 2.
            let power = len.checked_next_power_of_two().expect("len should be less than usize::MAX / 2 - 1");

            self.state.queries_depth.push(power.ilog2() as usize);
        }

        if !alphas
            .iter()
            .enumerate()
            .all(|(i, &alpha)| (i as u32) * k <= alpha && alpha < ((i + 1) as u32) * k)
        {
            return Err(ReceiverError::InvalidInput(
                "the input position is not regular".to_string(),
            ));
        }

        let res: Vec<(usize, u32)> = self
            .state
            .queries_depth
            .iter()
            .zip(alphas.iter())
            .map(|(&d, &alpha)| (d, alpha % k))
            .collect();

        Ok(res)
    }

    /// Performs MPCOT extension.
    ///
    /// # Arguments.
    ///
    /// * `rt` - The vector received from SPCOT protocol on multiple queries.
    /// * `n` - The total number of indices.
    pub fn extend(&mut self, rt: &[Vec<Block>], n: u32) -> Result<Vec<Block>, ReceiverError> {
        if rt
            .iter()
            .zip(self.state.queries_depth.iter())
            .any(|(blks, m)| blks.len() != 1 << m)
        {
            return Err(ReceiverError::InvalidInput(
                "the length of rt[i] should be 2^self.state.queries_depth[i]".to_string(),
            ));
        }

        let mut res: Vec<Block> = Vec::with_capacity(n as usize);

        for (blks, pos) in rt.iter().zip(self.state.queries_length.iter()) {
            res.extend(&blks[..*pos]);
        }

        self.state.counter += 1;
        self.state.queries_depth.clear();
        self.state.queries_length.clear();

        Ok(res)
    }
}
/// The receiver's state.
pub mod state {

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

    /// The receiver's state after the setup phase.
    ///
    /// In this state the receiver performs MPCOT extension (potentially multiple times).
    pub struct Extension {
        /// Current MPCOT counter
        pub(super) counter: usize,

        /// Current queries length.
        pub(super) queries_length: Vec<usize>,

        /// The depth of queries.
        pub(super) queries_depth: Vec<usize>,
    }

    impl State for Extension {}

    opaque_debug::implement!(Extension);
}
