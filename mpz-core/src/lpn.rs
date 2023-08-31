//! Implement LPN with local linear code.
//! More especifically, a local linear code is a random boolean matrix with at most D non-zero values in each row.

use crate::{prp::Prp, Block};
use rayon::prelude::*;
/// A struct related to LPN.
/// The `seed` defines a sparse binary matrix `A` with at most `D` non-zero values in each row.\
/// `A` - is a binary matrix with `k` columns and `n` rows. The concrete number of `n` is determined by the input length. `A` will be generated on-the-fly.\
/// `x` - is a `F_{2^128}` vector with length `k`.\
/// `e` - is a `F_{2^128}` vector with length `n`.\
/// Given a vector `x` and `e`, compute `y = Ax + e`.\
/// Note that in the standard LPN problem, `x` is a binary vector, `e` is a sparse binary vector. The way we difined here is a more generic way in term of computing `y`.
pub struct Lpn<const D: usize> {
    // The seed to generate the random sparse matrix A.
    seed: Block,

    // The length of the secret, i.e., x.
    k: u32,

    // A mask to optimize reduction operation.
    mask: u32,
}

impl<const D: usize> Lpn<D> {
    /// New an LPN instance
    pub fn new(seed: Block, k: u32) -> Self {
        let mut mask = 1;
        while mask < k {
            mask <<= 1;
            mask |= 0x1;
        }
        Self { seed, k, mask }
    }

    // Compute 4 rows as a batch, this is for the `compute_naive` function.
    #[inline]
    fn compute_four_rows_non_indep(&self, y: &mut [Block], x: &[Block], pos: usize, prp: &Prp) {
        let mut cnt = 0u64;
        let index = [0; D].map(|_| {
            let i: u64 = cnt;
            cnt += 1;
            Block::from(bytemuck::cast::<_, [u8; 16]>([pos as u64, i]))
        });

        let mut index: [Block; D] = prp.permute_many_blocks(index);
        let index: &mut [u32] = bytemuck::cast_slice_mut::<_, u32>(&mut index);

        for (i, y) in y[pos..].iter_mut().enumerate().take(4) {
            for ind in index[i * D..(i + 1) * D].iter_mut() {
                *ind &= self.mask;
                *ind = if *ind >= self.k { *ind - self.k } else { *ind };

                *y ^= x[*ind as usize];
            }
        }
    }

    // Compute 4 rows as a batch, this is for the `compute` function.
    #[inline]
    fn compute_four_rows_indep(&self, y: &mut [Block], x: &[Block], pos: usize, prp: &Prp) {
        let mut cnt = 0u64;
        let index = [0; D].map(|_| {
            let i = cnt;
            cnt += 1;
            Block::from(bytemuck::cast::<_, [u8; 16]>([pos as u64, i]))
        });

        let mut index = prp.permute_many_blocks(index);
        let index = bytemuck::cast_slice_mut::<_, u32>(&mut index);

        for (i, y) in y.iter_mut().enumerate().take(4) {
            for ind in index[i * D..(i + 1) * D].iter_mut() {
                *ind &= self.mask;
                *ind = if *ind >= self.k { *ind - self.k } else { *ind };

                *y ^= x[*ind as usize];
            }
        }
    }

    // Compute one row.
    #[inline]
    fn compute_one_row(&self, y: &mut [Block], x: &[Block], pos: usize, prp: &Prp) {
        let block_size = (D + 4 - 1) / 4;
        let mut index = (0..block_size)
            .map(|i| Block::from(bytemuck::cast::<_, [u8; 16]>([pos as u64, i as u64])))
            .collect::<Vec<Block>>();
        prp.permute_block_slice(&mut index);
        let index = bytemuck::cast_slice_mut::<_, u32>(&mut index);

        for ind in index.iter_mut().take(D) {
            *ind &= self.mask;
            *ind = if *ind >= self.k { *ind - self.k } else { *ind };
            y[pos] ^= x[*ind as usize];
        }
    }

    /// Compute `Ax + e` in a naive way.\
    /// Input: `x` with length `k`.\
    /// Input: `y` with length `n`, this is actually `e` in LPN.\
    /// Output: `y = Ax + y`.\
    /// Use `compute` function for optimized performance, but will keep it probably for test.
    pub fn compute_naive(&self, y: &mut [Block], x: &[Block]) {
        assert_eq!(x.len() as u32, self.k);
        assert!(x.len() >= D);
        let prp = Prp::new(self.seed);
        let batch_size = y.len() / 4;

        for i in 0..batch_size {
            self.compute_four_rows_non_indep(y, x, i * 4, &prp);
        }

        for i in batch_size * 4..y.len() {
            self.compute_one_row(y, x, i, &prp);
        }
    }

    /// Compute `Ax + e` with multiple threads.\
    /// Input: `x` with length `k`.\
    /// Input: `y` with length `n`, this is actually `e` in LPN.\
    /// Output: `y = Ax + y`.\
    pub fn compute(&self, y: &mut [Block], x: &[Block]) {
        assert_eq!(x.len() as u32, self.k);
        assert!(x.len() >= D);
        let prp = Prp::new(self.seed);
        let size = y.len() - (y.len() % 4);

        y.par_chunks_exact_mut(4).enumerate().for_each(|(i, y)| {
            self.compute_four_rows_indep(y, x, i * 4, &prp);
        });

        for i in size..y.len() {
            self.compute_one_row(y, x, i, &prp);
        }
    }
}

#[test]
fn lpn_test() {
    use crate::prg::Prg;

    let k = 20;
    let n = 200;
    let lpn = Lpn::<10>::new(Block::ZERO, k);
    let mut x = vec![Block::ONES; k as usize];
    let mut y = vec![Block::ONES; n];
    let mut prg = Prg::new();
    prg.random_blocks(&mut x);
    prg.random_blocks(&mut y);
    let mut z = y.clone();

    lpn.compute_naive(&mut y, &x);
    lpn.compute(&mut z, &x);

    assert_eq!(y, z);
}
