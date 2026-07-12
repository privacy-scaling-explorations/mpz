//! Correlated OT extension via SoftSpoken.
//!
//! A [`Sender`] and [`Receiver`] turn a fixed number of base OTs into many
//! correlated OTs. Each party is a state machine driven through setup,
//! extension, and a consistency check, exchanging the [`Corrections`],
//! [`Extend`], and [`Check`] messages produced by the other.
//!
//! # Warning
//!
//! This protocol admits a limited amount of leakage. Callers must carefully
//! consider whether that leakage is acceptable for their application.

mod check;
mod config;
mod error;
mod fold;
mod ggm;
mod receiver;
mod sender;

pub use config::{
    ReceiverConfig, ReceiverConfigBuilder, ReceiverConfigBuilderError, SenderConfig,
    SenderConfigBuilder, SenderConfigBuilderError,
};
pub use error::{ReceiverError, SenderError};
use mpz_fields::gf2_128::Gf2_128;
pub use receiver::{Receiver, state as receiver_state};
pub use sender::{Sender, state as sender_state};
use serde::{Deserialize, Serialize};

/// Computational security parameter, in bits.
pub const CSP: usize = 128;
/// Statistical security parameter, in bits.
pub const SSP: usize = 128;

const TREE_CORRECTIONS: usize = 2 * CSP;

pub(crate) const SUPPORTED_K: [usize; 3] = [2, 4, 8];

/// Setup message sent from the [`Receiver`] to the [`Sender`].
///
/// Produced by [`Receiver::corrections`] and consumed by
/// [`Sender::corrections`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "validation::CorrectionsUnchecked")]
pub struct Corrections {
    corrections: Vec<[u8; 16]>,
    s: [u8; 16],
}

/// Extension message sent from the [`Receiver`] to the [`Sender`].
///
/// Produced by [`Receiver::extend`] and consumed by [`Sender::extend`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "validation::ExtendUnchecked")]
pub struct Extend {
    count: usize,
    us: Vec<u8>,
}

/// Consistency-check message sent from the [`Receiver`] to the [`Sender`].
///
/// Produced by [`Receiver::check`] and verified by [`Sender::check`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "validation::CheckUnchecked")]
pub struct Check {
    x: Gf2_128,
    t: Vec<Gf2_128>,
}

mod validation {
    use super::*;

    #[derive(Deserialize)]
    pub(super) struct CorrectionsUnchecked {
        corrections: Vec<[u8; 16]>,
        s: [u8; 16],
    }

    impl TryFrom<CorrectionsUnchecked> for Corrections {
        type Error = String;

        fn try_from(value: CorrectionsUnchecked) -> Result<Self, Self::Error> {
            if value.corrections.len() != TREE_CORRECTIONS {
                return Err("invalid tree corrections length".to_string());
            }

            Ok(Corrections {
                corrections: value.corrections,
                s: value.s,
            })
        }
    }

    #[derive(Deserialize)]
    pub(super) struct ExtendUnchecked {
        count: usize,
        us: Vec<u8>,
    }

    impl TryFrom<ExtendUnchecked> for Extend {
        type Error = String;

        fn try_from(value: ExtendUnchecked) -> Result<Self, Self::Error> {
            let ExtendUnchecked { count, us } = value;

            if count == 0 || count % CSP != 0 {
                return Err("count must be a positive multiple of CSP".to_string());
            }

            let total = count / 8 * CSP;
            if us.is_empty() || total % us.len() != 0 {
                return Err("invalid extension matrix size".to_string());
            }
            let k = total / us.len();
            if !SUPPORTED_K.contains(&k) {
                return Err("invalid extension matrix size".to_string());
            }

            Ok(Extend { count, us })
        }
    }

    #[derive(Deserialize)]
    pub(super) struct CheckUnchecked {
        x: Gf2_128,
        t: Vec<Gf2_128>,
    }

    impl TryFrom<CheckUnchecked> for Check {
        type Error = String;

        fn try_from(value: CheckUnchecked) -> Result<Self, Self::Error> {
            let CheckUnchecked { x, t } = value;

            if t.len() != CSP {
                return Err("t length is invalid".to_string());
            }

            Ok(Check { x, t })
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        rcot::{RCOTReceiver, RCOTReceiverOutput, RCOTSender, RCOTSenderOutput},
        test::assert_cot,
    };

