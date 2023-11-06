//! Core components used to implement garbled circuit protocols
//!
//! This module implements "half-gate" garbled circuits from the [Two Halves Make a Whole \[ZRE15\]](https://eprint.iacr.org/2014/756) paper.
//!
//! # Example
//!
//! ```
//! use mpz_circuits::circuits::AES128;
//! use mpz_garble_core::{Generator, Evaluator, ChaChaEncoder, Encoder};
//!
//!
//! let encoder = ChaChaEncoder::new([0u8; 32]);
//! let encoded_key = encoder.encode::<[u8; 16]>(0);
//! let encoded_plaintext = encoder.encode::<[u8; 16]>(1);
//!
//! let key = b"super secret key";
//! let plaintext = b"super secret msg";
//!
//! let active_key = encoded_key.select(*key).unwrap();
//! let active_plaintext = encoded_plaintext.select(*plaintext).unwrap();
//!
//! let mut gen =
//!     Generator::new(
//!         AES128.clone(),
//!         encoder.delta(),
//!         &[encoded_key, encoded_plaintext]
//!     ).unwrap();
//!
//! let mut ev =
//!     Evaluator::new(
//!         AES128.clone(),
//!         &[active_key, active_plaintext]
//!     ).unwrap();
//!
//! const BATCH_SIZE: usize = 1000;
//! while !(gen.is_complete() && ev.is_complete()) {
//!     let batch: Vec<_> = gen.by_ref().take(BATCH_SIZE).collect();
//!     ev.evaluate(batch.iter());
//! }
//!
//! let encoded_outputs = gen.outputs().unwrap();
//! let encoded_ciphertext = encoded_outputs[0].clone();
//! let ciphertext_decoding = encoded_ciphertext.decoding();
//!
//! let active_outputs = ev.outputs().unwrap();
//! let active_ciphertext = active_outputs[0].clone();
//! let ciphertext: [u8; 16] =
//!     active_ciphertext.decode(&ciphertext_decoding).unwrap().try_into().unwrap();
//!
//! println!("'{plaintext:?} AES encrypted with key '{key:?}' is '{ciphertext:?}'");
//! ```

// [EMP-toolkit](https://github.com/emp-toolkit/emp-tool) was frequently used as a reference implementation during
// the development of this library.

#![deny(missing_docs, unreachable_pub, unused_must_use)]
#![deny(clippy::all)]

pub(crate) mod circuit;
pub mod encoding;
mod evaluator;
mod generator;
pub mod msg;

pub use circuit::{EncryptedGate, GarbledCircuit, EncryptedRow};
pub use encoding::{
    state as encoding_state, ChaChaEncoder, Decoding, Delta, Encode, EncodedValue, Encoder,
    EncodingCommitment, EqualityCheck, Label, ValueError,
};
pub use evaluator::{Evaluator, EvaluatorError};
pub use generator::{Generator, GeneratorError};

mod mode {
    use super::*;
    use mpz_core::aes::FixedKeyAes;

    mod sealed {
        pub trait Sealed {}

        impl Sealed for super::Normal {}
        impl Sealed for super::PrivacyFree {}
    }

    /// The mode of garbling to use
    pub trait GarbleMode: sealed::Sealed {
        /// The number of rows per gate
        const ROWS_PER_AND_GATE: usize;

        /// Garble an AND gate
        fn garble_and_gate(
            cipher: &FixedKeyAes,
            x_0: &Label,
            y_0: &Label,
            delta: &Delta,
            gid: usize,
            rows: &mut Vec<EncryptedRow>,
        ) -> Label;

        /// Evaluate an AND gate
        fn evaluate_and_gate(
            cipher: &FixedKeyAes,
            x: &Label,
            y: &Label,
            gid: usize,
            rows: &mut impl Iterator<Item = EncryptedRow>,
        ) -> Label;
    }

    /// Normal garbling mode
    pub struct Normal;

    impl GarbleMode for Normal {
        const ROWS_PER_AND_GATE: usize = 2;

        #[inline]
        fn garble_and_gate(
            cipher: &FixedKeyAes,
            x_0: &Label,
            y_0: &Label,
            delta: &Delta,
            gid: usize,
            rows: &mut Vec<EncryptedRow>,
        ) -> Label {
            generator::and_gate(cipher, x_0, y_0, delta, gid, rows)
        }

        #[inline]
        fn evaluate_and_gate(
            cipher: &FixedKeyAes,
            x: &Label,
            y: &Label,
            gid: usize,
            rows: &mut impl Iterator<Item = EncryptedRow>,
        ) -> Label {
            evaluator::and_gate(cipher, x, y, gid, rows)
        }
    }

    /// Privacy-free garbling mode
    pub struct PrivacyFree;

    impl GarbleMode for PrivacyFree {
        const ROWS_PER_AND_GATE: usize = 1;

        #[inline]
        fn garble_and_gate(
            cipher: &FixedKeyAes,
            x_0: &Label,
            y_0: &Label,
            delta: &Delta,
            gid: usize,
            rows: &mut Vec<EncryptedRow>,
        ) -> Label {
            generator::and_gate_pf(cipher, x_0, y_0, delta, gid, rows)
        }

