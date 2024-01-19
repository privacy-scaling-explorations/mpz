use crate::{OTError, OTSetup, RandomOTReceiver, RandomOTSender};
use async_trait::async_trait;
use futures::{channel::mpsc, StreamExt};
use mpz_core::{prg::Prg, Block, ProtocolMessage};
use rand::Rng;
use rand_chacha::ChaCha12Rng;
use rand_core::{RngCore, SeedableRng};
use utils_aio::{sink::IoSink, stream::IoStream};

/// Ideal random OT sender.
#[derive(Debug)]
pub struct IdealRandomOTSender<T = Block> {
    sender: mpsc::Sender<Vec<[T; 2]>>,
    rng: ChaCha12Rng,
}

/// Ideal random OT receiver.
#[derive(Debug)]
pub struct IdealRandomOTReceiver<T = Block> {
    receiver: mpsc::Receiver<Vec<[T; 2]>>,
    rng: ChaCha12Rng,
}

impl<T> ProtocolMessage for IdealRandomOTSender<T> {
    type Msg = ();
}

impl<T> ProtocolMessage for IdealRandomOTReceiver<T> {
    type Msg = ();
}

/// Creates a pair of ideal random OT sender and receiver.
pub fn ideal_random_ot_pair<T: Send + Sync + 'static>(
    seed: [u8; 32],
) -> (IdealRandomOTSender<T>, IdealRandomOTReceiver<T>) {
    let (sender, receiver) = mpsc::channel(10);

    (
        IdealRandomOTSender {
            sender,
            rng: ChaCha12Rng::from_seed(seed),
        },
        IdealRandomOTReceiver {
            receiver,
            rng: ChaCha12Rng::from_seed(seed),
        },
    )
}

#[async_trait]
impl<T> OTSetup for IdealRandomOTSender<T>
where
    T: Send + Sync,
{
    async fn setup<Si: IoSink<()> + Send + Unpin, St: IoStream<()> + Send + Unpin>(
        &mut self,
        _sink: &mut Si,
        _stream: &mut St,
    ) -> Result<(), OTError> {
        Ok(())
    }
}

#[async_trait]
impl RandomOTSender<[Block; 2]> for IdealRandomOTSender<Block> {
    async fn send_random<Si: IoSink<()> + Send + Unpin, St: IoStream<()> + Send + Unpin>(
        &mut self,
        _sink: &mut Si,
        _stream: &mut St,
        count: usize,
    ) -> Result<Vec<[Block; 2]>, OTError> {
        let messages = (0..count)
            .map(|_| [Block::random(&mut self.rng), Block::random(&mut self.rng)])
            .collect::<Vec<_>>();

        self.sender
            .try_send(messages.clone())
            .expect("IdealRandomOTSender should be able to send");

        Ok(messages)
    }
}

#[async_trait]
impl<const N: usize> RandomOTSender<[[u8; N]; 2]> for IdealRandomOTSender<[u8; N]> {
    async fn send_random<Si: IoSink<()> + Send + Unpin, St: IoStream<()> + Send + Unpin>(
        &mut self,
        _sink: &mut Si,
        _stream: &mut St,
        count: usize,
    ) -> Result<Vec<[[u8; N]; 2]>, OTError> {
        let prng = |block| {
            let mut prg = Prg::from_seed(block);
            let mut out = [0_u8; N];
            prg.fill_bytes(&mut out);
            out
        };

        let messages = (0..count)
            .map(|_| {
                [
                    prng(Block::random(&mut self.rng)),
                    prng(Block::random(&mut self.rng)),
                ]
            })
            .collect::<Vec<_>>();

        self.sender
            .try_send(messages.clone())
            .expect("IdealRandomOTSender should be able to send");

        Ok(messages)
    }
}

