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
    /// Output: `tree`: a GGM (binary tree) `tree`, with size `2^{depth}`
    /// Output: `k0`: XORs of all the left-node values in each level, with size `depth`.
    /// Output: `k1`: XORs of all the right-node values in each level, with size `depth`.
    /// This implementation is adapted from EMP Toolkit.
    pub fn gen(&self, seed: Block, tree: &mut [Block], k0: &mut [Block], k1: &mut [Block]) {
        assert!(tree.len() == 1 << (self.depth));
        assert!(k0.len() == self.depth);
        assert!(k1.len() == self.depth);
        let mut buf = vec![Block::ZERO; 8];
        self.tkprp.expand_1to2(tree, seed);
        k0[0] = tree[0];
        k1[0] = tree[1];

        self.tkprp.expand_2to4(&mut buf, tree);
        k0[1] = buf[0] ^ buf[2];
        k1[1] = buf[1] ^ buf[3];
        tree[0..4].copy_from_slice(&buf[0..4]);

        for h in 2..self.depth {
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

    /// reconstruct
    pub fn reconstruct(&self, alpha: &[bool], k: &[Block], tree: &mut [Block]) {
        let mut pos = 0;
        for i in 1..=self.depth {
            pos *= 2;
            tree[pos] = Block::ZERO;
            tree[pos + 1] = Block::ZERO;
            if !alpha[i - 1] {
                self.reconstruct_layer(i, false, pos, k[i - 1], tree);
                pos += 1;
            } else {
                self.reconstruct_layer(i, true, pos + 1, k[i - 1], tree);
            }
        }
    }

    // Handle each layer.
    fn reconstruct_layer(
        &self,
        depth: usize,
        left_or_right: bool,
        pos: usize,
        k: Block,
        tree: &mut [Block],
    ) {
        let sz = 1 << depth;
        let mut sum = Block::ZERO;
        let start = if left_or_right { 1 } else { 0 };

        for i in (start..sz).step_by(2) {
            sum ^= tree[i];
        }
        tree[pos] = sum ^ k;

        if depth == (self.depth) {
            return;
        }

        let mut buf = vec![Block::ZERO; 8];
        if sz == 2 {
            self.tkprp.expand_2to4(&mut buf, tree);
            tree[0..4].copy_from_slice(&buf[0..4]);
        } else {
            for i in (0..=sz - 4).rev().step_by(4) {
                self.tkprp.expand_4to8(&mut buf, &tree[i..]);
                tree[2 * i..2 * i + 8].copy_from_slice(&buf);
            }
        }
    }
}

#[test]
fn ggm_test() {
    use crate::ggm_tree::GgmTree;
    use crate::Block;

    let depth = 3;
    let mut tree = vec![Block::ZERO; 1 << depth];
    let mut k0 = vec![Block::ZERO; depth];
    let mut k1 = vec![Block::ZERO; depth];
    let mut k = vec![Block::ZERO; depth];
    let alpha = [false, true, false];
    let mut pos = 0;

    for i in 0..depth {
        pos <<= 1;
        if !alpha[i] {
            pos += 1;
        }
    }

    let ggm = GgmTree::new(depth);

    ggm.gen(Block::ZERO, &mut tree, &mut k0, &mut k1);

    for i in 0..depth {
        if alpha[i] {
            k[i] = k1[i];
        } else {
            k[i] = k0[i];
        }
    }

    let mut tree_reconstruct = vec![Block::ZERO; 1 << depth];
    ggm.reconstruct(&alpha, &k, &mut tree_reconstruct);

    assert_eq!(tree_reconstruct[pos], Block::ZERO);
    tree_reconstruct[pos] = tree[pos];
    assert_eq!(tree, tree_reconstruct);
}
