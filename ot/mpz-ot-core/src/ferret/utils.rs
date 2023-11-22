//! Utils for the implementation of Ferret.

use mpz_core::{aes::AesEncryptor, Block};

use super::{CUCKOO_HASH_NUM, CUCKOO_TRIAL_NUM};

/// Errors that can occur when inserting Cuckoo hash.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum CuckooHashError {
    #[error("invalid Cuckoo hash state: expected {0}")]
    CuckooHashLoop(String),
}

/// Errors that can occur when handling Buckets.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum BucketError {
    #[error("invalid bucket state: expected {0}")]
    NotInBucket(String),
    #[error("invalid bucket index: expected {0}")]
    OutOfRange(String),
}

/// Item in Cuckoo hash table.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Item {
    /// Value in the table.
    pub value: u32,
    /// The hash index during the insertion.
    pub hash_index: usize,
}

/// Implementation of Cuckoo hash. See [here](https://eprint.iacr.org/2019/1084.pdf) for reference.
pub struct CuckooHash<'a> {
    /// The table contains the elements.
    pub table: Vec<Option<Item>>,
    /// The length of the table.
    pub m: usize,
    // The hash functions.
    hashes: &'a [AesEncryptor; CUCKOO_HASH_NUM],
}

impl<'a> CuckooHash<'a> {
    /// Creates a new instance.
    #[inline]
    pub fn new(hashes: &'a [AesEncryptor; CUCKOO_HASH_NUM]) -> Self {
        let table = Vec::default();

        Self {
            table,
            m: 0,
            hashes,
        }
    }

    /// Insert elements into a Cuckoo hash table.
    ///
    /// * Argument
    ///
    /// * `alphas` - A u32 vector being inserted.
    #[inline]
    pub fn insert(&mut self, alphas: &[u32]) -> Result<(), CuckooHashError> {
        // Always sets m = 1.5 * t. t is the length of `alphas`.
        self.m = compute_table_length(alphas.len() as u32);

        // Allocates table.
        self.table = vec![None; self.m];

        // Inserts each alpha.
        for &value in alphas {
            self.hash(value)?
        }
        Ok(())
    }

    // Hash an element to a position with the current hash function.
    #[inline]
    fn hash(&mut self, value: u32) -> Result<(), CuckooHashError> {
        // The item consists of the value and hash index, starting from 0.
        let mut item = Item {
            value,
            hash_index: 0,
        };

        for _ in 0..CUCKOO_TRIAL_NUM {
            // Computes the position of the value.
            let pos = hash_to_index(&self.hashes[item.hash_index], self.m, item.value);

            // Inserts the value to position `pos`.
            let opt_item = self.table[pos].replace(item);

            // If position `pos` is not empty before the above insertion, iteratively inserts the obtained value.
            if let Some(x) = opt_item {
                item = x;
                item.hash_index = (item.hash_index + 1) % CUCKOO_HASH_NUM;
            } else {
                // If no value assigned to position `pos`, end the process.
                return Ok(());
            }
        }
        Err(CuckooHashError::CuckooHashLoop(
            "insertion loops".to_string(),
        ))
    }
}

/// Implementation of Bucket. See step 3 in Figure 7
pub struct Bucket<'a> {
    // The hash functions.
    hashes: &'a [AesEncryptor; CUCKOO_HASH_NUM],
    // The number of buckets.
    m: usize,
    /// The buckets contain all the elements.
    pub buckets: Vec<Vec<Item>>,
}

impl<'a> Bucket<'a> {
    /// Creates a new instance.
    #[inline]
    pub fn new(hashes: &'a [AesEncryptor; CUCKOO_HASH_NUM], m: usize) -> Self {
        Self {
            hashes,
            m,
            buckets: Vec::default(),
        }
    }

    /// Inserts the input vector [0..n-1] into buckets.
    ///
    /// # Argument
    ///
    /// * `n` - The length of the vector [0..n-1].
    #[inline]
    pub fn insert(&mut self, n: u32) {
        self.buckets = vec![Vec::default(); self.m];
        for i in 0..n {
            for (index, hash) in self.hashes.iter().enumerate() {
                let pos = hash_to_index(hash, self.m, i);
                self.buckets[pos].push(Item {
                    value: i,
                    hash_index: index,
                });
            }
        }
    }
}

// Always sets m = 1.5 * t. t is the length of `alphas`.
#[inline(always)]
pub(crate) fn compute_table_length(t: u32) -> usize {
    (1.5 * (t as f32)).ceil() as usize
}

// Hash the value into index using AES.
#[inline(always)]
pub(crate) fn hash_to_index(hash: &AesEncryptor, range: usize, value: u32) -> usize {
    let mut blk: Block = bytemuck::cast::<_, Block>(value as u128);
    blk = hash.encrypt_block(blk);
    let res = u128::from_le_bytes(blk.to_bytes());
    (res as usize) % range
}

// Finds the position of the item in each Bucket.
#[inline(always)]
pub(crate) fn find_pos(bucket: &[Item], item: &Item) -> Result<usize, BucketError> {
    let pos = bucket.iter().position(|&x| *item == x);
    pos.ok_or(BucketError::NotInBucket("not in the bucket".to_string()))
}

#[cfg(test)]
mod tests {
    use crate::ferret::utils::find_pos;

    use super::{Bucket, CuckooHash};
    use mpz_core::{aes::AesEncryptor, prg::Prg};

    #[test]
    fn cockoo_hash_bucket_test() {
        let mut prg = Prg::new();
        const NUM: usize = 50;
        let hashes = std::array::from_fn(|_| AesEncryptor::new(prg.random_block()));
        let mut cuckoo = CuckooHash::new(&hashes);
        let input: [u32; NUM] = std::array::from_fn(|i| i as u32);

        cuckoo.insert(&input).unwrap();

        let mut bucket = Bucket::new(&hashes, cuckoo.m);
        bucket.insert((2 * NUM) as u32);

        assert!(cuckoo
            .table
            .iter()
            .zip(bucket.buckets.iter())
            .all(|(value, bin)| match value {
                Some(x) => bin.contains(x),
                None => true,
            }));

        let _: Vec<usize> = cuckoo
            .table
            .iter()
            .zip(bucket.buckets.iter())
            .map(|(value, bin)| {
                if let Some(x) = value {
                    find_pos(bin, x).unwrap()
                } else {
                    bin.len() + 1
                }
            })
            .collect();
    }
}
