use std::{
    any::Any,
    collections::HashMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use futures::channel::oneshot;
use mpz_core::{prg::Prg, Block};
use rand::Rng;
use rand_chacha::ChaCha12Rng;
use rand_core::{RngCore, SeedableRng};

use crate::{OTError, RandomOTReceiverShared, RandomOTSenderShared};

type SenderBuffer = Arc<Mutex<HashMap<String, Box<dyn Any + Send + 'static>>>>;
type ReceiverBuffer = Arc<Mutex<HashMap<String, oneshot::Sender<Box<dyn Any + Send + 'static>>>>>;

/// Creates an ideal random ot sender and receiver pair.
pub fn ideal_random_ot_shared_pair(
    seed: [u8; 32],
) -> (IdealSharedRandomOTSender, IdealSharedRandomOTReceiver) {
    let sender_buffer: Arc<Mutex<HashMap<String, Box<dyn Any + Send>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let receiver_buffer = Arc::new(Mutex::new(HashMap::new()));

    let sender = IdealSharedRandomOTSender {
        rng: Arc::new(Mutex::new(ChaCha12Rng::from_seed(seed))),
        sender_buffer: sender_buffer.clone(),
        receiver_buffer: receiver_buffer.clone(),
    };

    let receiver = IdealSharedRandomOTReceiver {
        rng: Arc::new(Mutex::new(ChaCha12Rng::from_seed(seed))),
        sender_buffer,
        receiver_buffer,
    };

    (sender, receiver)
}

/// An ideal random oblivious transfer sender.
#[derive(Clone, Debug)]
pub struct IdealSharedRandomOTSender {
    rng: Arc<Mutex<ChaCha12Rng>>,
    sender_buffer: SenderBuffer,
    receiver_buffer: ReceiverBuffer,
}

#[async_trait]
impl RandomOTSenderShared<[Block; 2]> for IdealSharedRandomOTSender {
    async fn send_random(&self, id: &str, count: usize) -> Result<Vec<[Block; 2]>, OTError> {
        let blocks = Block::random_vec(&mut (*self.rng.lock().unwrap()), 2 * count);
        let messages = (0..count)
            .map(|k| [blocks[2 * k], blocks[2 * k + 1]])
            .collect::<Vec<_>>();

        if let Some(sender) = self.receiver_buffer.lock().unwrap().remove(id) {
            sender
                .send(Box::new(messages.clone()))
                .expect("IdealOTSenderControl should be able to send");
        } else {
            self.sender_buffer
                .lock()
                .unwrap()
                .insert(id.to_string(), Box::new(messages.clone()));
        }
        Ok(messages)
    }
}

#[async_trait]
impl<const N: usize> RandomOTSenderShared<[[u8; N]; 2]> for IdealSharedRandomOTSender {
    async fn send_random(&self, id: &str, count: usize) -> Result<Vec<[[u8; N]; 2]>, OTError> {
        let prng = |block| {
            let mut prg = Prg::from_seed(block);
            let mut out = [0_u8; N];
            prg.fill_bytes(&mut out);
            out
        };

        let blocks = Block::random_vec(&mut (*self.rng.lock().unwrap()), 2 * count);
        let messages = (0..count)
            .map(|k| [prng(blocks[2 * k]), prng(blocks[2 * k + 1])])
            .collect::<Vec<_>>();

        if let Some(sender) = self.receiver_buffer.lock().unwrap().remove(id) {
            sender
                .send(Box::new(messages.clone()))
                .expect("IdealOTSenderControl should be able to send");
        } else {
            self.sender_buffer
                .lock()
                .unwrap()
                .insert(id.to_string(), Box::new(messages.clone()));
        }
        Ok(messages)
    }
}

/// An ideal random oblivious transfer receiver.
#[derive(Clone, Debug)]
pub struct IdealSharedRandomOTReceiver {
    rng: Arc<Mutex<ChaCha12Rng>>,
    sender_buffer: SenderBuffer,
    receiver_buffer: ReceiverBuffer,
}

#[async_trait]
impl RandomOTReceiverShared<bool, Block> for IdealSharedRandomOTReceiver {
    async fn receive_random(
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

#[async_trait]
impl<const N: usize> RandomOTReceiverShared<bool, [u8; N]> for IdealSharedRandomOTReceiver {
    async fn receive_random(
        &self,
        id: &str,
        count: usize,
    ) -> Result<(Vec<bool>, Vec<[u8; N]>), OTError> {
        let choices = (0..count)
            .map(|_| (*self.rng.lock().unwrap()).gen())
            .collect::<Vec<bool>>();

        if let Some(value) = self.sender_buffer.lock().unwrap().remove(id) {
            let values = *value
                .downcast::<Vec<[[u8; N]; 2]>>()
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
            .downcast::<Vec<[[u8; N]; 2]>>()
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
    async fn test_ideal_random_ot_shared_block() {
        let (sender, receiver) = ideal_random_ot_shared_pair([0u8; 32]);

        let values: Vec<[Block; 2]> = sender.send_random("", 8).await.unwrap();

        let (choices, received): (Vec<bool>, Vec<Block>) =
            receiver.receive_random("", 8).await.unwrap();

        let expected: Vec<Block> = values
            .into_iter()
            .zip(choices)
            .map(|(v, c): ([Block; 2], bool)| v[c as usize])
            .collect::<Vec<_>>();

        assert_eq!(received, expected);
    }

    #[tokio::test]
    async fn test_ideal_random_ot_shared_array() {
        let (sender, receiver) = ideal_random_ot_shared_pair([0u8; 32]);

        let values: Vec<[[u8; 64]; 2]> = sender.send_random("", 8).await.unwrap();

        let (choices, received): (Vec<bool>, Vec<[u8; 64]>) =
            receiver.receive_random("", 8).await.unwrap();

        let expected: Vec<[u8; 64]> = values
            .into_iter()
            .zip(choices)
            .map(|(v, c): ([[u8; 64]; 2], bool)| v[c as usize])
            .collect::<Vec<_>>();

        assert_eq!(received, expected);
    }
}
