//! This module implements the prime field of P256

use std::ops::{Add, Mul, Neg};

use ark_ff::{BigInt, BigInteger, Field as ArkField, FpConfig, MontBackend, One, PrimeField, Zero};
use ark_secp256r1::{fq::Fq, FqConfig};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use itybity::{BitLength, FromBitIterator, GetBit, Lsb0, Msb0};
use num_bigint::ToBigUint;
use rand::{distributions::Standard, prelude::Distribution};
use serde::{ser::SerializeSeq, Deserialize, Serialize, Serializer};

use mpz_core::{Block, BlockSerialize};

use super::Field;

/// A type for holding field elements of P256
///
/// Uses internally an MSB0 encoding
#[derive(Copy, Clone, PartialOrd, Ord, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "[u64; 4]")]
pub struct P256(#[serde(serialize_with = "serialize_p256")] pub(crate) Fq);

opaque_debug::implement!(P256);

impl P256 {
    /// Creates a new field element
    pub fn new(input: impl ToBigUint) -> Self {
        let input = input.to_biguint().expect("Unable to create field element");
        P256(Fq::from(input))
    }
}

fn serialize_p256<S>(value: &Fq, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let limbs = value.0 .0;

    let mut seq = serializer.serialize_seq(Some(4))?;
    for e in limbs {
        seq.serialize_element(&e)?;
    }

    seq.end()
}

impl From<[u64; 4]> for P256 {
    fn from(value: [u64; 4]) -> Self {
        P256(Fq::from(BigInt::new(value)))
    }
}

impl Distribution<P256> for Standard {
    fn sample<R: rand::Rng + ?Sized>(&self, rng: &mut R) -> P256 {
        P256(self.sample(rng))
    }
}

impl Add for P256 {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl Mul for P256 {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self::Output {
        Self(self.0 * rhs.0)
    }
}

impl Neg for P256 {
    type Output = Self;

    fn neg(self) -> Self::Output {
        Self(-self.0)
    }
}

impl Field for P256 {
    const BIT_SIZE: u32 = 256;

    fn zero() -> Self {
        P256(<Fq as Zero>::zero())
    }

    fn one() -> Self {
        P256(<Fq as One>::one())
    }

    fn two_pow(rhs: u32) -> Self {
        let mut out = <Fq as One>::one();
        for _ in 0..rhs {
            MontBackend::<FqConfig, 4>::double_in_place(&mut out);
        }

        P256(out)
    }

    fn inverse(self) -> Self {
        P256(ArkField::inverse(&self.0).expect("Unable to invert field element"))
    }

    fn to_le_bytes(&self) -> Vec<u8> {
        BigInt::to_bytes_le(&MontBackend::<FqConfig, 4>::into_bigint(self.0))
    }

    fn to_be_bytes(&self) -> Vec<u8> {
        BigInt::to_bytes_be(&MontBackend::<FqConfig, 4>::into_bigint(self.0))
    }
}

impl BitLength for P256 {
    const BITS: usize = 256;
}

impl GetBit<Lsb0> for P256 {
    fn get_bit(&self, index: usize) -> bool {
        MontBackend::<FqConfig, 4>::into_bigint(self.0).get_bit(index)
    }
}

impl GetBit<Msb0> for P256 {
    fn get_bit(&self, index: usize) -> bool {
        MontBackend::<FqConfig, 4>::into_bigint(self.0).get_bit(256 - index)
    }
}

impl FromBitIterator for P256 {
    fn from_lsb0_iter(iter: impl IntoIterator<Item = bool>) -> Self {
        P256(BigInt::from_bits_le(&iter.into_iter().collect::<Vec<bool>>()).into())
    }

    fn from_msb0_iter(iter: impl IntoIterator<Item = bool>) -> Self {
        P256(BigInt::from_bits_be(&iter.into_iter().collect::<Vec<bool>>()).into())
    }
}

impl BlockSerialize for P256 {
    type Serialized = [Block; 2];

    fn to_blocks(self) -> Self::Serialized {
        let limbs = self.0 .0 .0;
        let bytes: [u8; 32] = bytemuck::cast(limbs);

        let block_0: [u8; 16] = bytes[..16].try_into().unwrap();
        let block_1: [u8; 16] = bytes[16..].try_into().unwrap();

        [Block::new(block_0), Block::new(block_1)]
    }

    fn from_blocks(blocks: Self::Serialized) -> Self {
        let mut bytes = [0u8; 32];
        bytes[..16].copy_from_slice(&blocks[0].to_bytes());
        bytes[16..].copy_from_slice(&blocks[1].to_bytes());

        let limbs: [u64; 4] = bytemuck::cast(bytes);

        P256(Fq::from_bigint(BigInt::new(limbs)).unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::fields::{
        tests::{test_field_basic, test_field_bit_ops, test_field_compute_product_repeated},
        Field,
    };

    #[test]
    fn test_p256_basic() {
        test_field_basic::<P256>();
        assert_eq!(P256::new(0), P256::zero());
        assert_eq!(P256::new(1), P256::one());
    }

    #[test]
    fn test_p256_compute_product_repeated() {
        test_field_compute_product_repeated::<P256>();
    }

    #[test]
    fn test_p256_bit_ops() {
        test_field_bit_ops::<P256>();
    }

    #[test]
    fn test_p256_block_serialize() {
        let a = P256::new(42);
        let b = P256::from_blocks(a.to_blocks());

        assert_eq!(a.0 .0, b.0 .0);
    }

    #[test]
    fn test_bytemuck() {
        let a = [42u8; 32];
        let b: [u64; 4] = bytemuck::cast(a);
        let c: [u8; 32] = bytemuck::cast(b);

        assert_eq!(a, c);
    }
}
