use mpz_core::{Block, aes::AesEncryptor};
use mpz_fields::{
    Accumulator,
    gf2_128::{Gf2_128, Gf2_128Accumulator},
};
use zerocopy::FromBytes;

#[cfg(feature = "rayon")]
use rayon::prelude::*;

use crate::softspoken::{CSP, fold::ctr_block};

#[cfg(feature = "rayon")]
const CHI_CHUNK: usize = 1 << 11;

type Partial = ([Gf2_128Accumulator; CSP], Gf2_128Accumulator);

fn fold_range(
    aes: &AesEncryptor,
    mac: &[u8],
    total_rb: usize,
    choices: Option<&[u8]>,
    c0: usize,
    c1: usize,
) -> Partial {
    let mut chi: Vec<[u8; 16]> = (c0..c1).map(|j| ctr_block(j as u64)).collect();
    aes.encrypt_blocks(&mut chi);
    let chi: Vec<Gf2_128> = chi.iter().map(|&b| Block::from(b).into()).collect();

    let mut accs = [Gf2_128Accumulator::zero(); CSP];
    for (r, acc) in accs.iter_mut().enumerate() {
        let row = <[Block]>::ref_from_bytes(&mac[r * total_rb + c0 * 16..r * total_rb + c1 * 16])
            .expect("multiple of Block size");
        for (&c, &v) in chi.iter().zip(row) {
            acc.add_product(c, v.into());
        }
    }

    let mut acc_x = Gf2_128Accumulator::zero();
    if let Some(ch) = choices {
        let row = <[Block]>::ref_from_bytes(&ch[c0 * 16..c1 * 16]).expect("multiple of Block size");
        for (&c, &v) in chi.iter().zip(row) {
            acc_x.add_product(c, v.into());
        }
    }

    (accs, acc_x)
}

#[cfg(feature = "rayon")]
fn merge(mut a: Partial, b: Partial) -> Partial {
    for (x, y) in a.0.iter_mut().zip(b.0.iter()) {
        x.merge(y);
    }
    a.1.merge(&b.1);
    a
}

fn finish(acc: Gf2_128Accumulator, row_bytes: &[u8], m: usize) -> Gf2_128 {
    let cst: Gf2_128 = <[Block]>::ref_from_bytes(&row_bytes[m * 16..(m + 1) * 16])
        .expect("multiple of Block size")[0]
        .into();
    acc.reduce() + cst
}

pub(crate) fn check_fold(
    seed: Block,
    mac: &[u8],
    total_rb: usize,
    choices: Option<&[u8]>,
) -> (Vec<Gf2_128>, Option<Gf2_128>) {
    let m = total_rb / 16 - 1;
    let aes = AesEncryptor::new(seed.to_bytes());

    cfg_if::cfg_if! {
        if #[cfg(feature = "rayon")] {
            let (accs, acc_x) = (0..m.div_ceil(CHI_CHUNK))
                .into_par_iter()
                .map(|ci| {
                    let c0 = ci * CHI_CHUNK;
                    let c1 = ((ci + 1) * CHI_CHUNK).min(m);
                    fold_range(&aes, mac, total_rb, choices, c0, c1)
                })
                .reduce(
                    || ([Gf2_128Accumulator::zero(); CSP], Gf2_128Accumulator::zero()),
                    merge,
                );
        } else {
            let (accs, acc_x) = fold_range(&aes, mac, total_rb, choices, 0, m);
        }
    }

    let t = accs
        .iter()
        .enumerate()
        .map(|(r, &acc)| finish(acc, &mac[r * total_rb..(r + 1) * total_rb], m))
        .collect();
    let x = choices.map(|c| finish(acc_x, c, m));

    (t, x)
}

#[cfg(test)]
mod tests {
    use super::{CSP, check_fold};

    use mpz_core::{Block, aes::AesEncryptor};
    use mpz_fields::gf2_128::Gf2_128;

    use rand::{Rng, SeedableRng, rngs::StdRng};

    fn naive_check_fold(
        seed: Block,
        mac: &[u8],
        total_rb: usize,
        choices: Option<&[u8]>,
    ) -> (Vec<Gf2_128>, Option<Gf2_128>) {
        let m = total_rb / 16 - 1;

        let aes = AesEncryptor::new(seed.to_bytes());
        let mut chi: Vec<[u8; 16]> = (0..m)
            .map(|j| {
                let mut b = [0u8; 16];
                b[..8].copy_from_slice(&(j as u64).to_le_bytes());
                b
            })
            .collect();
        aes.encrypt_blocks(&mut chi);
        let chi: Vec<Gf2_128> = chi.iter().map(|&b| Block::from(b).into()).collect();

        let block_at = |row: &[u8], col: usize| -> Gf2_128 {
            let mut b = [0u8; 16];
            b.copy_from_slice(&row[col * 16..(col + 1) * 16]);
            Block::from(b).into()
        };
        let fold_row = |row: &[u8]| -> Gf2_128 {
            let acc = chi
                .iter()
                .enumerate()
                .fold(Gf2_128::new(0), |acc, (j, &c)| acc + c * block_at(row, j));
            acc + block_at(row, m)
        };

        let t = (0..CSP)
            .map(|r| fold_row(&mac[r * total_rb..(r + 1) * total_rb]))
            .collect();
        let x = choices.map(fold_row);
        (t, x)
    }

    #[test]
    fn check_fold_matches_naive() {
        let mut rng = StdRng::seed_from_u64(0);

        for cols in [2usize, 3, 17, 64, 2050] {
            let total_rb = cols * 16;
            let mac: Vec<u8> = (0..CSP * total_rb).map(|_| rng.random()).collect();
            let choices: Vec<u8> = (0..total_rb).map(|_| rng.random()).collect();
            let seed: Block = rng.random::<[u8; 16]>().into();

            let (t, x) = check_fold(seed, &mac, total_rb, Some(&choices));
            let (t_ref, x_ref) = naive_check_fold(seed, &mac, total_rb, Some(&choices));
            assert_eq!(t, t_ref, "cols={cols} t mismatch");
            assert_eq!(x, x_ref, "cols={cols} x mismatch");

            let (t_none, x_none) = check_fold(seed, &mac, total_rb, None);
            assert_eq!(t_none, t_ref, "cols={cols} t (no choices) mismatch");
            assert!(x_none.is_none());
        }
    }
}
