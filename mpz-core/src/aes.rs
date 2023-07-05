//! Fixed-key AES cipher

use aes::{cipher::generic_array::GenericArray, Aes128};
use cipher::{generic_array::functional::FunctionalSequence, BlockEncrypt, KeyInit};
use once_cell::sync::Lazy;

use crate::Block;

/// AES fixed key
pub const FIXED_KEY: [u8; 16] = [
    69, 42, 69, 42, 69, 42, 69, 42, 69, 42, 69, 42, 69, 42, 69, 42,
];

/// Fixed-key AES cipher
pub static FIXED_KEY_AES: Lazy<FixedKeyAes> = Lazy::new(|| FixedKeyAes {
    aes: Aes128::new_from_slice(&FIXED_KEY).unwrap(),
});

/// Fixed-key AES cipher
pub struct FixedKeyAes {
    aes: Aes128,
}

impl FixedKeyAes {
    /// Tweakable circular correlation-robust hash function instantiated
    /// using fixed-key AES.
    ///
    /// See <https://eprint.iacr.org/2019/074>
    ///
    /// `π(π(x) ⊕ i) ⊕ π(x)`
    #[inline]
    pub fn tccr(&self, tweak: Block, block: Block) -> Block {
        let tweak = GenericArray::from(tweak);

        let mut h1 = GenericArray::from(block);
        self.aes.encrypt_block(&mut h1);

        let mut h2 = h1.zip(tweak, |a, b| a ^ b);
        self.aes.encrypt_block(&mut h2);

        let out: [u8; 16] = h2.zip(h1, |a, b| a ^ b).into();

        Block::from(out)
    }
}
