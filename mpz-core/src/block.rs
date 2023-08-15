use cipher::{consts::U16, generic_array::GenericArray};
use clmul::Clmul;
use core::ops::{BitAnd, BitAndAssign, BitXor, BitXorAssign};
use itybity::{BitIterable, BitLength, GetBit, Lsb0, Msb0};
use rand::{distributions::Standard, prelude::Distribution, CryptoRng, Rng};
use serde::{Deserialize, Serialize};
use std::convert::From;

/// A block of 128 bits
#[repr(transparent)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Block([u8; 16]);

impl Block {
    /// The length of a block in bytes
    pub const LEN: usize = 16;
    /// A zero block
    pub const ZERO: Self = Self([0; 16]);
    /// A block with all bits set to 1
    pub const ONES: Self = Self([0xff; 16]);
    /// A length 2 array of zero and one blocks
    pub const SELECT_MASK: [Self; 2] = [Self::ZERO, Self::ONES];

    /// Create a new block
    #[inline]
    pub fn new(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Returns the byte representation of the block
    #[inline]
    pub fn to_bytes(self) -> [u8; 16] {
        self.0
    }

    /// Generate a random block using the provided RNG
    #[inline]
    pub fn random<R: Rng + CryptoRng + ?Sized>(rng: &mut R) -> Self {
        Self::new(rng.gen())
    }

    /// Generate a random array of blocks using the provided RNG
    #[inline]
    pub fn random_array<const N: usize, R: Rng + CryptoRng>(rng: &mut R) -> [Self; N] {
        std::array::from_fn(|_| rng.gen::<[u8; 16]>().into())
    }

    /// Generate a random vector of blocks using the provided RNG
    #[inline]
    pub fn random_vec<R: Rng + CryptoRng + ?Sized>(rng: &mut R, n: usize) -> Vec<Self> {
        (0..n).map(|_| rng.gen::<[u8; 16]>().into()).collect()
    }

    /// Carry-less multiplication of two blocks, without the reduction step.
    #[inline]
    pub fn clmul(self, other: Self) -> (Self, Self) {
        let (a, b) = Clmul::new(&self.0).clmul(Clmul::new(&other.0));
        (Self::new(a.into()), Self::new(b.into()))
    }

    #[inline]
    /// Performs reduction
    pub fn reduce(x: Self, y: Self) -> Self {
        let r = Clmul::reduce(Clmul::new(&x.0), Clmul::new(&y.0));
        Self::new(r.into())
    }

    /// The multiplication of two field elements.
    #[inline]
    pub fn gfmul(self, x: Self) -> Self {
        let (a, b) = self.clmul(x);
        Block::reduce(a, b)
    }

    /// Compute the inner product of two block vectors, without reducing the polynomial.
    #[inline]
    pub fn inn_prdt_no_red(a: &Vec<Block>, b: &Vec<Block>) -> (Block, Block) {
        assert_eq!(a.len(), b.len());
        a.iter()
            .zip(b.iter())
            .fold((Block::default(), Block::default()), |acc, (x, y)| {
                let t = x.clmul(*y);
                (t.0 ^ acc.0, t.1 ^ acc.1)
            })
    }

    /// Compute the inner product of two block vectors.
    #[inline]
    pub fn inn_prdt_red(a: &Vec<Block>, b: &Vec<Block>) -> Block {
        let (x, y) = Block::inn_prdt_no_red(a, b);
        Block::reduce(x, y)
    }

    /// Sets the least significant bit of the block
    #[inline]
    pub fn set_lsb(&mut self) {
        self.0[0] |= 1;
    }

    /// Returns the least significant bit of the block
    #[inline]
    pub fn lsb(&self) -> usize {
        ((self.0[0] & 1) == 1) as usize
    }

    ///Function ``sigma( x0 || x1 ) = (x0 xor x1) || x1``.
    #[inline(always)]
    pub fn sigma(a: Self) -> Self {
        let mut x: [u64; 2] = unsafe { std::mem::transmute(a) };
        x[0] = x[0] ^ x[1];
        Block(unsafe { std::mem::transmute(x) })
    }
}

/// A trait for converting a type to blocks
pub trait BlockSerialize {
    /// The block representation of the type
    type Serialized: std::fmt::Debug + Clone + Copy + Send + Sync + 'static;

    /// Convert the type to blocks
    fn to_blocks(self) -> Self::Serialized;

