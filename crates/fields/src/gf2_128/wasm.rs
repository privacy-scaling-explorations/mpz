//! WASM `simd128` backend for GF(2¹²⁸).
//!
//! Multiplication runs on `bmul_simd::clmul32_x4` (32×32→64 carry-less
//! pieces via single-multiply `extmul`s), recombined with two-level
//! Karatsuba: 64→32 splits each 64×64 product into three clmul32
//! streams, 128→64 splits the full product into three 64×64 products —
//! nine streams total, packed across `extmul` lanes. All recombination
//! is linear over GF(2), so the accumulating kernels defer it (and the
//! final reduction mod p(x) = x¹²⁸+x⁷+x²+x+1) out of their loops.

use std::arch::wasm32::*;

use crate::bmul_simd::{bit_spread_v128, clmul32_x2, clmul32_x4};

use super::Gf2_128;

/// Splits an element into the two operand packs and the lone scalar:
/// pack1 = `[w0, w1, w2, w3]` (streams P00.q00, P00.q11 | P11.q00,
/// P11.q11), pack2 = `[w0^w1, w2^w3, w0^w2, w1^w3]` (P00.qm, P11.qm |
/// PM.q00, PM.q11) and scalar `(w0^w2)^(w1^w3)` (PM.qm), where P00/P11/PM
/// are the outer (64-bit) Karatsuba streams and q00/q11/qm the inner
/// (32-bit) ones.
#[inline(always)]
fn packs(v: u128) -> (v128, v128, u32) {
    let w0 = v as u32;
    let w1 = (v >> 32) as u32;
    let w2 = (v >> 64) as u32;
    let w3 = (v >> 96) as u32;
    let m0 = w0 ^ w2;
    let m1 = w1 ^ w3;
    (u32x4(w0, w1, w2, w3), u32x4(w0 ^ w1, w2 ^ w3, m0, m1), m0 ^ m1)
}

/// Recombines one 64×64 Karatsuba-32 stream triple into a 128-bit product.
#[inline(always)]
fn comb64(q00: u64, q11: u64, qm: u64) -> u128 {
    let mid = qm ^ q00 ^ q11;
    (q00 as u128) ^ ((q11 as u128) << 64) ^ ((mid as u128) << 32)
}

/// Recovers the unreduced 256-bit product `(lo, hi)` from the accumulated
/// stream pairs of [`packs`]-shaped operands: `acc_p` holds P00/P11's
/// q00 and q11 streams, `acc_q` holds their qm streams plus PM's q00 and
/// q11, and `pm_qm` is PM's accumulated qm stream.
#[inline(always)]
fn recover(acc_p: (v128, v128), acc_q: (v128, v128), pm_qm: u64) -> (u128, u128) {
    let p00 = comb64(
        u64x2_extract_lane::<0>(acc_p.0),
        u64x2_extract_lane::<1>(acc_p.0),
        u64x2_extract_lane::<0>(acc_q.0),
    );
    let p11 = comb64(
        u64x2_extract_lane::<0>(acc_p.1),
        u64x2_extract_lane::<1>(acc_p.1),
        u64x2_extract_lane::<1>(acc_q.0),
    );
    let pm = comb64(
        u64x2_extract_lane::<0>(acc_q.1),
        u64x2_extract_lane::<1>(acc_q.1),
        pm_qm,
    );

    let mid = pm ^ p00 ^ p11;
    (p00 ^ (mid << 64), p11 ^ (mid >> 64))
}

#[inline]
pub(super) fn mul(a: u128, b: u128) -> u128 {
    let (lo, hi) = mul_full(a, b);
    reduce128(lo, hi)
}

/// Unreduced 256-bit carry-less product `a · b`, as `(lo, hi)`. The
/// accumulator XORs these and reduces once with [`reduce`].
#[inline]
pub(super) fn mul_full(a: u128, b: u128) -> (u128, u128) {
    let zero = u64x2_splat(0);
    let (ap1, ap2, am) = packs(a);
    let (bp1, bp2, bm) = packs(b);

    let acc_p = clmul32_x4(ap1, bp1, (zero, zero));
    let acc_q = clmul32_x4(ap2, bp2, (zero, zero));
    let r = clmul32_x2(u32x4(am, 0, 0, 0), u32x4(bm, 0, 0, 0), zero);

    recover(acc_p, acc_q, u64x2_extract_lane::<0>(r))
}

/// Reduces an accumulated 256-bit polynomial `hi·x¹²⁸ + lo` to a field element.
#[inline]
pub(super) fn reduce(lo: u128, hi: u128) -> u128 {
    reduce128(lo, hi)
}

