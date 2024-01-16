use std::{
    any::Any,
    collections::HashMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use futures::channel::oneshot;
use mpz_core::Block;
use rand::Rng;
use rand_chacha::ChaCha12Rng;
use rand_core::SeedableRng;

use crate::{OTError, RandomCOTReceiverShared, RandomCOTSenderShared};

type SenderBuffer = Arc<Mutex<HashMap<String, Box<dyn Any + Send + 'static>>>>;
type ReceiverBuffer = Arc<Mutex<HashMap<String, oneshot::Sender<Box<dyn Any + Send + 'static>>>>>;

/// Creates an ideal random cot sender and receiver pair.
pub fn ideal_random_cot_shared_pair(
    seed: [u8; 32],
    delta: Block,
) -> (IdealSharedRandomCOTSender, IdealSharedRandomCOTReceiver) {
    let sender_buffer: Arc<Mutex<HashMap<String, Box<dyn Any + Send>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let receiver_buffer = Arc::new(Mutex::new(HashMap::new()));

    let sender = IdealSharedRandomCOTSender {
        rng: Arc::new(Mutex::new(ChaCha12Rng::from_seed(seed))),
        delta,
        sender_buffer: sender_buffer.clone(),
        receiver_buffer: receiver_buffer.clone(),
    };

    let receiver = IdealSharedRandomCOTReceiver {
        rng: Arc::new(Mutex::new(ChaCha12Rng::from_seed(seed))),
        sender_buffer,
        receiver_buffer,
    };

    (sender, receiver)
}

/// An ideal random correlated oblivious transfer sender.
#[derive(Clone, Debug)]
pub struct IdealSharedRandomCOTSender {
    delta: Block,
    rng: Arc<Mutex<ChaCha12Rng>>,
    sender_buffer: SenderBuffer,
    receiver_buffer: ReceiverBuffer,
}

#[async_trait]
impl RandomCOTSenderShared<Block> for IdealSharedRandomCOTSender {
    async fn send_random_correlated(&self, id: &str, count: usize) -> Result<Vec<Block>, OTError> {
        let low = Block::random_vec(&mut (*self.rng.lock().unwrap()), count);
        let msgs = Box::new(
            low.iter()
                .map(|msg| [*msg, *msg ^ self.delta])
                .collect::<Vec<_>>(),
        );
        if let Some(sender) = self.receiver_buffer.lock().unwrap().remove(id) {
            sender
                .send(msgs)
                .expect("IdealCOTSenderControl should be able to send");
        } else {
            self.sender_buffer
                .lock()
                .unwrap()
                .insert(id.to_string(), msgs);
        }
        Ok(low)
    }
}

/// An ideal random correlated oblivious transfer receiver.
#[derive(Clone, Debug)]
pub struct IdealSharedRandomCOTReceiver {
    rng: Arc<Mutex<ChaCha12Rng>>,
    sender_buffer: SenderBuffer,
    receiver_buffer: ReceiverBuffer,
}

#[async_trait]
impl RandomCOTReceiverShared<bool, Block> for IdealSharedRandomCOTReceiver {
    async fn receive_random_correlated(
        &self,
        id: &str,
        count: usize,
    ) -> Result<(Vec<bool>, Vec<Block>), OTError> {
        let choices = (0..count)
            .map(|_| (*self.rng.lock().unwrap()).gen())
            .collect::<Vec<bool>>();
        if let Some(value) = self.sender_buffer.lock().unwrap().remove(id) {
            let values = *value
                .downcast::<Vec<[Block; 2]>>()
                .expect("value type should be consistent");

            let value = values
                .into_iter()
                .zip(&choices)
                .map(|([low, high], c)| if *c { high } else { low })
                .collect::<Vec<_>>();

            return Ok((choices, value));
        }

        let (sender, receiver) = oneshot::channel();
        self.receiver_buffer
            .lock()
            .unwrap()
            .insert(id.to_string(), sender);

        let values = receiver.await.unwrap();

        let values = *values
            .downcast::<Vec<[Block; 2]>>()
            .expect("value type should be consistent");

        let values = values
            .into_iter()
            .zip(&choices)
            .map(|([low, high], c)| if *c { high } else { low })
            .collect::<Vec<_>>();

        Ok((choices, values))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_ideal_random_cot_shared() {
        let delta = Block::from([42u8; 16]);
        let (sender, receiver) = ideal_random_cot_shared_pair([0u8; 32], delta);

        let values = sender.send_random_correlated("", 8).await.unwrap();

        let (choices, received) = receiver.receive_random_correlated("", 8).await.unwrap();

        let expected = values
            .into_iter()
            .zip(choices)
            .map(|(v, c)| if c { v ^ delta } else { v })
            .collect::<Vec<_>>();

        assert_eq!(received, expected);
    }
}
