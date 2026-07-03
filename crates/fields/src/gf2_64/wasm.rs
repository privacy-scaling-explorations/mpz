//! WASM `simd128` backend for GF(2⁶⁴).
//!
//! Multiplication runs on `bmul_simd::clmul32_x4` (32×32→64 carry-less
//! pieces via single-multiply `extmul`s): Karatsuba splits the 64×64
//! product into three clmul32 streams — q00, q11 and the middle — which
//! fit one pack. The recombination is linear over GF(2), so the
//! accumulating kernels defer it (and the final reduction mod
//! p(x) = x⁶⁴+x⁴+x³+x+1) out of their loops.

use std::arch::wasm32::*;

use crate::bmul_simd::{bit_spread_v128, clmul32_x4};

use super::Gf2_64;

/// Splits an element into its clmul32 stream operands
/// `[w0, w1, w0^w1, 0]` (streams q00, q11, qm).
#[inline(always)]
fn pack(v: u64) -> v128 {
    let w0 = v as u32;
    let w1 = (v >> 32) as u32;
    u32x4(w0, w1, w0 ^ w1, 0)
}

/// Recovers the unreduced 128-bit product from an accumulated stream
/// pack of [`pack`]-shaped operands.
#[inline(always)]
fn recover(acc: (v128, v128)) -> u128 {
    let q00 = u64x2_extract_lane::<0>(acc.0);
    let q11 = u64x2_extract_lane::<1>(acc.0);
    let qm = u64x2_extract_lane::<0>(acc.1);

    let mid = qm ^ q00 ^ q11;
    (q00 as u128) ^ ((q11 as u128) << 64) ^ ((mid as u128) << 32)
}

#[inline]
pub(super) fn mul(a: u64, b: u64) -> u64 {
    reduce(mul_full(a, b))
}

/// Unreduced carry-less product `a · b` (≤ 127 bits) packed into a `u128`.
/// The accumulator XORs these and reduces once with [`reduce`].
#[inline]
pub(super) fn mul_full(a: u64, b: u64) -> u128 {
    let zero = u64x2_splat(0);
    recover(clmul32_x4(pack(a), pack(b), (zero, zero)))
}

/// Reduces an accumulated 128-bit polynomial to a field element.
#[inline]
pub(super) fn reduce(prod: u128) -> u64 {
    reduce64(prod as u64, (prod >> 64) as u64)
}

/// Squaring via parallel bit-spread. In char 2,
/// `(Σ aᵢ xⁱ)² = Σ aᵢ x^(2i)`. Pack the low and high 32-bit halves of
/// `a` into the two v128 lanes and run the 32→64 bit-spread on both
/// simultaneously — no multiplies at all.
#[inline]
pub(super) fn square(a: u64) -> u64 {
    let v = bit_spread_v128(u64x2(a as u32 as u64, (a >> 32) as u64));
    let lo = u64x2_extract_lane::<0>(v);
    let hi = u64x2_extract_lane::<1>(v);
    reduce64(lo, hi)
}

#[inline]
pub(super) fn inner_product(a: &[Gf2_64], b: &[Gf2_64]) -> u64 {
    let zero = u64x2_splat(0);
    let mut acc = (zero, zero);
    for (x, y) in a.iter().zip(b.iter()) {
        acc = clmul32_x4(pack(x.0), pack(y.0), acc);
    }
    reduce(recover(acc))
}

/// `Σ aᵢ · bᵢ · cᵢ`. Per iteration: one full `mul(aᵢ, bᵢ)` to get the
/// 64-bit `xy` intermediate, then accumulate the `(xy · cᵢ)` streams,
/// deferring their recombination and the final reduction to one
/// post-loop pass.
#[inline]
pub(super) fn double_inner_product(a: &[Gf2_64], b: &[Gf2_64], c: &[Gf2_64]) -> u64 {
    let zero = u64x2_splat(0);
    let mut acc = (zero, zero);
    for ((x, y), z) in a.iter().zip(b.iter()).zip(c.iter()) {
        let xy = mul(x.0, y.0);
        acc = clmul32_x4(pack(xy), pack(z.0), acc);
    }
    reduce(recover(acc))
}

#[inline]
pub(super) fn inverse(a: u64) -> u64 {
    let mut y = square(a);
    let mut out = y;
    for _ in 2..64 {
        y = square(y);
        out = mul(out, y);
    }
    out
}

/// Reduce a 128-bit polynomial `hi·2⁶⁴ + lo` modulo
/// p(x) = x⁶⁴ + x⁴ + x³ + x + 1 (so `x⁶⁴ ≡ R = x⁴+x³+x+1`).
#[inline(always)]
fn reduce64(lo: u64, hi: u64) -> u64 {
    let folded_lo = hi ^ (hi << 1) ^ (hi << 3) ^ (hi << 4);
    let overflow = (hi >> 63) ^ (hi >> 61) ^ (hi >> 60);
    let overflow_folded = overflow ^ (overflow << 1) ^ (overflow << 3) ^ (overflow << 4);
    lo ^ folded_lo ^ overflow_folded
}
