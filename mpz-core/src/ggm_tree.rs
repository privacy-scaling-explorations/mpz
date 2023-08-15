//! Implement GGM tree for OT.

use crate::{tkprp::TwoKeyPrp, Block};

/// Struct of GGM
pub struct GgmTree {
    tkprp: TwoKeyPrp,
    depth: usize,
}

impl GgmTree {
    ///New GgmTree instance.
    #[inline(always)]
    pub fn new(depth: usize) -> Self {
        let tkprp = TwoKeyPrp::new([Block::ZERO, Block::from(1u128.to_le_bytes())]);
        Self { tkprp, depth }
    }

    /// Generate tree.\
    /// Take as input a `seed`. \
    /// The generated tree is stored in `tree`. \
    /// Output two block slices used for OT.
    pub fn gen(&self, seed: Block, tree: &mut [Block], k0: &mut [Block], k1: &mut [Block]) {
        let mut buf = vec![Block::ZERO; 8];
        self.tkprp.expand_1to2(tree, seed);
        k0[0] = tree[0];
        k1[0] = tree[1];

        self.tkprp.expand_2to4(&mut buf, tree);
        k0[1] = buf[0] ^ buf[2];
        k1[1] = buf[1] ^ buf[3];
        tree[0..4].copy_from_slice(&buf[0..4]);

        for h in 2..self.depth - 1 {
            k0[h] = Block::ZERO;
            k1[h] = Block::ZERO;
            let sz = 1 << h;
            for i in (0..=sz - 4).rev().step_by(4) {
                self.tkprp.expand_4to8(&mut buf, &mut tree[i..]);
                k0[h] ^= buf[0];
                k0[h] ^= buf[2];
                k0[h] ^= buf[4];
                k0[h] ^= buf[6];
                k1[h] ^= buf[1];
                k1[h] ^= buf[3];
                k1[h] ^= buf[5];
                k1[h] ^= buf[7];

                tree[2 * i..2 * i + 8].copy_from_slice(&buf);
            }
        }
    }
}
