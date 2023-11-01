//! MPCOT sender for general indices.

use crate::ferret::{
    mpcot::error::SenderError,
    utils::{compute_table_length, hash_to_index, pos, Bucket},
    CUCKOO_HASH_NUM,
};
use mpz_core::{aes::AesEncryptor, prg::Prg, Block};
use rand_core::SeedableRng;

/// MPCOT sender.
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

    /// Completes the setup phase of the protocol.
    ///
    /// # Arguments.
    ///
    /// * `delta` - The sender's global secret.
    /// * `hash_seed` - The seed for Cuckoo hash sent by the receiver.
    pub fn setup(self, delta: Block, hash_seed: Block) -> Sender<state::Extension> {
        let mut prg = Prg::from_seed(hash_seed);
        let hashes = std::array::from_fn(|_| AesEncryptor::new(prg.random_block()));
        Sender {
            state: state::Extension {
                delta,
                counter: 0,
                m: 0,
                hashes,
                buckets: Vec::default(),
            },
        }
    }
}

impl Sender<state::Extension> {
    /// Performs the hash procedure in MPCOT extension.
    /// Outputs the length of each bucket plus 1.
    ///
    /// See Step 1 to Step 4 in Figure 7.
    ///
    /// # Arguments
    ///
    /// * `t` - The number of queried indices.
    /// * `n` - The total number of indices.
    pub fn extend_hash(&mut self, t: u32, n: u32) -> Result<Vec<usize>, SenderError> {
        if t > n {
            return Err(SenderError::InvalidInput(
                "t should not exceed n".to_string(),
            ));
        }

        // Compute m = 1.5 * t.
        self.state.m = compute_table_length(t);

        let mut bucket = Bucket::new(&self.state.hashes, self.state.m);

        // Geneates the buckets.
        bucket.insert(n);

        // Computes `length + 1` of each bucket.
        let bs = bucket.buckets.iter().map(|bin| bin.len() + 1).collect();

        // Stores the buckets.
        self.state.buckets = bucket.buckets;

        Ok(bs)
    }

    /// Performs MPCOT extension.
    ///
    /// See Step 5 in Figure 7.
    ///
    /// # Arguments
    ///
    /// * `st` - The vector received from SPCOT protocol on multiple queries.
    /// * `n` - The total nunber of indices.
    pub fn extend(&mut self, st: &[Vec<Block>], n: u32) -> Result<Vec<Block>, SenderError> {
        if st.len() != self.state.m {
            return Err(SenderError::InvalidInput(
                "the length st should be m".to_string(),
            ));
        }

        if st
            .iter()
            .zip(self.state.buckets.iter())
            .any(|(s, b)| s.len() != b.len() + 1)
        {
            return Err(SenderError::InvalidInput(
                "the length of st[i] should be self.state.buckets.len() + 1".to_string(),
            ));
        }

        let mut res = Vec::<Block>::with_capacity(n as usize);

        for (value, x) in res.iter_mut().enumerate() {
            let mut s = Block::ZERO;
            for tau in 0..CUCKOO_HASH_NUM {
                // Computes the index of `value`.
                let bucket_index =
                    hash_to_index(&self.state.hashes[tau], self.state.m, value as u32);
                let pos = pos(&self.state.buckets[bucket_index], value as u32)?;

                s ^= st[bucket_index][pos];
            }
            *x = s;
        }

        self.state.counter += 1;

        // Clears the buckets.
        self.state.buckets.clear();

        Ok(res)
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

    /// The sender's state after the setup phase.
    ///
    /// In this state the sender performs MPCOT extension (potentially multiple times).
    pub struct Extension {
        /// Sender's global secret.
        #[allow(dead_code)]
        pub(super) delta: Block,
        /// Current MPCOT counter
        pub(super) counter: usize,

        /// Current length of Cuckoo hash table, will possibly be changed in each extension.
        pub(super) m: usize,
        /// The hashes to generate Cuckoo hash table.
        pub(super) hashes: [AesEncryptor; CUCKOO_HASH_NUM],
        /// The buckets contains all the hash values.
        pub(super) buckets: Vec<Vec<u32>>,
    }

    impl State for Extension {}

    opaque_debug::implement!(Extension);
}
