use std::{
    any::Any,
    collections::HashMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use futures::channel::oneshot;
use mpz_core::Block;

use crate::{COTReceiverShared, COTSenderShared, OTError};

type SenderBuffer = Arc<Mutex<HashMap<String, Box<dyn Any + Send + 'static>>>>;
type ReceiverBuffer = Arc<Mutex<HashMap<String, oneshot::Sender<Box<dyn Any + Send + 'static>>>>>;

/// Creates an ideal correlated ot sender and receiver pair.
pub fn ideal_cot_shared_pair(delta: Block) -> (IdealSharedCOTSender, IdealSharedCOTReceiver) {
    let sender_buffer = Arc::new(Mutex::new(HashMap::new()));
    let receiver_buffer = Arc::new(Mutex::new(HashMap::new()));

    let sender = IdealSharedCOTSender {
        delta,
        sender_buffer: sender_buffer.clone(),
        receiver_buffer: receiver_buffer.clone(),
    };

    let receiver = IdealSharedCOTReceiver {
        sender_buffer,
        receiver_buffer,
    };

    (sender, receiver)
}

/// An ideal correlated oblivious transfer sender.
#[derive(Clone, Debug)]
pub struct IdealSharedCOTSender {
    delta: Block,
    sender_buffer: SenderBuffer,
    receiver_buffer: ReceiverBuffer,
}

#[async_trait]
impl COTSenderShared<Block> for IdealSharedCOTSender {
    async fn send_correlated(&self, id: &str, msgs: &[Block]) -> Result<(), OTError> {
        let msgs = Box::new(
            msgs.iter()
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
        Ok(())
    }
}

/// An ideal correlated oblivious transfer receiver.
#[derive(Clone, Debug)]
pub struct IdealSharedCOTReceiver {
    sender_buffer: SenderBuffer,
    receiver_buffer: ReceiverBuffer,
}

#[async_trait]
impl COTReceiverShared<bool, Block> for IdealSharedCOTReceiver {
    async fn receive_correlated(&self, id: &str, choices: &[bool]) -> Result<Vec<Block>, OTError> {
        if let Some(value) = self.sender_buffer.lock().unwrap().remove(id) {
            let value = *value
                .downcast::<Vec<[Block; 2]>>()
                .expect("value type should be consistent");

            return Ok(value
                .into_iter()
                .zip(choices)
                .map(|(v, c)| v[*c as usize])
                .collect::<Vec<_>>());
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

        Ok(values
            .into_iter()
            .zip(choices)
            .map(|(v, c)| v[*c as usize])
            .collect::<Vec<_>>())
    }
}

#[cfg(test)]
mod tests {
    use itybity::IntoBits;
    use rand::Rng;
    use rand_chacha::ChaCha12Rng;
    use rand_core::SeedableRng;

    use super::*;

    #[tokio::test]
    async fn test_ideal_cot_shared() {
        let mut rng = ChaCha12Rng::seed_from_u64(0);

        let values = Block::random_vec(&mut rng, 8);
        let choices = rng.gen::<u8>().into_lsb0_vec();
        let delta = Block::from([42u8; 16]);
        let (sender, receiver) = ideal_cot_shared_pair(delta);

        sender.send_correlated("", &values).await.unwrap();

        let received = receiver.receive_correlated("", &choices).await.unwrap();

        let expected = values
            .into_iter()
            .zip(choices)
            .map(|(v, c)| if c { v ^ delta } else { v })
            .collect::<Vec<_>>();

        assert_eq!(received, expected);
    }
}
