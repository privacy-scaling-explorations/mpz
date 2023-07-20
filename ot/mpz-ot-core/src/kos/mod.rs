//! An implementation of the [`KOS15`](https://eprint.iacr.org/2015/546.pdf) oblivious transfer extension protocol.

mod config;
mod error;
pub mod msgs;
mod receiver;
mod sender;

pub use config::{
    ReceiverConfig, ReceiverConfigBuilder, ReceiverConfigBuilderError, SenderConfig,
    SenderConfigBuilder, SenderConfigBuilderError,
};
pub use error::{ReceiverError, ReceiverVerifyError, SenderError};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
pub use receiver::{state as receiver_state, Receiver};
pub use sender::{state as sender_state, Keys, Sender};

/// Computational security parameter
pub const CSP: usize = 128;
/// Statistical security parameter
pub const SSP: usize = 128;
/// Rng to use for secret sharing the IKNP matrix.
pub(crate) type Rng = ChaCha20Rng;
/// Rng seed type
pub(crate) type RngSeed = <Rng as SeedableRng>::Seed;

#[cfg(test)]
mod tests {
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
        (0..128).map(|_| rng.gen()).collect()
    }

    #[fixture]
    fn data() -> Vec<[Block; 2]> {
        let mut rng = ChaCha12Rng::seed_from_u64(1);
        (0..128)
            .map(|_| [rng.gen::<[u8; 16]>().into(), rng.gen::<[u8; 16]>().into()])
            .collect()
    }

    #[fixture]
    fn delta() -> Block {
        let mut rng = ChaCha12Rng::seed_from_u64(2);
        rng.gen::<[u8; 16]>().into()
    }

    #[fixture]
    fn receiver_seeds() -> [[Block; 2]; CSP] {
        let mut rng = ChaCha12Rng::seed_from_u64(3);
        std::array::from_fn(|_| [rng.gen(), rng.gen()])
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
        rng.gen::<[u8; 16]>().into()
    }

    #[fixture]
    fn expected(data: Vec<[Block; 2]>, choices: Vec<bool>) -> Vec<Block> {
        data.iter()
            .zip(choices.iter())
            .map(|([a, b], choice)| if *choice { *b } else { *a })
            .collect()
    }

    #[rstest]
    fn test_kos_extension(
        delta: Block,
        sender_seeds: [Block; CSP],
        receiver_seeds: [[Block; 2]; CSP],
        chi_seed: Block,
        choices: Vec<bool>,
        data: Vec<[Block; 2]>,
        expected: Vec<Block>,
    ) {
        let sender = Sender::new(SenderConfig::default());
        let receiver = Receiver::new(ReceiverConfig::default());

        let mut sender = sender.base_setup(delta, sender_seeds);
        let mut receiver = receiver.base_setup(receiver_seeds);

        let receiver_setup = receiver.extend(choices.len() + 256);
        sender.extend(data.len() + 256, receiver_setup).unwrap();

        let receiver_check = receiver.check(chi_seed);
        sender.check(chi_seed, receiver_check).unwrap();

        let derandomize = receiver.derandomize(&choices);
        let payload = sender.send(&data, derandomize).unwrap();
        let received = receiver.receive(payload).unwrap();

        assert_eq!(received, expected);
    }

    #[rstest]
    fn test_kos_extension_multiple_extends(
        delta: Block,
        sender_seeds: [Block; CSP],
        receiver_seeds: [[Block; 2]; CSP],
        chi_seed: Block,
        mut choices: Vec<bool>,
        mut data: Vec<[Block; 2]>,
        mut expected: Vec<Block>,
    ) {
        let sender = Sender::new(SenderConfig::default());
        let receiver = Receiver::new(ReceiverConfig::default());

        let mut sender = sender.base_setup(delta, sender_seeds);
        let mut receiver = receiver.base_setup(receiver_seeds);

        let receiver_setup = receiver.extend(choices.len() + 256);
        sender.extend(data.len() + 256, receiver_setup).unwrap();

        let more_choices = choices[..7].to_vec();
        let more_data = data[..7].to_vec();
        let more_expected = expected[..7].to_vec();

        let receiver_setup = receiver.extend(7);
        sender.extend(7, receiver_setup).unwrap();

        let receiver_check = receiver.check(chi_seed);
        sender.check(chi_seed, receiver_check).unwrap();

        choices.extend(more_choices);
        data.extend(more_data);
        expected.extend(more_expected);

        let derandomize = receiver.derandomize(&choices);
        let payload = sender.send(&data, derandomize).unwrap();
        let received = receiver.receive(payload).unwrap();

        assert_eq!(received, expected);
    }

    #[rstest]
    fn test_kos_extension_bad_consistency_check(
        delta: Block,
        sender_seeds: [Block; CSP],
        receiver_seeds: [[Block; 2]; CSP],
        chi_seed: Block,
        choices: Vec<bool>,
        data: Vec<[Block; 2]>,
    ) {
        let sender = Sender::new(SenderConfig::default());
        let receiver = Receiver::new(ReceiverConfig::default());

        let mut sender = sender.base_setup(delta, sender_seeds);
        let mut receiver = receiver.base_setup(receiver_seeds);

        let mut receiver_setup = receiver.extend(choices.len() + 256);

        // Flip a bit in the receiver's extension message (breaking the mono-chrome choice vector)
        *receiver_setup.us.first_mut().unwrap() ^= 1;

        sender.extend(data.len() + 256, receiver_setup).unwrap();

        let receiver_check = receiver.check(chi_seed);
        let err = sender.check(chi_seed, receiver_check).unwrap_err();

        assert!(matches!(err, SenderError::ConsistencyCheckFailed));
    }

    #[rstest]
    fn test_kos_extension_verify_messages(
        delta: Block,
        sender_seeds: [Block; CSP],
        receiver_seeds: [[Block; 2]; CSP],
        chi_seed: Block,
        choices: Vec<bool>,
        data: Vec<[Block; 2]>,
        expected: Vec<Block>,
    ) {
        let sender = Sender::new(SenderConfig::default());
        let receiver = Receiver::new(ReceiverConfig::builder().sender_commit().build().unwrap());

        let mut sender = sender.base_setup(delta, sender_seeds);
        let mut receiver = receiver.base_setup(receiver_seeds);

        let receiver_setup = receiver.extend(choices.len() + 256);
        sender.extend(data.len() + 256, receiver_setup).unwrap();

        let receiver_check = receiver.check(chi_seed);
        sender.check(chi_seed, receiver_check).unwrap();

        let derandomize = receiver.derandomize(&choices);
        let payload = sender.send(&data, derandomize).unwrap();
        let received = receiver.receive(payload).unwrap();

        assert_eq!(received, expected);

        receiver.verify(0, delta, &data).unwrap();
    }

    #[rstest]
    fn test_kos_extension_verify_messages_fail(
        delta: Block,
        sender_seeds: [Block; CSP],
        receiver_seeds: [[Block; 2]; CSP],
        chi_seed: Block,
        choices: Vec<bool>,
        mut data: Vec<[Block; 2]>,
        expected: Vec<Block>,
    ) {
        let sender = Sender::new(SenderConfig::default());
        let receiver = Receiver::new(ReceiverConfig::builder().sender_commit().build().unwrap());

        let mut sender = sender.base_setup(delta, sender_seeds);
        let mut receiver = receiver.base_setup(receiver_seeds);

        let receiver_setup = receiver.extend(choices.len() + 256);
        sender.extend(data.len() + 256, receiver_setup).unwrap();

        let receiver_check = receiver.check(chi_seed);
        sender.check(chi_seed, receiver_check).unwrap();

        let derandomize = receiver.derandomize(&choices);
        let payload = sender.send(&data, derandomize).unwrap();
        let received = receiver.receive(payload).unwrap();

        assert_eq!(received, expected);

        data[0][0] = Block::default();

        let err = receiver.verify(0, delta, &data).unwrap_err();

        assert!(matches!(
            err,
            ReceiverError::ReceiverVerifyError(ReceiverVerifyError::InconsistentPayload)
        ));
    }
}