    use super::*;
    use itybity::ToBits;
    use rstest::*;

    use mpz_core::Block;

    use rand::Rng;
    use rand_chacha::ChaCha12Rng;
    use rand_core::SeedableRng;

    #[fixture]
    fn choices() -> Vec<bool> {
        let mut rng = ChaCha12Rng::seed_from_u64(0);
        (0..128).map(|_| rng.random()).collect()
    }

    #[fixture]
    fn data() -> Vec<[Block; 2]> {
        let mut rng = ChaCha12Rng::seed_from_u64(1);
        (0..128)
            .map(|_| {
                [
                    rng.random::<[u8; 16]>().into(),
                    rng.random::<[u8; 16]>().into(),
                ]
            })
            .collect()
    }

    #[fixture]
    fn delta() -> Block {
        let mut rng = ChaCha12Rng::seed_from_u64(2);
        rng.random::<[u8; 16]>().into()
    }

    #[fixture]
    fn receiver_seeds() -> [[Block; 2]; CSP] {
        let mut rng = ChaCha12Rng::seed_from_u64(3);
        std::array::from_fn(|_| [rng.random(), rng.random()])
    }

    #[fixture]
    fn sender_seeds(delta: Block, receiver_seeds: [[Block; 2]; CSP]) -> [Block; CSP] {
        delta
            .iter_lsb0()
            .zip(receiver_seeds)
            .map(|(b, seeds)| if b { seeds[1] } else { seeds[0] })
            .collect::<Vec<_>>()
            .try_into()
            .unwrap()
    }

    #[fixture]
    fn chi_seed() -> Block {
        let mut rng = ChaCha12Rng::seed_from_u64(4);
        rng.random::<[u8; 16]>().into()
    }

    #[fixture]
    fn expected(data: Vec<[Block; 2]>, choices: Vec<bool>) -> Vec<Block> {
        data.iter()
            .zip(choices.iter())
            .map(|([a, b], choice)| if *choice { *b } else { *a })
            .collect()
    }

    #[rstest]
    fn test_softspoken_extension(
        delta: Block,
        sender_seeds: [Block; CSP],
        receiver_seeds: [[Block; 2]; CSP],
    ) {
        let count = 128;

        let sender = Sender::new(SenderConfig::default(), delta);
        let receiver = Receiver::new(ReceiverConfig::default());

        let (mut receiver, corrections) = receiver.setup(receiver_seeds).corrections();
        let mut sender = sender.setup(sender_seeds).corrections(corrections);

        sender.alloc(count).unwrap();
        receiver.alloc(count).unwrap();

        assert!(sender.wants_extend());
        assert!(receiver.wants_extend());

        while receiver.wants_extend() {
            sender.extend(receiver.extend().unwrap()).unwrap();
        }

        assert!(!sender.wants_extend());
        assert!(!receiver.wants_extend());
        assert!(sender.wants_check());
        assert!(receiver.wants_check());

        let chi_seed = sender.check_start();
        let receiver_check = receiver.check(chi_seed).unwrap();
        sender.check(receiver_check).unwrap();

        assert_eq!(sender.available(), count);
        assert_eq!(receiver.available(), count);

        let RCOTSenderOutput {
            id: sender_id,
            keys,
        } = sender.try_send_rcot(count).unwrap();
        let RCOTReceiverOutput {
            id: receiver_id,
            choices,
            msgs,
        } = receiver.try_recv_rcot(count).unwrap();

        assert_eq!(sender_id, receiver_id);
        assert_cot(delta, &choices, &keys, &msgs);
    }

