use crate::{COTReceiver, COTSender, OTError, OTSetup};
use async_trait::async_trait;
use futures::{channel::mpsc, StreamExt};
use mpz_core::Block;

/// Ideal OT sender.
#[derive(Debug)]
pub struct IdealCOTSender<T = Block> {
    sender: mpsc::Sender<Vec<[T; 2]>>,
    delta: Block,
}

/// Ideal OT receiver.
#[derive(Debug)]
pub struct IdealCOTReceiver<T = Block> {
    receiver: mpsc::Receiver<Vec<[T; 2]>>,
}

/// Creates a pair of ideal COT sender and receiver.
pub fn ideal_cot_pair<T: Send + Sync + 'static>(
    delta: Block,
) -> (IdealCOTSender<T>, IdealCOTReceiver<T>) {
    let (sender, receiver) = mpsc::channel(10);

    (
        IdealCOTSender { sender, delta },
        IdealCOTReceiver { receiver },
    )
}

#[async_trait]
impl<T> OTSetup for IdealCOTSender<T>
where
    T: Send + Sync,
{
    async fn setup(&mut self) -> Result<(), OTError> {
        Ok(())
    }
}

#[async_trait]
impl COTSender<Block> for IdealCOTSender<Block> {
    async fn send_correlated(&mut self, msgs: &[Block]) -> Result<(), OTError> {
        self.sender
            .try_send(
                msgs.iter()
                    .map(|msg| [*msg, *msg ^ self.delta])
                    .collect::<Vec<_>>(),
            )
            .expect("IdealCOTSender should be able to send");

        Ok(())
    }
}

#[async_trait]
impl<T> OTSetup for IdealCOTReceiver<T>
where
    T: Send + Sync,
{
    async fn setup(&mut self) -> Result<(), OTError> {
        Ok(())
    }
}

#[async_trait]
impl COTReceiver<bool, Block> for IdealCOTReceiver<Block> {
    async fn receive_correlated(&mut self, choices: &[bool]) -> Result<Vec<Block>, OTError> {
        let payload = self
            .receiver
            .next()
            .await
            .expect("IdealCOTSender should send a value");

        Ok(payload
            .into_iter()
            .zip(choices)
            .map(|(v, c)| {
                let [low, high] = v;
                if *c {
                    high
                } else {
                    low
                }
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use itybity::IntoBits;
    use rand::Rng;
    use rand_chacha::ChaCha12Rng;
    use rand_core::SeedableRng;

    use super::*;

    // Test that the sender and receiver can be used to send and receive values
    #[tokio::test]
    async fn test_ideal_cot_owned() {
        let mut rng = ChaCha12Rng::seed_from_u64(0);

        let values = Block::random_vec(&mut rng, 8);
        let choices = rng.gen::<u8>().into_lsb0_vec();
        let delta = Block::from([42u8; 16]);
        let (mut sender, mut receiver) = ideal_cot_pair::<Block>(delta);

        sender.send_correlated(&values).await.unwrap();

        let received = receiver.receive_correlated(&choices).await.unwrap();

        let expected = values
            .into_iter()
            .zip(choices)
            .map(|(v, c)| if c { v ^ delta } else { v })
            .collect::<Vec<_>>();

        assert_eq!(received, expected);
    }
}
