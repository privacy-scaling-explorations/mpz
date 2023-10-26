//! An implementation of the [`KOS15`](https://eprint.iacr.org/2015/546.pdf) oblivious transfer extension protocol.

mod error;
mod receiver;
mod sender;

pub use error::{ReceiverError, ReceiverVerifyError, SenderError};
use futures_util::{SinkExt, StreamExt};
pub use receiver::Receiver;
pub use sender::Sender;

pub(crate) use receiver::StateError as ReceiverStateError;
pub(crate) use sender::StateError as SenderStateError;

pub use mpz_ot_core::kos::{
    msgs, PayloadRecord, ReceiverConfig, ReceiverConfigBuilder, ReceiverConfigBuilderError,
    ReceiverKeys, SenderConfig, SenderConfigBuilder, SenderConfigBuilderError, SenderKeys,
};
use utils_aio::{sink::IoSink, stream::IoStream};

/// Converts a sink of KOS messages into a sink of base OT messages.
pub(crate) fn into_base_sink<'a, Si: IoSink<msgs::Message<T>> + Send + Unpin, T: Send + 'a>(
    sink: &'a mut Si,
) -> impl IoSink<T> + Send + Unpin + 'a {
    Box::pin(SinkExt::with(sink, |msg| async move {
        Ok(msgs::Message::BaseMsg(msg))
    }))
}

/// Converts a stream of KOS messages into a stream of base OT messages.
pub(crate) fn into_base_stream<'a, St: IoStream<msgs::Message<T>> + Send + Unpin, T: Send + 'a>(
    stream: &'a mut St,
) -> impl IoStream<T> + Send + Unpin + 'a {
    StreamExt::map(stream, |msg| match msg {
        Ok(msg) => msg.try_into_base_msg().map_err(From::from),
        Err(err) => Err(err),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;
    use rstest::*;

    use itybity::ToBits;
    use mpz_core::Block;
    use mpz_ot_core::kos::msgs::Message;
    use rand::Rng;
    use rand_chacha::ChaCha12Rng;
    use rand_core::SeedableRng;
    use utils_aio::{duplex::MemoryDuplex, sink::IoSink, stream::IoStream};

    use crate::{
        mock::{mock_ot_pair, MockOTReceiver, MockOTSender},
        OTReceiver, OTSender, OTSetup, VerifiableOTReceiver,
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

    async fn setup<
        Si: IoSink<Message<()>> + Send + Unpin,
        St: IoStream<Message<()>> + Send + Unpin,
    >(
        sender_config: SenderConfig,
        receiver_config: ReceiverConfig,
        sender_sink: &mut Si,
        sender_stream: &mut St,
        receiver_sink: &mut Si,
        receiver_stream: &mut St,
        count: usize,
    ) -> (Sender<MockOTReceiver<Block>>, Receiver<MockOTSender<Block>>) {
        let (base_sender, base_receiver) = mock_ot_pair();

        let mut sender = Sender::new(sender_config, base_receiver);
        let mut receiver = Receiver::new(receiver_config, base_sender);

        let (sender_res, receiver_res) = tokio::join!(
            sender.setup(sender_sink, sender_stream),
            receiver.setup(receiver_sink, receiver_stream)
        );

        sender_res.unwrap();
        receiver_res.unwrap();

        let (sender_res, receiver_res) = tokio::join!(
            sender.extend(sender_sink, sender_stream, count),
            receiver.extend(receiver_sink, receiver_stream, count)
        );

        sender_res.unwrap();
        receiver_res.unwrap();

        (sender, receiver)
    }

    #[rstest]
    #[tokio::test]
    async fn test_kos(data: Vec<[Block; 2]>, choices: Vec<bool>) {
        let (sender_channel, receiver_channel) = MemoryDuplex::new();

        let (mut sender_sink, mut sender_stream) = sender_channel.split();
        let (mut receiver_sink, mut receiver_stream) = receiver_channel.split();

        let (mut sender, mut receiver) = setup(
            SenderConfig::default(),
            ReceiverConfig::default(),
            &mut sender_sink,
            &mut sender_stream,
            &mut receiver_sink,
            &mut receiver_stream,
            data.len(),
        )
        .await;

        let (sender_res, receiver_res) = tokio::join!(
            sender.send(&mut sender_sink, &mut sender_stream, &data),
            receiver.receive(&mut receiver_sink, &mut receiver_stream, &choices)
        );

        sender_res.unwrap();
        let received: Vec<Block> = receiver_res.unwrap();

        let expected = choose(data.iter().copied(), choices.iter_lsb0()).collect::<Vec<_>>();

        assert_eq!(received, expected);
    }

    #[rstest]
    #[tokio::test]
    async fn test_kos_bytes(data: Vec<[Block; 2]>, choices: Vec<bool>) {
        let (sender_channel, receiver_channel) = MemoryDuplex::new();

        let (mut sender_sink, mut sender_stream) = sender_channel.split();
        let (mut receiver_sink, mut receiver_stream) = receiver_channel.split();

        let (mut sender, mut receiver) = setup(
            SenderConfig::default(),
            ReceiverConfig::default(),
            &mut sender_sink,
            &mut sender_stream,
            &mut receiver_sink,
            &mut receiver_stream,
            data.len(),
        )
        .await;

        let data: Vec<_> = data
            .into_iter()
            .map(|[a, b]| [a.to_bytes(), b.to_bytes()])
            .collect();

        let (sender_res, receiver_res) = tokio::join!(
            sender.send(&mut sender_sink, &mut sender_stream, &data),
            receiver.receive(&mut receiver_sink, &mut receiver_stream, &choices)
        );

        sender_res.unwrap();
        let received: Vec<[u8; 16]> = receiver_res.unwrap();

        let expected = choose(data.iter().copied(), choices.iter_lsb0()).collect::<Vec<_>>();

        assert_eq!(received, expected);
    }

    #[rstest]
    #[tokio::test]
    async fn test_kos_committed_sender(data: Vec<[Block; 2]>, choices: Vec<bool>) {
        let (sender_channel, receiver_channel) = MemoryDuplex::new();

        let (mut sender_sink, mut sender_stream) = sender_channel.split();
        let (mut receiver_sink, mut receiver_stream) = receiver_channel.split();

        let (mut sender, mut receiver) = setup(
            SenderConfig::builder().sender_commit().build().unwrap(),
            ReceiverConfig::builder().sender_commit().build().unwrap(),
            &mut sender_sink,
            &mut sender_stream,
            &mut receiver_sink,
            &mut receiver_stream,
            data.len(),
        )
        .await;

        let (sender_res, receiver_res) = tokio::join!(
            sender.send(&mut sender_sink, &mut sender_stream, &data),
            receiver.receive(&mut receiver_sink, &mut receiver_stream, &choices)
        );

        sender_res.unwrap();
        let received: Vec<Block> = receiver_res.unwrap();

        let expected = choose(data.iter().copied(), choices.iter_lsb0()).collect::<Vec<_>>();

        assert_eq!(received, expected);

        let (sender_res, receiver_res) = tokio::join!(
            sender.reveal(&mut sender_sink, &mut sender_stream),
            receiver.verify(&mut receiver_sink, &mut receiver_stream, 0, &data)
        );

        sender_res.unwrap();
        receiver_res.unwrap();
    }
}