    #[rstest]
    fn test_softspoken_partial_consume(
        delta: Block,
        sender_seeds: [Block; CSP],
        receiver_seeds: [[Block; 2]; CSP],
    ) {
        let count = 512;

        let sender = Sender::new(SenderConfig::default(), delta);
        let receiver = Receiver::new(ReceiverConfig::default());

        let (mut receiver, corrections) = receiver.setup(receiver_seeds).corrections();
        let mut sender = sender.setup(sender_seeds).corrections(corrections);

        sender.alloc(count).unwrap();
        receiver.alloc(count).unwrap();

        while receiver.wants_extend() {
            sender.extend(receiver.extend().unwrap()).unwrap();
        }

        let chi_seed = sender.check_start();
        let receiver_check = receiver.check(chi_seed).unwrap();
        sender.check(receiver_check).unwrap();

        assert_eq!(sender.available(), count);
        assert_eq!(receiver.available(), count);

        let mut remaining = count;
        for chunk in [100usize, 300, 112] {
            let RCOTSenderOutput { id: sid, keys } = sender.try_send_rcot(chunk).unwrap();
            let RCOTReceiverOutput {
                id: rid,
                choices,
                msgs,
            } = receiver.try_recv_rcot(chunk).unwrap();

            assert_eq!(sid, rid);
            assert_cot(delta, &choices, &keys, &msgs);

            remaining -= chunk;
            assert_eq!(sender.available(), remaining);
            assert_eq!(receiver.available(), remaining);
        }
        assert_eq!(remaining, 0);
    }

    #[rstest]
    #[case::k2(2, 128)]
    #[case::k4(4, 128)]
    #[case::k8(8, 128)]
    #[case::k4_large(4, 4096 * 2 + 256)]
    #[case::k8_large(8, 4096 + 512)]
    fn test_softspoken_k(
        #[case] k: usize,
        #[case] count: usize,
        delta: Block,
        sender_seeds: [Block; CSP],
        receiver_seeds: [[Block; 2]; CSP],
    ) {
        let sender_config = SenderConfig::builder()
            .k(k)
            .batch_size(2048)
            .build()
            .unwrap();
        let receiver_config = ReceiverConfig::builder()
            .k(k)
            .batch_size(2048)
            .build()
            .unwrap();

        let sender = Sender::new(sender_config, delta);
        let receiver = Receiver::new(receiver_config);

        let (mut receiver, corrections) = receiver.setup(receiver_seeds).corrections();
        let mut sender = sender.setup(sender_seeds).corrections(corrections);

        sender.alloc(count).unwrap();
        receiver.alloc(count).unwrap();

        while receiver.wants_extend() {
            sender.extend(receiver.extend().unwrap()).unwrap();
        }

        let chi_seed = sender.check_start();
        let receiver_check = receiver.check(chi_seed).unwrap();
        sender.check(receiver_check).unwrap();

        assert_eq!(sender.available(), count);
        assert_eq!(receiver.available(), count);

        let RCOTSenderOutput { keys, .. } = sender.try_send_rcot(count).unwrap();
        let RCOTReceiverOutput { choices, msgs, .. } = receiver.try_recv_rcot(count).unwrap();

        assert_cot(delta, &choices, &keys, &msgs);
    }

    #[rstest]
    fn test_softspoken_extension_stream_extends(
        delta: Block,
        sender_seeds: [Block; CSP],
        receiver_seeds: [[Block; 2]; CSP],
    ) {
        let sender_config = SenderConfig::builder().batch_size(2048).build().unwrap();
        let receiver_config = ReceiverConfig::builder().batch_size(2048).build().unwrap();

        let count = sender_config.batch_size() * 3;

        let sender = Sender::new(sender_config, delta);
        let receiver = Receiver::new(receiver_config);

        let (mut receiver, corrections) = receiver.setup(receiver_seeds).corrections();
        let mut sender = sender.setup(sender_seeds).corrections(corrections);

        sender.alloc(count).unwrap();
        receiver.alloc(count).unwrap();

        assert!(sender.wants_extend());
        assert!(receiver.wants_extend());

        while receiver.wants_extend() {
            sender.extend(receiver.extend().unwrap()).unwrap();
        }

        assert!(!sender.wants_extend());
        assert!(!receiver.wants_extend());
        assert!(sender.wants_check());
        assert!(receiver.wants_check());

        let chi_seed = sender.check_start();
        let receiver_check = receiver.check(chi_seed).unwrap();
        sender.check(receiver_check).unwrap();

        assert_eq!(sender.available(), count);
        assert_eq!(receiver.available(), count);

        let RCOTSenderOutput {
            id: sender_id,
            keys,
        } = sender.try_send_rcot(count).unwrap();
        let RCOTReceiverOutput {
            id: receiver_id,
            choices,
            msgs,
        } = receiver.try_recv_rcot(count).unwrap();

        assert_eq!(sender_id, receiver_id);
        assert_cot(delta, &choices, &keys, &msgs);
    }

