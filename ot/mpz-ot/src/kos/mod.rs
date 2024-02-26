//! An implementation of the [`KOS15`](https://eprint.iacr.org/2015/546.pdf) oblivious transfer extension protocol.

mod error;
mod receiver;
mod sender;

pub use error::{ReceiverError, ReceiverVerifyError, SenderError};
pub use receiver::Receiver;
pub use sender::Sender;

pub(crate) use receiver::StateError as ReceiverStateError;
pub(crate) use sender::StateError as SenderStateError;

pub use mpz_ot_core::kos::{
    msgs, PayloadRecord, ReceiverConfig, ReceiverConfigBuilder, ReceiverConfigBuilderError,
    ReceiverKeys, SenderConfig, SenderConfigBuilder, SenderConfigBuilderError, SenderKeys,
};

// If we're testing we use a smaller chunk size to make sure the chunking code paths are tested.
cfg_if::cfg_if! {
    if #[cfg(test)] {
        pub(crate) const EXTEND_CHUNK_SIZE: usize = 1024;
    } else {
        /// The size of the chunks used to send the extension matrix, 4MB.
        pub(crate) const EXTEND_CHUNK_SIZE: usize = 4 * 1024 * 1024;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::*;

    use itybity::ToBits;
    use mpz_core::Block;
    use mpz_ot_core::kos::msgs::Message;
    use rand::Rng;
    use rand_chacha::ChaCha12Rng;
    use rand_core::SeedableRng;
    use utils_aio::duplex::MemoryDuplex;

    use crate::{
        ideal::{ideal_ot_pair, IdealOTReceiver, IdealOTSender},
        OTReceiver, OTSender, OTSetup, RandomOTReceiver, RandomOTSender, VerifiableOTReceiver,
    };

    #[fixture]
    fn choices() -> Vec<bool> {
        let mut rng = ChaCha12Rng::seed_from_u64(0);
        (0..128).map(|_| rng.gen()).collect()
    }

    #[fixture]
    fn data() -> Vec<[Block; 2]> {
        let mut rng = ChaCha12Rng::seed_from_u64(0);
        (0..128)
            .map(|_| [rng.gen::<[u8; 16]>().into(), rng.gen::<[u8; 16]>().into()])
            .collect()
    }

    fn choose<T>(
        data: impl Iterator<Item = [T; 2]>,
        choices: impl Iterator<Item = bool>,
    ) -> impl Iterator<Item = T> {
        data.zip(choices)
            .map(|([zero, one], choice)| if choice { one } else { zero })
    }

    async fn setup(
        sender_config: SenderConfig,
        receiver_config: ReceiverConfig,
        count: usize,
    ) -> (
        Sender<MemoryDuplex<Message>, IdealOTReceiver<Block>>,
        Receiver<MemoryDuplex<Message>, IdealOTSender<Block>>,
    ) {
        let (sender_channel, receiver_channel) = MemoryDuplex::new();

        let (base_sender, base_receiver) = ideal_ot_pair();

        let mut sender = Sender::new(sender_config, sender_channel, base_receiver);
        let mut receiver = Receiver::new(receiver_config, receiver_channel, base_sender);

        let (sender_res, receiver_res) = tokio::join!(sender.setup(), receiver.setup());

        sender_res.unwrap();
        receiver_res.unwrap();

        let (sender_res, receiver_res) = tokio::join!(sender.extend(count), receiver.extend(count));

        sender_res.unwrap();
        receiver_res.unwrap();

        (sender, receiver)
    }

    #[rstest]
    #[tokio::test]
    async fn test_kos(data: Vec<[Block; 2]>, choices: Vec<bool>) {
        let (mut sender, mut receiver) = setup(
            SenderConfig::default(),
            ReceiverConfig::default(),
            data.len(),
        )
        .await;

        let (sender_res, receiver_res) =
            tokio::join!(sender.send(&data), receiver.receive(&choices));

        sender_res.unwrap();
        let received: Vec<Block> = receiver_res.unwrap();

        let expected = choose(data.iter().copied(), choices.iter_lsb0()).collect::<Vec<_>>();

        assert_eq!(received, expected);
    }

    #[tokio::test]
    async fn test_kos_random() {
        let (mut sender, mut receiver) =
            setup(SenderConfig::default(), ReceiverConfig::default(), 10).await;

        let (sender_res, receiver_res) = tokio::join!(
            RandomOTSender::send_random(&mut sender, 10),
            RandomOTReceiver::receive_random(&mut receiver, 10)
        );

        let sender_output: Vec<[Block; 2]> = sender_res.unwrap();
        let (choices, receiver_output): (Vec<bool>, Vec<Block>) = receiver_res.unwrap();

        let expected = sender_output
            .into_iter()
            .zip(choices)
            .map(|(output, choice)| output[choice as usize])
            .collect::<Vec<_>>();

        assert_eq!(receiver_output, expected);
    }

    #[rstest]
    #[tokio::test]
    async fn test_kos_bytes(data: Vec<[Block; 2]>, choices: Vec<bool>) {
        let (mut sender, mut receiver) = setup(
            SenderConfig::default(),
            ReceiverConfig::default(),
            data.len(),
        )
        .await;

        let data: Vec<_> = data
            .into_iter()
            .map(|[a, b]| [a.to_bytes(), b.to_bytes()])
            .collect();

        let (sender_res, receiver_res) =
            tokio::join!(sender.send(&data), receiver.receive(&choices));

        sender_res.unwrap();
        let received: Vec<[u8; 16]> = receiver_res.unwrap();

        let expected = choose(data.iter().copied(), choices.iter_lsb0()).collect::<Vec<_>>();

        assert_eq!(received, expected);
    }

    #[rstest]
    #[tokio::test]
    async fn test_kos_committed_sender(data: Vec<[Block; 2]>, choices: Vec<bool>) {
        let (mut sender, mut receiver) = setup(
            SenderConfig::builder().sender_commit().build().unwrap(),
            ReceiverConfig::builder().sender_commit().build().unwrap(),
            data.len(),
        )
        .await;

        let (sender_res, receiver_res) =
            tokio::join!(sender.send(&data), receiver.receive(&choices));

        sender_res.unwrap();
        let received: Vec<Block> = receiver_res.unwrap();

        let expected = choose(data.iter().copied(), choices.iter_lsb0()).collect::<Vec<_>>();

        assert_eq!(received, expected);

        let (sender_res, receiver_res) = tokio::join!(sender.reveal(), receiver.verify(0, &data));

        sender_res.unwrap();
        receiver_res.unwrap();
    }
}
