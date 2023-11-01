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
#[derive(Copy, Clone, Debug)]
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
    /// * `alphas` - A sorted and non-repeated u32 vector.
    pub fn insert(&mut self, alphas: &[u32]) -> Result<(), CuckooHashError> {
        // Always sets m = 1.5 * t. t is the length of `alphas`.
        self.m = compute_table_length(alphas.len() as u32);

        // Allocates table.
        self.table = vec![None; self.m];

        // Insert each alpha.
        for &value in alphas {
            self.hash(value)?
        }
        Ok(())
    }

    // Hash the element to a position with the current hash function.
    // The only requirement of hash is to ensure random output.
    #[inline]
    fn hash(&mut self, value: u32) -> Result<(), CuckooHashError> {
        // item consists of the value and hash index, starting from 0.
        let mut item = Item {
            value,
            hash_index: 0,
        };

        for _ in 0..CUCKOO_TRIAL_NUM {
            // Computes the position of the value.
            let pos = hash_to_index(&self.hashes[item.hash_index], self.m, item.value);

            // Insert the value to position `pos`.
            let opt_item = self.table[pos].replace(item);

            // If position `pos` is not empty before the above insertion, iteratively insert the obtained value.
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
    pub buckets: Vec<Vec<u32>>,
}

impl<'a> Bucket<'a> {
    /// Creates a new instance.
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
    #[inline(always)]
    pub fn insert(&mut self, n: u32) {
        self.buckets = vec![Vec::default(); self.m];
        for i in 0..n {
            for hash in self.hashes {
                let pos = hash_to_index(&hash, self.m, i);
                self.buckets[pos].push(i);
            }
        }

        // Sorts the value in each bucket.
        self.buckets.iter_mut().for_each(|v| v.sort());
    }
}

#[inline(always)]
// Always sets m = 1.5 * t. t is the length of `alphas`.
pub(crate) fn compute_table_length(t: u32) -> usize {
    (1.5 * (t as f32)).ceil() as usize
}

#[inline(always)]
pub(crate)fn hash_to_index(hash: &AesEncryptor, range: usize, value: u32) -> usize {
    let mut blk = bytemuck::cast::<_, Block>(value as u128);
    blk = hash.encrypt_block(blk);
    let res = u128::from_le_bytes(blk.to_bytes());
    (res as usize) % range
}

// Finds the position of the value in each Bucket.
#[inline(always)]
pub(crate) fn pos(bucket: &[u32], value: u32) -> Result<usize, BucketError> {
    let pos = bucket.iter().position(|&x| value == x);
    pos.ok_or(BucketError::NotInBucket("not in the bucket".to_string()))
}

#[cfg(test)]
mod tests {
    use crate::ferret::utils::pos;

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

        cuckoo
            .table
            .iter()
            .zip(bucket.buckets.iter())
            .all(|(value, bin)| match value {
                Some(x) => bin.contains(&(*x).value),
                None => true,
            });

        let _: Vec<usize> = cuckoo
            .table
            .iter()
            .zip(bucket.buckets.iter())
            .map(|(value, bin)| {
                if let Some(x) = value {
                    pos(bin, x.value).unwrap()
                } else {
                    bin.len() + 1
                }
            })
            .collect();
    }
}