    #[rstest]
    #[case::equal_rounds(&[256, 256])]
    #[case::growing_rounds(&[128, 512, 2048])]
    #[case::shrinking_rounds(&[2048, 512, 128])]
    fn test_softspoken_multiple_rounds(
        #[case] rounds: &[usize],
        delta: Block,
        sender_seeds: [Block; CSP],
        receiver_seeds: [[Block; 2]; CSP],
    ) {
        let sender = Sender::new(SenderConfig::default(), delta);
        let receiver = Receiver::new(ReceiverConfig::default());

        let (mut receiver, corrections) = receiver.setup(receiver_seeds).corrections();
        let mut sender = sender.setup(sender_seeds).corrections(corrections);

        for &count in rounds {
            assert!(!sender.wants_extend());
            assert!(!receiver.wants_extend());

            sender.alloc(count).unwrap();
            receiver.alloc(count).unwrap();

            while receiver.wants_extend() {
                sender.extend(receiver.extend().unwrap()).unwrap();
            }

            let chi_seed = sender.check_start();
            let receiver_check = receiver.check(chi_seed).unwrap();
            sender.check(receiver_check).unwrap();

            assert_eq!(sender.available(), count);
            assert_eq!(receiver.available(), count);

            let RCOTSenderOutput { id: sid, keys } = sender.try_send_rcot(count).unwrap();
            let RCOTReceiverOutput {
                id: rid,
                choices,
                msgs,
            } = receiver.try_recv_rcot(count).unwrap();

            assert_eq!(sid, rid);
            assert_cot(delta, &choices, &keys, &msgs);
        }
    }

    #[rstest]
    fn test_softspoken_rounds_accumulate_without_consuming(
        delta: Block,
        sender_seeds: [Block; CSP],
        receiver_seeds: [[Block; 2]; CSP],
    ) {
        let sender = Sender::new(SenderConfig::default(), delta);
        let receiver = Receiver::new(ReceiverConfig::default());

        let (mut receiver, corrections) = receiver.setup(receiver_seeds).corrections();
        let mut sender = sender.setup(sender_seeds).corrections(corrections);

        let run_round = |sender: &mut Sender<_>, receiver: &mut Receiver<_>, count: usize| {
            sender.alloc(count).unwrap();
            receiver.alloc(count).unwrap();
            while receiver.wants_extend() {
                sender.extend(receiver.extend().unwrap()).unwrap();
            }
            let chi_seed = sender.check_start();
            let receiver_check = receiver.check(chi_seed).unwrap();
            sender.check(receiver_check).unwrap();
        };

        run_round(&mut sender, &mut receiver, 256);
        run_round(&mut sender, &mut receiver, 384);

        assert_eq!(sender.available(), 256 + 384);
        assert_eq!(receiver.available(), 256 + 384);

        for count in [200usize, 300, 140] {
            let RCOTSenderOutput { keys, .. } = sender.try_send_rcot(count).unwrap();
            let RCOTReceiverOutput { choices, msgs, .. } = receiver.try_recv_rcot(count).unwrap();
            assert_cot(delta, &choices, &keys, &msgs);
        }
        assert_eq!(sender.available(), 0);
        assert_eq!(receiver.available(), 0);
    }

    #[rstest]
    fn test_softspoken_extension_insufficient_setup(
        delta: Block,
        sender_seeds: [Block; CSP],
        receiver_seeds: [[Block; 2]; CSP],
    ) {
        let count = 128;

        let sender = Sender::new(SenderConfig::default(), delta);
        let receiver = Receiver::new(ReceiverConfig::default());

        let (mut receiver, corrections) = receiver.setup(receiver_seeds).corrections();
        let mut sender = sender.setup(sender_seeds).corrections(corrections);

        sender.alloc(count).unwrap();
        receiver.alloc(count).unwrap();

        while receiver.wants_extend() {
            sender.extend(receiver.extend().unwrap()).unwrap();
        }

        let chi_seed = sender.check_start();
        let receiver_check = receiver.check(chi_seed).unwrap();
        sender.check(receiver_check).unwrap();

        let err = sender.try_send_rcot(count + 1).unwrap_err();
        assert!(matches!(err, SenderError::InsufficientSetup { .. }));

        let err = receiver.try_recv_rcot(count + 1).unwrap_err();
        assert!(matches!(err, ReceiverError::InsufficientSetup { .. }));
    }

