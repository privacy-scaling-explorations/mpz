//! An implementation of the Chou-Orlandi [`CO15`](https://eprint.iacr.org/2015/267.pdf) oblivious transfer protocol.
//!
//! # Examples
//!
//! ```
//! use utils_aio::duplex::MemoryDuplex;
//! use mpz_ot::chou_orlandi::{Receiver, Sender, SenderConfig, ReceiverConfig};
//! use mpz_ot::{OTReceiver, OTSender, OTSetup};
//! use mpz_core::Block;
//! use futures::StreamExt;
//!
//! # futures::executor::block_on(async {
//! // An in-memory duplex channel.
//! let (sender_channel, receiver_channel) = MemoryDuplex::new();
//!
//! let mut sender = Sender::new(SenderConfig::default(), sender_channel);
//! let mut receiver = Receiver::new(ReceiverConfig::default(), receiver_channel);
//!
//! // Perform the setup phase.
//! let (sender_res, receiver_res) = futures::join!(
//!     sender.setup(),
//!     receiver.setup()
//! );
//!
//! sender_res.unwrap();
//! receiver_res.unwrap();
//!
//! // Perform the transfer phase.
//! let messages = vec![[Block::ZERO, Block::ONES], [Block::ZERO, Block::ONES]];
//!
//! let (sender_res, receiver_res) = futures::join!(
//!     sender.send(&messages),
//!     receiver.receive(&[true, false])
//! );
//!
//! sender_res.unwrap();
//!
//! let received = receiver_res.unwrap();
//!
//! assert_eq!(received, vec![Block::ONES, Block::ZERO]);
//! # });
//! ```
//!
//! # Committed Receiver
//!
//! This implementation also provides support for a committed receiver. This is a receiver that commits to their choice
//! bits, and can later provably reveal them to the sender.
//!
//! ## Example
//!
//! ```
//! use utils_aio::duplex::MemoryDuplex;
//! use mpz_ot::chou_orlandi::{Receiver, Sender, SenderConfig, ReceiverConfig};
//! use mpz_ot::{OTReceiver, OTSender, CommittedOTReceiver, VerifiableOTSender, OTSetup};
//! use mpz_core::Block;
//! use futures::StreamExt;
//!
//! # futures::executor::block_on(async {
//! // An in-memory duplex channel.
//! let (sender_channel, receiver_channel) = MemoryDuplex::new();
//!
//! // Enable committed receiver in config.
//! let mut sender = Sender::new(
//!     SenderConfig::builder().receiver_commit().build().unwrap(),
//!     sender_channel
//! );
//!
//! let mut receiver = Receiver::new(
//!     ReceiverConfig::builder().receiver_commit().build().unwrap(),
//!     receiver_channel
//! );
//!
//! // Perform the setup phase.
//! let (sender_res, receiver_res) = futures::join!(
//!     sender.setup(),
//!     receiver.setup()
//! );
//!
//! sender_res.unwrap();
//! receiver_res.unwrap();
//!
//! // Perform the transfer phase.
//! let messages = vec![[Block::ZERO, Block::ONES], [Block::ZERO, Block::ONES]];
//!
//! let (sender_res, receiver_res) = futures::join!(
//!     sender.send(&messages),
//!     receiver.receive(&[true, false])
//! );
//!
//! sender_res.unwrap();
//! _ = receiver_res.unwrap();
//!
//! // Reveal the choice bits.
//! let (sender_res, receiver_res) = futures::join!(
//!     sender.verify_choices(),
//!     receiver.reveal_choices()
//! );
//!
//! receiver_res.unwrap();
//!
//! // The verified choice bits are returned to the sender.
//! let choices = sender_res.unwrap();
//!
//! assert_eq!(choices, vec![true, false]);
//! # });
//! ```

mod error;
mod receiver;
mod sender;

pub use error::{ReceiverError, SenderError};
pub use receiver::Receiver;
pub use sender::Sender;

pub use mpz_ot_core::chou_orlandi::{
    msgs, ReceiverConfig, ReceiverConfigBuilder, ReceiverConfigBuilderError, SenderConfig,
    SenderConfigBuilder, SenderConfigBuilderError,
};

#[cfg(test)]
mod tests {
    use itybity::ToBits;
    use mpz_core::Block;
    use mpz_ot_core::chou_orlandi::msgs::Message;
    use rand::Rng;
    use rand_chacha::ChaCha12Rng;
    use rand_core::SeedableRng;
    use utils_aio::duplex::MemoryDuplex;

    use crate::{CommittedOTReceiver, OTReceiver, OTSender, OTSetup, VerifiableOTSender};

    use super::*;
    use rstest::*;

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
    ) -> (
        Sender<MemoryDuplex<Message>>,
        Receiver<MemoryDuplex<Message>>,
    ) {
        let (sender_channel, receiver_channel) = MemoryDuplex::new();
        let mut sender = Sender::new(sender_config, sender_channel);
        let mut receiver = Receiver::new(receiver_config, receiver_channel);

        let (sender_res, receiver_res) = tokio::join!(sender.setup(), receiver.setup());

        sender_res.unwrap();
        receiver_res.unwrap();

        (sender, receiver)
    }

    #[rstest]
    #[tokio::test]
    async fn test_chou_orlandi(data: Vec<[Block; 2]>, choices: Vec<bool>) {
        let (mut sender, mut receiver) =
            setup(SenderConfig::default(), ReceiverConfig::default()).await;

        let (sender_res, receiver_res) =
            tokio::join!(sender.send(&data), receiver.receive(&choices));

        sender_res.unwrap();
        let received = receiver_res.unwrap();

        let expected = choose(data.iter().copied(), choices.iter_lsb0()).collect::<Vec<_>>();

        assert_eq!(received, expected);
    }

    #[rstest]
    #[tokio::test]
    async fn test_chou_orlandi_committed_receiver(data: Vec<[Block; 2]>, choices: Vec<bool>) {
        let (mut sender, mut receiver) = setup(
            SenderConfig::builder().receiver_commit().build().unwrap(),
            ReceiverConfig::builder().receiver_commit().build().unwrap(),
        )
        .await;

        let (sender_res, receiver_res) =
            tokio::join!(sender.send(&data), receiver.receive(&choices));

        sender_res.unwrap();
        _ = receiver_res.unwrap();

        let (sender_res, receiver_res) =
            tokio::join!(sender.verify_choices(), receiver.reveal_choices());

        let verified_choices = sender_res.unwrap();
        receiver_res.unwrap();

        assert_eq!(verified_choices, choices);
    }
}
