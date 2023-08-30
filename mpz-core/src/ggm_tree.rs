//! Implement GGM tree for OT.
//! Implementation of GGM based on the procedure explained in the write-up
//! (<https://eprint.iacr.org/2020/925.pdf>, Page 14)

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

    /// Input: `seed`: a seed.
    /// Output: `tree`: a GGM (binary tree) `tree`, with size `2^{depth-1}`
    /// Output: `k0`: XORs of all the left-node values in each level, with size `depth-1`.
    /// Output: `k1`: XORs of all the right-node values in each level, with size `depth-1`.
    /// This implementation is adapted from EMP Toolkit.
    pub fn gen(&self, seed: Block, tree: &mut [Block], k0: &mut [Block], k1: &mut [Block]) {
        assert!(tree.len() == 1 << (self.depth - 1));
        assert!(k0.len() == self.depth - 1);
        assert!(k1.len() == self.depth - 1);
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
                self.tkprp.expand_4to8(&mut buf, &tree[i..]);
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

#[test]
fn ggm_test() {
    let depth = 3;
    let mut tree = vec![Block::ZERO; 1 << (depth - 1)];
    let mut k0 = vec![Block::ZERO; depth - 1];
    let mut k1 = vec![Block::ZERO; depth - 1];

    let ggm = GgmTree::new(depth);

    ggm.gen(Block::ZERO, &mut tree, &mut k0, &mut k1);

    // Test vectors are from EMP Toolkit.
    assert_eq!(
        tree,
        [
            Block::from(0x92A6DDEAA3E99F9BECB268BD9EF67C91_u128.to_le_bytes()),
            Block::from(0x9E7E9C02ED1E62385EE8A9EDDC63A2B5_u128.to_le_bytes()),
            Block::from(0xBD4B85E90AACBD106694537DB6251264_u128.to_le_bytes()),
            Block::from(0x230485DC4360014833E07D8D914411A2_u128.to_le_bytes()),
        ]
    );

    assert_eq!(
        k0,
        [
            Block::from(0x2E2B34CA59FA4C883B2C8AEFD44BE966_u128.to_le_bytes()),
            Block::from(0x2FED5803A945228B8A263BC028D36EF5_u128.to_le_bytes()),
        ]
    );

    assert_eq!(
        k1,
        [
            Block::from(0x7E46C568D1CD4972BB1A61F95DD80EDC_u128.to_le_bytes()),
            Block::from(0xBD7A19DEAE7E63706D08D4604D27B317_u128.to_le_bytes()),
        ]
    );
}
