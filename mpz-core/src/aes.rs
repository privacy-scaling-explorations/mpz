//! Fixed-key AES cipher

use aes::{cipher::generic_array::GenericArray, Aes128, Aes128Enc};
use cipher::{generic_array::functional::FunctionalSequence, BlockEncrypt, KeyInit};
use once_cell::sync::Lazy;

use crate::Block;

/// A fixed AES key (arbitrarily chosen).
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
    /// See <https://eprint.iacr.org/2019/074> (Section 7.4)
    ///
    /// `π(π(x) ⊕ i) ⊕ π(x)`, where `π` is instantiated using fixed-key AES.
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

    /// Correlation-robust hash function for 128-bit inputs
    /// (cf. <https://eprint.iacr.org/2019/074>, §7.2).
    /// The function computes `π(x) ⊕ x`.
    /// π(x) = AES(fixedkey,x)
    #[inline]
    pub fn cr(&self, block: Block) -> Block {
        let mut x = GenericArray::from(block);
        self.aes.encrypt_block(&mut x);
        Block::from(x) ^ block
    }

    /// Circular correlation-robust hash function
    /// (cf.<https://eprint.iacr.org/2019/074>, §7.3).
    ///
    /// The function computes `H(sigma(x))`, where `H` is a correlation-robust hash
    /// function and `sigma( x = x0 || x1 ) = x1 || (x0 xor x1)`.
    /// `x0` and `x1` are the lower and higher halves of `x`, respectively.
    #[inline]
    pub fn ccr(&self, block: Block) -> Block {
        self.cr(Block::sigma(block))
    }
}

/// A wrapper of aes, only for encryption.
#[derive(Clone)]
pub struct AesEncryptor(Aes128Enc);

impl AesEncryptor {
    /// Constant number of AES blocks, always set to 8.
    pub const AES_BLOCK_COUNT: usize = 8;

    /// Initiate an AesEncryptor instance with key.
    #[inline(always)]
    pub fn new(key: Block) -> Self {
        let _key: [u8; 16] = key.into();
        AesEncryptor(Aes128Enc::new_from_slice(&_key).unwrap())
    }

    /// Encrypt a block.
    #[inline(always)]
    pub fn encrypt_block(&self, blk: Block) -> Block {
        let mut ctxt = GenericArray::from(blk);
        self.0.encrypt_block(&mut ctxt);
        Block::from(ctxt)
    }

    /// Encrypt many blocks.
    #[inline(always)]
    pub fn encrypt_many_blocks<const N: usize>(&self, blks: [Block; N]) -> [Block; N] {
        let mut out = [Block::default(); N];
        let out_mut = Block::as_generic_array_slice_mut(out.as_mut_slice());

        let blks = Block::as_generic_array_slice(blks.as_slice());

        self.0
            .encrypt_blocks_b2b(blks, out_mut)
            .expect("block slices are equal length");

        out
    }

    /// Encrypt many blocks with many keys.
    ///
    /// Each batch of NM blocks are encrypted by a corresponding AES key.
    ///
    /// **Only the first NK * NM blocks of blks are handled, the rest are ignored.**
    ///
    /// # Arguments
    ///
    /// * `keys` - A slice of keys used to encrypt the blocks.
    /// * `blks` - A slice of blocks to be encrypted.
    ///
    /// # Panics
    ///
    /// * If the length of `blks` is less than `NM * NK`.
    #[inline(always)]
    pub fn para_encrypt<const NK: usize, const NM: usize>(keys: &[Self; NK], blks: &mut [Block]) {
        assert!(blks.len() >= NM * NK);
        let mut ctxt = [Block::default(); NM];
        keys.iter().enumerate().for_each(|(i, key)| {
            ctxt.copy_from_slice(&blks[i * NM..(i + 1) * NM]);
            blks[i * NM..(i + 1) * NM].copy_from_slice(&key.encrypt_many_blocks(ctxt))
        });
    }
}

#[test]
fn aes_test() {
    let aes = AesEncryptor::new(Block::default());
    let aes1 = AesEncryptor::new(Block::ONES);

    let mut blks = [Block::default(); 4];
    blks[1] = Block::ONES;
    blks[3] = Block::ONES;
    AesEncryptor::para_encrypt::<2, 2>(&[aes, aes1], &mut blks);
    assert_eq!(
        blks,
        [
            Block::from((0x2E2B34CA59FA4C883B2C8AEFD44BE966_u128).to_le_bytes()),
            Block::from((0x4E668D3ED24773FA0A5A85EAC98C5B3F_u128).to_le_bytes()),
            Block::from((0x2CC9BF3845486489CD5F7D878C25F6A1_u128).to_le_bytes()),
            Block::from((0x79B93A19527051B230CF80B27C21BFBC_u128).to_le_bytes())
        ]
    );
}