/// Squaring via parallel bit-spread. In characteristic 2,
/// `(a_lo + a_hi · x⁶⁴)² = a_lo² + a_hi² · x¹²⁸` — the cross term
/// vanishes. Each half-square is a pure bit-spread of a u64 into a
/// u128. We run two 32→64 spreads per v128, so the whole 256-bit
/// squared polynomial comes from two `bit_spread_v128` calls — zero
/// multiplies.
#[inline]
pub(super) fn square(a: u128) -> u128 {
    let v_lo = bit_spread_v128(u64x2(a as u32 as u64, (a >> 32) as u32 as u64));
    let v_hi = bit_spread_v128(u64x2((a >> 64) as u32 as u64, (a >> 96) as u64));

    let ll = u64x2_extract_lane::<0>(v_lo);
    let lh = u64x2_extract_lane::<1>(v_lo);
    let hl = u64x2_extract_lane::<0>(v_hi);
    let hh = u64x2_extract_lane::<1>(v_hi);

    // a_lo² fills bits [0..128], a_hi² fills bits [128..256].
    let lo = ((lh as u128) << 64) | (ll as u128);
    let hi = ((hh as u128) << 64) | (hl as u128);
    reduce128(lo, hi)
}

#[inline]
pub(super) fn inner_product(a: &[Gf2_128], b: &[Gf2_128]) -> u128 {
    let zero = u64x2_splat(0);
    let mut acc_p = (zero, zero);
    let mut acc_q = (zero, zero);
    let mut acc_r = (zero, zero);

    // The lone PM.qm stream of four consecutive elements shares one
    // full-width clmul32 pack; every recombination the streams need is
    // linear, so it happens once in `recover`.
    let ca = a.chunks_exact(4);
    let cb = b.chunks_exact(4);
    let (ra, rb) = (ca.remainder(), cb.remainder());
    for (xs4, ys4) in ca.zip(cb) {
        let mut xs = [0u32; 4];
        let mut ys = [0u32; 4];
        for e in 0..4 {
            let (xp1, xp2, xsc) = packs(xs4[e].0);
            let (yp1, yp2, ysc) = packs(ys4[e].0);
            acc_p = clmul32_x4(xp1, yp1, acc_p);
            acc_q = clmul32_x4(xp2, yp2, acc_q);
            xs[e] = xsc;
            ys[e] = ysc;
        }
        acc_r = clmul32_x4(
            u32x4(xs[0], xs[1], xs[2], xs[3]),
            u32x4(ys[0], ys[1], ys[2], ys[3]),
            acc_r,
        );
    }

    for (x, y) in ra.iter().zip(rb.iter()) {
        let (xp1, xp2, xs) = packs(x.0);
        let (yp1, yp2, ys) = packs(y.0);
        acc_p = clmul32_x4(xp1, yp1, acc_p);
        acc_q = clmul32_x4(xp2, yp2, acc_q);
        acc_r.0 = clmul32_x2(u32x4(xs, 0, 0, 0), u32x4(ys, 0, 0, 0), acc_r.0);
    }

    let r = v128_xor(acc_r.0, acc_r.1);
    let pm_qm = u64x2_extract_lane::<0>(r) ^ u64x2_extract_lane::<1>(r);
    let (lo, hi) = recover(acc_p, acc_q, pm_qm);
    reduce128(lo, hi)
}

/// `Σ aᵢ · bᵢ · cᵢ`. Per iteration: one full `mul(aᵢ, bᵢ)` to get the
/// 128-bit `xy` intermediate, then accumulate the `(xy · cᵢ)` streams,
/// deferring their recombination and the final reduction to one
/// post-loop pass.
#[inline]
pub(super) fn double_inner_product(a: &[Gf2_128], b: &[Gf2_128], c: &[Gf2_128]) -> u128 {
    let zero = u64x2_splat(0);
    let mut acc_p = (zero, zero);
    let mut acc_q = (zero, zero);
    let mut acc_r = zero;

    for ((x, y), z) in a.iter().zip(b.iter()).zip(c.iter()) {
        let xy = mul(x.0, y.0);

        let (xp1, xp2, xs) = packs(xy);
        let (zp1, zp2, zs) = packs(z.0);
        acc_p = clmul32_x4(xp1, zp1, acc_p);
        acc_q = clmul32_x4(xp2, zp2, acc_q);
        acc_r = clmul32_x2(u32x4(xs, 0, 0, 0), u32x4(zs, 0, 0, 0), acc_r);
    }

    let pm_qm = u64x2_extract_lane::<0>(acc_r);
    let (lo, hi) = recover(acc_p, acc_q, pm_qm);
    reduce128(lo, hi)
}

#[inline]
pub(super) fn inverse(a: u128) -> u128 {
    let mut y = square(a);
    let mut out = y;
    for _ in 2..128 {
        y = square(y);
        out = mul(out, y);
    }
    out
}

/// Reduce a 256-bit polynomial `hi·2¹²⁸ + lo` modulo
/// p(x) = x¹²⁸ + x⁷ + x² + x + 1 (so `x¹²⁸ ≡ R = x⁷+x²+x+1`).
#[inline(always)]
fn reduce128(lo: u128, hi: u128) -> u128 {
    let folded_lo = hi ^ (hi << 1) ^ (hi << 2) ^ (hi << 7);
    let overflow = (hi >> 127) ^ (hi >> 126) ^ (hi >> 121);
    let overflow_folded = overflow ^ (overflow << 1) ^ (overflow << 2) ^ (overflow << 7);
    lo ^ folded_lo ^ overflow_folded
}
