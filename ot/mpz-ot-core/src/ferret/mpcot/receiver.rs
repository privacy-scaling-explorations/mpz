//! MPCOT receiver for general indices.

use crate::ferret::{
    mpcot::error::ReceiverError,
    utils::{hash_to_index, pos, Bucket, CuckooHash},
    CUCKOO_HASH_NUM,
};
use mpz_core::{aes::AesEncryptor, prg::Prg, Block};
use rand_core::SeedableRng;

use super::msgs::HashSeed;

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
    ///
    /// See step 1 in Figure 6.
    ///
    /// # Argument
    ///
    /// * `hash_seed` - Random seed to generate hashes, will be sent to the sender.
    pub fn setup(self, hash_seed: Block) -> (Receiver<state::Extension>, HashSeed) {
        let mut prg = Prg::from_seed(hash_seed);
        let hashes = std::array::from_fn(|_| AesEncryptor::new(prg.random_block()));
        let recv = Receiver {
            state: state::Extension {
                counter: 0,
                m: 0,
                hashes,
                buckets: Vec::default(),
            },
        };

        let seed = HashSeed { seed: hash_seed };

        (recv, seed)
    }
}

impl Receiver<state::Extension> {
    /// Performs the hash procedure in MPCOT extension.
    /// Outputs the length of each bucket plus 1.
    ///
    /// See Step 1 to Step 4 in Figure 7.
    ///
    /// # Arguments
    ///
    /// * `alphas` - The queried indices.
    /// * `n` - The total number of indices.
    pub fn extend_hash(
        &mut self,
        alphas: &[u32],
        n: u32,
    ) -> Result<Vec<(usize, u32)>, ReceiverError> {
        if alphas.len() as u32 > n {
            return Err(ReceiverError::InvalidInput(
                "length of alphas should not exceed n".to_string(),
            ));
        }

        let mut cuckoo = CuckooHash::new(&self.state.hashes);

        // Inserts all the alpha's.
        cuckoo.insert(alphas)?;

        self.state.m = cuckoo.m;

        // Removes the hash indices in the Cuckoo hash table.
        let table: Vec<Option<u32>> = cuckoo
            .table
            .into_iter()
            .map(|item| {
                if let Some(x) = item {
                    Some(x.value)
                } else {
                    None
                }
            })
            .collect();

        let mut bucket = Bucket::new(&self.state.hashes, self.state.m);

        // Geneates the buckets.
        bucket.insert(n);

        // Generates queries for SPCOT.
        // See Step 4 in Figure 7.
        let mut p = vec![];
        for (value, bin) in table.iter().zip(bucket.buckets.iter()) {
            if let Some(x) = value {
                let pos = pos(bin, *x)?;
                if let Some(power) = (bin.len() + 1).checked_next_power_of_two() {
                    p.push((power.ilog2() as usize, pos as u32));
                } else {
                    return Err(ReceiverError::InvalidBucketSize(
                        "The next power of 2 of the bucket size exceeds the MAX number".to_string(),
                    ));
                }
            } else {
                if let Some(power) = (bin.len() + 1).checked_next_power_of_two() {
                    p.push((power.ilog2() as usize, (bin.len() + 1) as u32));
                } else {
                    return Err(ReceiverError::InvalidBucketSize(
                        "The next power of 2 of the bucket size exceeds the MAX number".to_string(),
                    ));
                }
            }
        }

        // Stores the buckets.
        self.state.buckets = bucket.buckets;

        Ok(p)
    }

    /// Performs MPCOT extension.
    ///
    /// See Step 5 in Figure 7.
    ///
    /// # Arguments
    ///
    /// * `rt` - The vector received from SPCOT protocol on multiple queries.
    /// * `n` - The total nunber of indices.
    pub fn extend(&mut self, rt: &[Vec<Block>], n: u32) -> Result<Vec<Block>, ReceiverError> {
        if rt.len() != self.state.m {
            return Err(ReceiverError::InvalidInput(
                "the length rt should be m".to_string(),
            ));
        }

        if rt
            .iter()
            .zip(self.state.buckets.iter())
            .any(|(s, b)| s.len() != b.len() + 1)
        {
            return Err(ReceiverError::InvalidInput(
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

                s ^= rt[bucket_index][pos];
            }
            *x = s;
        }
        self.state.counter += 1;

        // Clears the buckets.
        self.state.buckets.clear();

        Ok(res)
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
    pub struct Initialized {}

    impl State for Initialized {}

    opaque_debug::implement!(Initialized);

    /// The receiver's state after the setup phase.
    ///
    /// In this state the receiver performs MPCOT extension (potentially multiple times).
    pub struct Extension {
        /// Current MPCOT counter
        pub(super) counter: usize,
        /// Current length of Cuckoo hash table, will possibly be changed in each extension.
        pub(super) m: usize,
        /// The hashes to generate Cuckoo hash table.
        pub(super) hashes: [AesEncryptor; CUCKOO_HASH_NUM],
        /// The buckets contains all the hash values, will be cleared after each extension.
        pub(super) buckets: Vec<Vec<u32>>,
    }

    impl State for Extension {}

    opaque_debug::implement!(Extension);
}
