//! MPCOT sender for general indices.

use crate::ferret::{
    mpcot::error::SenderError,
    cuckoo::{compute_table_length, find_pos, hash_to_index, Bucket, Item},
    CUCKOO_HASH_NUM,
};
use mpz_core::{aes::AesEncryptor, prg::Prg, Block};
use rand_core::SeedableRng;

use super::msgs::HashSeed;

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
    pub fn setup(self, delta: Block, hash_seed: HashSeed) -> Sender<state::Extension> {
        let HashSeed { seed: hash_seed } = hash_seed;
        let mut prg = Prg::from_seed(hash_seed);
        let hashes = std::array::from_fn(|_| AesEncryptor::new(prg.random_block()));
        Sender {
            state: state::Extension {
                delta,
                counter: 0,
                m: 0,
                hashes,
                buckets: Vec::default(),
                buckets_length: Vec::default(),
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
    pub fn extend_pre(&mut self, t: u32, n: u32) -> Result<Vec<usize>, SenderError> {
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

        // Computes `log(length + 1)` of each bucket.
        let mut bs = vec![];
        for bin in bucket.buckets.iter() {
            if let Some(power) = (bin.len() + 1).checked_next_power_of_two() {
                bs.push(power.ilog2() as usize);
                self.state.buckets_length.push(power);
            } else {
                return Err(SenderError::InvalidBucketSize(
                    "The next power of 2 of the bucket size exceeds the MAX number".to_string(),
                ));
            }
        }

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
            .zip(self.state.buckets_length.iter())
            .any(|(s, b)| s.len() != *b)
        {
            return Err(SenderError::InvalidInput(
                "the length of st[i] should be self.state.buckets_length[i]".to_string(),
            ));
        }

        let mut res = vec![Block::ZERO; n as usize];

        for (value, x) in res.iter_mut().enumerate() {
            for tau in 0..CUCKOO_HASH_NUM {
                // Computes the index of `value`.
                let bucket_index =
                    hash_to_index(&self.state.hashes[tau], self.state.m, value as u32);
                let pos = find_pos(
                    &self.state.buckets[bucket_index],
                    &Item {
                        value: value as u32,
                        hash_index: tau,
                    },
                )?;

                *x ^= st[bucket_index][pos];
            }
        }

        self.state.counter += 1;

        // Clears the buckets.
        self.state.buckets.clear();
        self.state.buckets_length.clear();

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
        pub(super) buckets: Vec<Vec<Item>>,
        /// The padded buckets length (power of 2).
        pub(super) buckets_length: Vec<usize>,
    }

    impl State for Extension {}

    opaque_debug::implement!(Extension);
}
