use mpz_core::aes::FixedKeyAes;

pub(crate) const TILE_TARGET_BLOCKS: usize = 1 << 13;

pub(crate) fn xor16(a: [u8; 16], b: [u8; 16]) -> [u8; 16] {
    std::array::from_fn(|i| a[i] ^ b[i])
}

pub(crate) fn ctr_block(c: u64) -> [u8; 16] {
    let mut b = [0u8; 16];
    b[..8].copy_from_slice(&c.to_le_bytes());
    b
}

pub(crate) fn xor_into(dst: &mut [u8], src: &[u8]) {
    debug_assert_eq!(dst.len(), src.len());
    debug_assert_eq!(dst.len() % 16, 0);
    for (d, s) in dst.chunks_exact_mut(16).zip(src.chunks_exact(16)) {
        let a: [u8; 16] = d.try_into().expect("16 bytes");
        let b: [u8; 16] = s.try_into().expect("16 bytes");
        let r: [u8; 16] = std::array::from_fn(|i| a[i] ^ b[i]);
        d.copy_from_slice(&r);
    }
}

pub(crate) fn xor_masked_into(dst: &mut [u8], src: &[u8], mask: [u8; 16]) {
    debug_assert_eq!(dst.len(), src.len());
    debug_assert_eq!(dst.len() % 16, 0);
    for (d, s) in dst.chunks_exact_mut(16).zip(src.chunks_exact(16)) {
        let a: [u8; 16] = d.try_into().expect("16 bytes");
        let b: [u8; 16] = s.try_into().expect("16 bytes");
        let r: [u8; 16] = std::array::from_fn(|i| a[i] ^ (b[i] & mask[i]));
        d.copy_from_slice(&r);
    }
}

pub(crate) fn stretch(
    aes: &FixedKeyAes,
    leaves: &[[u8; 16]],
    tile_blocks: usize,
    ctr: u64,
    src: &mut [[u8; 16]],
    scratch: &mut [[u8; 16]],
) {
    let q = leaves.len();
    let n = q * tile_blocks;
    for (x, &leaf) in leaves.iter().enumerate() {
        for j in 0..tile_blocks {
            src[x * tile_blocks + j] = xor16(leaf, ctr_block(ctr + j as u64));
        }
    }
    aes.mmo_blocks_to(&src[..n], &mut scratch[..n]);
}

pub(crate) fn fold_emit(
    scratch: &mut [u8],
    tb: usize,
    k: usize,
    slab: &mut [u8],
    stride: usize,
    col: usize,
    u: &mut [u8],
) {
    debug_assert_eq!(scratch.len(), (1 << k) * tb);

    let mut size = 1 << k;
    for t in 0..k {
        let dst = &mut slab[t * stride + col..t * stride + col + tb];
        dst.copy_from_slice(&scratch[tb..2 * tb]);
        let mut j = 3;
        while j < size {
            xor_into(dst, &scratch[j * tb..(j + 1) * tb]);
            j += 2;
        }

        for j in 0..size / 2 {
            if j != 0 {
                scratch.copy_within(2 * j * tb..(2 * j + 1) * tb, j * tb);
            }
            let (head, tail) = scratch.split_at_mut((2 * j + 1) * tb);
            xor_into(&mut head[j * tb..(j + 1) * tb], &tail[..tb]);
        }
        size /= 2;
    }

    u.copy_from_slice(&scratch[..tb]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{Rng, SeedableRng, rngs::StdRng};

    #[test]
    fn test_fold_emit_matches_naive() {
        let mut rng = StdRng::seed_from_u64(0);

        for k in [2usize, 4, 8] {
            for tb in [16usize, 48] {
                let q = 1 << k;
                let rows: Vec<u8> = (0..q * tb).map(|_| rng.random()).collect();

                let mut u_ref = vec![0u8; tb];
                let mut v_ref = vec![0u8; k * tb];
                for x in 0..q {
                    xor_into(&mut u_ref, &rows[x * tb..(x + 1) * tb]);
                    for i in 0..k {
                        if (x >> i) & 1 == 1 {
                            xor_into(
                                &mut v_ref[i * tb..(i + 1) * tb],
                                &rows[x * tb..(x + 1) * tb],
                            );
                        }
                    }
                }

                let stride = tb + 32;
                let col = 16;
                let mut slab = vec![0u8; k * stride];
                let mut u = vec![0u8; tb];
                let mut scratch = rows.clone();
                fold_emit(&mut scratch, tb, k, &mut slab, stride, col, &mut u);

                assert_eq!(u, u_ref, "k={k} tb={tb} u mismatch");
                for i in 0..k {
                    assert_eq!(
                        &slab[i * stride + col..i * stride + col + tb],
                        &v_ref[i * tb..(i + 1) * tb],
                        "k={k} tb={tb} v[{i}] mismatch"
                    );
                }
            }
        }
    }
}