    /// Convert the blocks to the type
    fn from_blocks(blocks: Self::Serialized) -> Self;
}

impl BitLength for Block {
    const BITS: usize = 128;
}

impl GetBit<Lsb0> for Block {
    fn get_bit(&self, index: usize) -> bool {
        GetBit::<Lsb0>::get_bit(&self.0[index / 8], index % 8)
    }
}

impl GetBit<Msb0> for Block {
    fn get_bit(&self, index: usize) -> bool {
        GetBit::<Msb0>::get_bit(&self.0[15 - (index / 8)], index % 8)
    }
}

impl BitIterable for Block {}

impl From<[u8; 16]> for Block {
    #[inline]
    fn from(bytes: [u8; 16]) -> Self {
        Block::new(bytes)
    }
}

impl<'a> TryFrom<&'a [u8]> for Block {
    type Error = <[u8; 16] as TryFrom<&'a [u8]>>::Error;

    fn try_from(value: &'a [u8]) -> Result<Self, Self::Error> {
        <[u8; 16]>::try_from(value).map(Self::from)
    }
}

impl From<Block> for GenericArray<u8, U16> {
    #[inline]
    fn from(b: Block) -> Self {
        b.0.into()
    }
}

impl From<GenericArray<u8, U16>> for Block {
    #[inline]
    fn from(b: GenericArray<u8, U16>) -> Self {
        Block::new(b.into())
    }
}

impl From<Block> for [u8; 16] {
    #[inline]
    fn from(b: Block) -> Self {
        b.0
    }
}

impl BitXor for Block {
    type Output = Self;

    #[inline]
    fn bitxor(self, other: Self) -> Self::Output {
        Self(std::array::from_fn(|i| self.0[i] ^ other.0[i]))
    }
}

impl BitXorAssign for Block {
    #[inline(always)]
    fn bitxor_assign(&mut self, rhs: Self) {
        *self = *self ^ rhs;
    }
}

impl BitAnd for Block {
    type Output = Self;

    #[inline]
    fn bitand(self, other: Self) -> Self::Output {
        Self(std::array::from_fn(|i| self.0[i] & other.0[i]))
    }
}

impl BitAndAssign for Block {
    #[inline(always)]
    fn bitand_assign(&mut self, rhs: Self) {
        *self = *self & rhs
    }
}

impl Distribution<Block> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Block {
        Block::new(rng.gen())
    }
}

impl AsMut<[u8]> for Block {
    #[inline(always)]
    fn as_mut(&mut self) -> &mut [u8] {
        unsafe { &mut *(self as *mut Block as *mut [u8; 16]) }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_lsb() {
        let zero = [0; 16];
        let mut one = [0; 16];
        one[0] = 1;
        let mut three = [0; 16];
        three[0] = 3;

        let mut b = Block::new(zero);
        b.set_lsb();
        assert_eq!(Block::new(one), b);

        // no-op when the bit is already set
        let mut b = Block::new(three);
        b.set_lsb();
        assert_eq!(Block::new(three), b);
    }

    #[test]
    fn test_lsb() {
        let a = Block::new([0; 16]);
        assert_eq!(a.lsb(), 0);

        let mut one = [0; 16];
        one[0] = 1;

        let a = Block::new(one);
        assert_eq!(a.lsb(), 1);

        let mut two = [0; 16];
        two[0] = 2;

        let a = Block::new(two);
        assert_eq!(a.lsb(), 0);

        let mut three = [0; 16];
        three[0] = 3;

        let a = Block::new(three);
        assert_eq!(a.lsb(), 1);
    }

    #[test]
    fn inn_prdt_test() {
        use rand::{Rng, SeedableRng};
        use rand_chacha::ChaCha12Rng;
        let mut rng = ChaCha12Rng::from_entropy();

        const SIZE: usize = 1000;
        let mut a = Vec::new();
        let mut b = Vec::new();
        let mut c = (Block::default(), Block::default());
        let mut d = Block::default();
        for i in 0..SIZE {
            let r: [u8; 16] = rng.gen();
            a.push(Block::from(r));
            let r: [u8; 16] = rng.gen();
            b.push(Block::from(r));

            let z = a[i].clmul(b[i]);
            c.0 = c.0 ^ z.0;
            c.1 = c.1 ^ z.1;

            let x = a[i].gfmul(b[i]);
            d ^= x;
        }

        assert_eq!(c, Block::inn_prdt_no_red(&a, &b));
        assert_eq!(d, Block::inn_prdt_red(&a, &b));
    }
}
