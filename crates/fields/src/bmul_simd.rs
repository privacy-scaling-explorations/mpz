//! WASM `simd128` carry-less multiplication primitives.
//!
//! WASM has no carry-less-multiply instruction in `simd128` (neither
//! stable nor relaxed-SIMD), so multiplication is built from the BearSSL
//! bit-interleaving algorithm over 32×32→64 integer products
//! ([`clmul32_x4`]). The `i64x2.extmul_*_u32x4` instructions each compute
//! two such products and lower to a single widening multiply on common
//! hosts — much cheaper than `i64x2.mul`, which engines emulate with a
//! multi-instruction sequence. Working from 32-bit pieces also yields the
//! full product with no truncation, so no bit-reversal trick is needed;
//! wider multiplies recombine the 64-bit pieces with Karatsuba, and since
//! the recombination is linear over GF(2) it can be deferred across
//! XOR-accumulation loops.

use std::arch::wasm32::*;

/// Spreads two `u32`s (one per lane) into two `u64`s with one zero
/// between each bit — the GF(2^n) squaring primitive, run on both
/// v128 lanes in parallel.
#[inline(always)]
pub(crate) fn bit_spread_v128(mut v: v128) -> v128 {
    v = v128_and(
        v128_or(v, u64x2_shl(v, 16)),
        u64x2_splat(0x0000_FFFF_0000_FFFF),
    );
    v = v128_and(
        v128_or(v, u64x2_shl(v, 8)),
        u64x2_splat(0x00FF_00FF_00FF_00FF),
    );
    v = v128_and(
        v128_or(v, u64x2_shl(v, 4)),
        u64x2_splat(0x0F0F_0F0F_0F0F_0F0F),
    );
    v = v128_and(
        v128_or(v, u64x2_shl(v, 2)),
        u64x2_splat(0x3333_3333_3333_3333),
    );
    v = v128_and(
        v128_or(v, u64x2_shl(v, 1)),
        u64x2_splat(0x5555_5555_5555_5555),
    );
    v
}

/// BearSSL 4-mask clmul32 over all four u32 lane pairs of `a` × `b`,
/// XORed into `acc = (low lane products, high lane products)`: result
/// u64 lane `l` of the pair is the full 32×32 carry-less product of
/// `a`'s and `b`'s u32 lane `l`, one `extmul` per two products.
///
/// Carry safety: 4-bit mask spacing leaves each product column with at
/// most 8 addends, so integer carries never reach the next lattice
/// position and are cleared by the per-`k` masking.
#[inline(always)]
pub(crate) fn clmul32_x4(a: v128, b: v128, acc: (v128, v128)) -> (v128, v128) {
    let mut t_lo = [u64x2_splat(0); 4];
    let mut t_hi = [u64x2_splat(0); 4];

    let am = mask4(a);
    let bm = mask4(b);

    for i in 0..4 {
        for j in 0..4 {
            let k = (i + j) & 3;
            t_lo[k] = v128_xor(t_lo[k], i64x2_extmul_low_u32x4(am[i], bm[j]));
            t_hi[k] = v128_xor(t_hi[k], i64x2_extmul_high_u32x4(am[i], bm[j]));
        }
    }

    let mut r_lo = acc.0;
    let mut r_hi = acc.1;
    for k in 0..4 {
        let mk = u64x2_splat(0x1111_1111_1111_1111 << k);
        r_lo = v128_xor(r_lo, v128_and(t_lo[k], mk));
        r_hi = v128_xor(r_hi, v128_and(t_hi[k], mk));
    }
    (r_lo, r_hi)
}

/// Low-lanes-only variant of [`clmul32_x4`], for packs with only the
/// two low u32 lane pairs populated.
#[inline(always)]
pub(crate) fn clmul32_x2(a: v128, b: v128, acc: v128) -> v128 {
    let mut t = [u64x2_splat(0); 4];

    let am = mask4(a);
    let bm = mask4(b);

    for i in 0..4 {
        for j in 0..4 {
            let k = (i + j) & 3;
            t[k] = v128_xor(t[k], i64x2_extmul_low_u32x4(am[i], bm[j]));
        }
    }

    let mut r = acc;
    for k in 0..4 {
        let mk = u64x2_splat(0x1111_1111_1111_1111 << k);
        r = v128_xor(r, v128_and(t[k], mk));
    }
    r
}

/// The four BearSSL lattice maskings of `v`.
#[inline(always)]
fn mask4(v: v128) -> [v128; 4] {
    [
        v128_and(v, u32x4_splat(0x1111_1111)),
        v128_and(v, u32x4_splat(0x2222_2222)),
        v128_and(v, u32x4_splat(0x4444_4444)),
        v128_and(v, u32x4_splat(0x8888_8888)),
    ]
}