#[async_trait]
impl<T> OTSetup for IdealRandomOTReceiver<T>
where
    T: Send + Sync,
{
    async fn setup<Si: IoSink<()> + Send + Unpin, St: IoStream<()> + Send + Unpin>(
        &mut self,
        _sink: &mut Si,
        _stream: &mut St,
    ) -> Result<(), OTError> {
        Ok(())
    }
}

#[async_trait]
impl RandomOTReceiver<bool, Block> for IdealRandomOTReceiver<Block> {
    async fn receive_random<Si: IoSink<()> + Send + Unpin, St: IoStream<()> + Send + Unpin>(
        &mut self,
        _sink: &mut Si,
        _stream: &mut St,
        count: usize,
    ) -> Result<(Vec<bool>, Vec<Block>), OTError> {
        let payload = self
            .receiver
            .next()
            .await
            .expect("IdealRandomOTSender should send a value");

        assert_eq!(payload.len(), count);

        let choices = (0..count).map(|_| self.rng.gen()).collect::<Vec<bool>>();
        let payload = payload
            .into_iter()
            .zip(&choices)
            .map(|(v, c)| {
                let [low, high] = v;
                if *c {
                    high
                } else {
                    low
                }
            })
            .collect();

        Ok((choices, payload))
    }
}

#[async_trait]
impl<const N: usize> RandomOTReceiver<bool, [u8; N]> for IdealRandomOTReceiver<[u8; N]> {
    async fn receive_random<Si: IoSink<()> + Send + Unpin, St: IoStream<()> + Send + Unpin>(
        &mut self,
        _sink: &mut Si,
        _stream: &mut St,
        count: usize,
    ) -> Result<(Vec<bool>, Vec<[u8; N]>), OTError> {
        let payload = self
            .receiver
            .next()
            .await
            .expect("IdealRandomOTSender should send a value");

        assert_eq!(payload.len(), count);

        let choices = (0..count).map(|_| self.rng.gen()).collect::<Vec<bool>>();
        let payload = payload
            .into_iter()
            .zip(&choices)
            .map(|(v, c)| {
                let [low, high] = v;
                if *c {
                    high
                } else {
                    low
                }
            })
            .collect();

        Ok((choices, payload))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use utils_aio::duplex::MemoryDuplex;

    #[tokio::test]
    async fn test_ideal_random_ot_owned_block() {
        let seed = [0u8; 32];
        let (send_channel, recv_channel) = MemoryDuplex::<()>::new();

        let (mut send_sink, mut send_stream) = send_channel.split();
        let (mut recv_sink, mut recv_stream) = recv_channel.split();

        let (mut sender, mut receiver) = ideal_random_ot_pair::<Block>(seed);

        let values = RandomOTSender::send_random(&mut sender, &mut send_sink, &mut send_stream, 8)
            .await
            .unwrap();

        let (choices, received) =
            RandomOTReceiver::receive_random(&mut receiver, &mut recv_sink, &mut recv_stream, 8)
                .await
                .unwrap();

        let expected = values
            .into_iter()
            .zip(choices)
            .map(|(v, c)| v[c as usize])
            .collect::<Vec<_>>();

        assert_eq!(received, expected);
    }

    #[tokio::test]
    async fn test_ideal_random_ot_owned_array() {
        let seed = [0u8; 32];
        let (send_channel, recv_channel) = MemoryDuplex::<()>::new();

        let (mut send_sink, mut send_stream) = send_channel.split();
        let (mut recv_sink, mut recv_stream) = recv_channel.split();

        let (mut sender, mut receiver) = ideal_random_ot_pair::<[u8; 64]>(seed);

        let values = RandomOTSender::send_random(&mut sender, &mut send_sink, &mut send_stream, 8)
            .await
            .unwrap();

        let (choices, received) =
            RandomOTReceiver::receive_random(&mut receiver, &mut recv_sink, &mut recv_stream, 8)
                .await
                .unwrap();

        let expected = values
            .into_iter()
            .zip(choices)
            .map(|(v, c)| v[c as usize])
            .collect::<Vec<_>>();

        assert_eq!(received, expected);
    }
}