        #[inline]
        fn evaluate_and_gate(
            cipher: &FixedKeyAes,
            x: &Label,
            y: &Label,
            gid: usize,
            rows: &mut impl Iterator<Item = EncryptedRow>,
        ) -> Label {
            evaluator::and_gate_pf(cipher, x, y, gid, rows)
        }
    }
}

pub use mode::{PrivacyFree, Normal, GarbleMode};

#[cfg(test)]
mod tests {
    use aes::{
        cipher::{BlockEncrypt, KeyInit},
        Aes128,
    };
    use mpz_circuits::{circuits::AES128, types::Value};
    use mpz_core::aes::FIXED_KEY_AES;
    use rand::SeedableRng;
    use rand_chacha::ChaCha12Rng;

    use super::*;

    #[test]
    fn test_and_gate() {
        use crate::{evaluator as ev, generator as gen};

        let mut rng = ChaCha12Rng::seed_from_u64(0);
        let cipher = &(*FIXED_KEY_AES);

        let mut rows = Vec::new();
        let delta = Delta::random(&mut rng);
        let x_0 = Label::random(&mut rng);
        let x_1 = x_0 ^ delta;
        let y_0 = Label::random(&mut rng);
        let y_1 = y_0 ^ delta;
        let gid: usize = 1;

        let z_0 = gen::and_gate(cipher, &x_0, &y_0, &delta, gid, &mut rows);
        let z_1 = z_0 ^ delta;

        assert_eq!(ev::and_gate(cipher, &x_0, &y_1, gid, &mut rows.iter().copied()), z_0);
        assert_eq!(ev::and_gate(cipher, &x_0, &y_1, gid, &mut rows.iter().copied()), z_0);
        assert_eq!(ev::and_gate(cipher, &x_1, &y_0, gid, &mut rows.iter().copied()), z_0);
        assert_eq!(ev::and_gate(cipher, &x_1, &y_1, gid, &mut rows.iter().copied()), z_1);
    }

    // #[test]
    // fn test_and_gate_privacy_free() {
    //     use crate::{evaluator as ev, generator as gen};

    //     let mut rng = ChaCha12Rng::seed_from_u64(0);
    //     let cipher = &(*FIXED_KEY_AES);

    //     let delta = Delta::random(&mut rng);
    //     let x_0 = Label::random(&mut rng);
    //     let x_1 = x_0 ^ delta;
    //     let y_0 = Label::random(&mut rng);
    //     let y_1 = y_0 ^ delta;
    //     let gid: usize = 1;

    //     let (z_0, encrypted_gate) = gen::and_gate_pf(cipher, &x_0, &y_0, &delta, gid);
    //     let z_1 = z_0 ^ delta;

    //     assert_eq!(
    //         ev::and_gate_pf(cipher, &x_0, &y_1, &encrypted_gate, gid),
    //         z_0
    //     );
    //     assert_eq!(
    //         ev::and_gate_pf(cipher, &x_0, &y_1, &encrypted_gate, gid),
    //         z_0
    //     );
    //     assert_eq!(
    //         ev::and_gate_pf(cipher, &x_1, &y_0, &encrypted_gate, gid),
    //         z_0
    //     );
    //     assert_eq!(
    //         ev::and_gate_pf(cipher, &x_1, &y_1, &encrypted_gate, gid),
    //         z_1
    //     );
    // }

    #[test]
    fn test_garble() {
        let encoder = ChaChaEncoder::new([0; 32]);

        let key = [69u8; 16];
        let msg = [42u8; 16];
        const BATCH_SIZE: usize = 1000;

        let expected: [u8; 16] = {
            let cipher = Aes128::new_from_slice(&key).unwrap();
            let mut out = msg.into();
            cipher.encrypt_block(&mut out);
            out.into()
        };

        let full_inputs: Vec<EncodedValue<encoding_state::Full>> = AES128
            .inputs()
            .iter()
            .map(|input| encoder.encode_by_type(0, &input.value_type()))
            .collect();

        let active_inputs: Vec<EncodedValue<encoding_state::Active>> = vec![
            full_inputs[0].clone().select(key).unwrap(),
            full_inputs[1].clone().select(msg).unwrap(),
        ];

        let mut gen =
            Generator::<Normal>::new_with_hasher(AES128.clone(), encoder.delta(), &full_inputs).unwrap();
        let mut ev = Evaluator::<Normal>::new_with_hasher(AES128.clone(), &active_inputs).unwrap();

        while !(gen.is_complete() && ev.is_complete()) {
            ev.evaluate(gen.generate(BATCH_SIZE)).unwrap();
        }

        let full_outputs = gen.outputs().unwrap();
        let active_outputs = ev.outputs().unwrap();

        let gen_digest = gen.hash().unwrap();
        let ev_digest = ev.hash().unwrap();

        assert_eq!(gen_digest, ev_digest);

        let outputs: Vec<Value> = active_outputs
            .iter()
            .zip(full_outputs)
            .map(|(active_output, full_output)| {
                active_output.decode(&full_output.decoding()).unwrap()
            })
            .collect();

        let actual: [u8; 16] = outputs[0].clone().try_into().unwrap();

        assert_eq!(actual, expected);
    }
}
