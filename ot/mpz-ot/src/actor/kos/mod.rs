mod error;
pub mod msgs;
mod receiver;
mod sender;

use futures::{SinkExt, StreamExt};
use utils_aio::{sink::IoSink, stream::IoStream};

use crate::kos::msgs::Message as KosMessage;

pub use error::{ReceiverActorError, SenderActorError};
pub use receiver::{ReceiverActor, SharedReceiver};
pub use sender::{SenderActor, SharedSender};

/// Converts a sink of KOS actor messages into a sink of KOS messages.
pub(crate) fn into_kos_sink<'a, Si: IoSink<msgs::Message<T>> + Send + Unpin, T: Send + 'a>(
    sink: &'a mut Si,
) -> impl IoSink<KosMessage<T>> + Send + Unpin + 'a {
    Box::pin(SinkExt::with(sink, |msg| async move {
        Ok(msgs::Message::Protocol(msg))
    }))
}

/// Converts a stream of KOS actor messages into a stream of KOS messages.
pub(crate) fn into_kos_stream<'a, St: IoStream<msgs::Message<T>> + Send + Unpin, T: Send + 'a>(
    stream: &'a mut St,
) -> impl IoStream<KosMessage<T>> + Send + Unpin + 'a {
    StreamExt::map(stream, |msg| match msg {
        Ok(msg) => msg.try_into_protocol().map_err(From::from),
        Err(err) => Err(err),
    })
}

#[cfg(test)]
mod tests {
    use crate::{
        kos::{Receiver, Sender},
        mock::{mock_ot_pair, MockOTReceiver, MockOTSender},
        OTReceiverShared, OTSenderShared, VerifiableOTReceiverShared,
    };

    use msgs::Message;

    use super::*;
    use futures::{
        stream::{SplitSink, SplitStream},
        StreamExt,
    };
    use rstest::*;

    use mpz_core::Block;
    use mpz_ot_core::kos::{ReceiverConfig, SenderConfig};
    use rand::{Rng, SeedableRng};
    use rand_chacha::ChaCha12Rng;
    use utils_aio::duplex::MemoryDuplex;

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
        data: impl IntoIterator<Item = [T; 2]>,
        choices: impl IntoIterator<Item = bool>,
    ) -> impl Iterator<Item = T> {
        data.into_iter()
            .zip(choices)
            .map(|([zero, one], choice)| if choice { one } else { zero })
    }

    async fn setup(
        sender_config: SenderConfig,
        receiver_config: ReceiverConfig,
        count: usize,
    ) -> (
        SenderActor<
            MockOTReceiver<Block>,
            SplitSink<MemoryDuplex<Message<()>>, Message<()>>,
            SplitStream<MemoryDuplex<Message<()>>>,
        >,
        ReceiverActor<
            MockOTSender<Block>,
            SplitSink<MemoryDuplex<Message<()>>, Message<()>>,
            SplitStream<MemoryDuplex<Message<()>>>,
        >,
    ) {
        let (sender_channel, receiver_channel) = MemoryDuplex::new();

        let (sender_sink, sender_stream) = sender_channel.split();
        let (receiver_sink, receiver_stream) = receiver_channel.split();

        let (base_sender, base_receiver) = mock_ot_pair();

        let sender = Sender::new(sender_config, base_receiver);
        let receiver = Receiver::new(receiver_config, base_sender);

        let mut sender = SenderActor::new(sender, sender_sink, sender_stream);
        let mut receiver = ReceiverActor::new(receiver, receiver_sink, receiver_stream);

        let (sender_res, receiver_res) = tokio::join!(sender.setup(count), receiver.setup(count));

        sender_res.unwrap();
        receiver_res.unwrap();

        (sender, receiver)
    }

    #[rstest]
    #[tokio::test]
    async fn test_kos_actor(data: Vec<[Block; 2]>, choices: Vec<bool>) {
        let (mut sender_actor, mut receiver_actor) = setup(
            SenderConfig::default(),
            ReceiverConfig::default(),
            data.len(),
        )
        .await;

        let sender = sender_actor.sender();
        let receiver = receiver_actor.receiver();

        tokio::spawn(async move {
            sender_actor.run().await.unwrap();
            sender_actor
        });

        tokio::spawn(async move {
            receiver_actor.run().await.unwrap();
            receiver_actor
        });

        let (sender_res, receiver_res) = tokio::join!(
            sender.send("test", &data),
            receiver.receive("test", &choices)
        );

        sender_res.unwrap();
        let received_data: Vec<Block> = receiver_res.unwrap();

        let expected_data = choose(data, choices).collect::<Vec<_>>();

        assert_eq!(received_data, expected_data);
    }

    #[rstest]
    #[tokio::test]
    async fn test_kos_actor_verifiable_receiver(data: Vec<[Block; 2]>, choices: Vec<bool>) {
        let (mut sender_actor, mut receiver_actor) = setup(
            SenderConfig::builder().sender_commit().build().unwrap(),
            ReceiverConfig::builder().sender_commit().build().unwrap(),
            data.len(),
        )
        .await;

        let sender = sender_actor.sender();
        let receiver = receiver_actor.receiver();

        let sender_task = tokio::spawn(async move {
            sender_actor.run().await.unwrap();
            sender_actor
        });

        tokio::spawn(async move {
            receiver_actor.run().await.unwrap();
            receiver_actor
        });

        let (sender_res, receiver_res) = tokio::join!(
            sender.send("test", &data),
            receiver.receive("test", &choices)
        );

        sender_res.unwrap();

        let received_data: Vec<Block> = receiver_res.unwrap();

        let expected_data = choose(data.clone(), choices).collect::<Vec<_>>();

        assert_eq!(received_data, expected_data);

        sender.shutdown().await.unwrap();

        let mut sender_actor = sender_task.await.unwrap();

        sender_actor.reveal().await.unwrap();

        receiver.verify("test", &data).await.unwrap();
    }
}