    #[rstest]
    fn test_softspoken_extension_bad_consistency_check(
        delta: Block,
        sender_seeds: [Block; CSP],
        receiver_seeds: [[Block; 2]; CSP],
    ) {
        let count = 128;

        let sender = Sender::new(SenderConfig::default(), delta);
        let receiver = Receiver::new(ReceiverConfig::default());

        let (mut receiver, corrections) = receiver.setup(receiver_seeds).corrections();
        let mut sender = sender.setup(sender_seeds).corrections(corrections);

        sender.alloc(count).unwrap();
        receiver.alloc(count).unwrap();

        while receiver.wants_extend() {
            let mut extend = receiver.extend().unwrap();

            *extend.us.first_mut().unwrap() ^= 1;

            sender.extend(extend).unwrap();
        }

        let chi_seed = sender.check_start();
        let receiver_check = receiver.check(chi_seed).unwrap();
        let err = sender.check(receiver_check).unwrap_err();

        assert!(matches!(err, SenderError::ConsistencyCheckFailed));
    }
}

#[cfg(test)]
mod validation_tests {
    use super::{CSP, Check, Corrections, Extend, TREE_CORRECTIONS};

    use mpz_fields::gf2_128::Gf2_128;

    #[test]
    fn extend_accepts_supported_k_and_rejects_bad_matrix() {
        for (k, us_len) in [(2usize, 1024usize), (4, 512), (8, 256)] {
            let bytes = bincode::serialize(&Extend {
                count: 128,
                us: vec![0u8; us_len],
            })
            .unwrap();
            assert!(bincode::deserialize::<Extend>(&bytes).is_ok(), "k={k}");
        }

        for us_len in [0usize, 257, 2048] {
            let bytes = bincode::serialize(&Extend {
                count: 128,
                us: vec![0u8; us_len],
            })
            .unwrap();
            assert!(
                bincode::deserialize::<Extend>(&bytes).is_err(),
                "us_len={us_len}"
            );
        }
    }

    #[test]
    fn extend_rejects_non_positive_multiple_of_csp_count() {
        for count in [0usize, 64, 200] {
            let bytes = bincode::serialize(&Extend {
                count,
                us: vec![0u8; 512],
            })
            .unwrap();
            assert!(
                bincode::deserialize::<Extend>(&bytes).is_err(),
                "count={count}"
            );
        }
    }

    #[test]
    fn corrections_validates_length() {
        let ok = Corrections {
            corrections: vec![[0u8; 16]; TREE_CORRECTIONS],
            s: [0u8; 16],
        };
        let bytes = bincode::serialize(&ok).unwrap();
        assert!(bincode::deserialize::<Corrections>(&bytes).is_ok());

        for len in [TREE_CORRECTIONS - 1, TREE_CORRECTIONS + 1] {
            let bad = Corrections {
                corrections: vec![[0u8; 16]; len],
                s: [0u8; 16],
            };
            let bytes = bincode::serialize(&bad).unwrap();
            assert!(
                bincode::deserialize::<Corrections>(&bytes).is_err(),
                "len={len}"
            );
        }
    }

    #[test]
    fn check_validates_t_length() {
        let ok = Check {
            x: Gf2_128::new(0),
            t: vec![Gf2_128::new(0); CSP],
        };
        let bytes = bincode::serialize(&ok).unwrap();
        assert!(bincode::deserialize::<Check>(&bytes).is_ok());

        let bad = Check {
            x: Gf2_128::new(0),
            t: vec![Gf2_128::new(0); CSP - 1],
        };
        let bytes = bincode::serialize(&bad).unwrap();
        assert!(bincode::deserialize::<Check>(&bytes).is_err());
    }
}
