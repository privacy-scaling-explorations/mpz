use std::{
    any::Any,
    collections::HashMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use futures::channel::oneshot;

use crate::{
    CommittedOTSenderShared, OTError, OTReceiverShared, OTSenderShared, VerifiableOTReceiverShared,
};

/// Creates a ideal sender and receiver pair.
pub fn ideal_ot_shared_pair() -> (IdealSharedOTSender, IdealSharedOTReceiver) {
    let sender_buffer = Arc::new(Mutex::new(HashMap::new()));
    let receiver_buffer = Arc::new(Mutex::new(HashMap::new()));

    let sender = IdealSharedOTSender {
        sender_buffer: sender_buffer.clone(),
        receiver_buffer: receiver_buffer.clone(),
    };

    let receiver = IdealSharedOTReceiver {
        sender_buffer,
        receiver_buffer,
    };

    (sender, receiver)
}

/// A mock oblivious transfer sender.
#[derive(Clone, Debug)]
#[allow(clippy::type_complexity)]
pub struct IdealSharedOTSender {
    sender_buffer: Arc<Mutex<HashMap<String, Box<dyn Any + Send + 'static>>>>,
    receiver_buffer: Arc<Mutex<HashMap<String, oneshot::Sender<Box<dyn Any + Send + 'static>>>>>,
}

#[async_trait]
impl<T: Clone + std::fmt::Debug + Send + Sync + 'static> OTSenderShared<[T; 2]>
    for IdealSharedOTSender
{
    async fn send(&self, id: &str, msgs: &[[T; 2]]) -> Result<(), OTError> {
        let msgs = Box::new(msgs.to_vec());
        if let Some(sender) = self.receiver_buffer.lock().unwrap().remove(id) {
            sender
                .send(msgs)
                .expect("IdealOTSenderControl should be able to send");
        } else {
            self.sender_buffer
                .lock()
                .unwrap()
                .insert(id.to_string(), msgs);
        }
        Ok(())
    }
}

#[async_trait]
impl<T: Clone + std::fmt::Debug + Send + Sync + 'static> CommittedOTSenderShared<[T; 2]>
    for IdealSharedOTSender
{
    async fn reveal(&self) -> Result<(), OTError> {
        Ok(())
    }
}

/// A mock oblivious transfer receiver.
#[derive(Clone, Debug)]
#[allow(clippy::type_complexity)]
pub struct IdealSharedOTReceiver {
    sender_buffer: Arc<Mutex<HashMap<String, Box<dyn Any + Send + 'static>>>>,
    receiver_buffer: Arc<Mutex<HashMap<String, oneshot::Sender<Box<dyn Any + Send + 'static>>>>>,
}

#[async_trait]
impl<T: Send + Copy + 'static> OTReceiverShared<bool, T> for IdealSharedOTReceiver {
    async fn receive(&self, id: &str, choices: &[bool]) -> Result<Vec<T>, OTError> {
        if let Some(value) = self.sender_buffer.lock().unwrap().remove(id) {
            let value = *value
                .downcast::<Vec<[T; 2]>>()
                .expect("value type should be consistent");

            return Ok(value
                .into_iter()
                .zip(choices)
                .map(|(v, c)| v[*c as usize])
                .collect::<Vec<T>>());
        }

        let (sender, receiver) = oneshot::channel();
        self.receiver_buffer
            .lock()
            .unwrap()
            .insert(id.to_string(), sender);

        let values = receiver.await.unwrap();

        let values = *values
            .downcast::<Vec<[T; 2]>>()
            .expect("value type should be consistent");

        Ok(values
            .into_iter()
            .zip(choices)
            .map(|(v, c)| v[*c as usize])
            .collect::<Vec<T>>())
    }
}

#[async_trait]
impl<T: Send + Copy + 'static> VerifiableOTReceiverShared<bool, T, [T; 2]>
    for IdealSharedOTReceiver
{
    async fn verify(&self, _id: &str, _msgs: &[[T; 2]]) -> Result<(), OTError> {
        // Ideal OT is always honest
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_ideal_ot() {
        let values = vec![[0, 1], [2, 3]];
        let choices = vec![false, true];
        let (sender, receiver) = ideal_ot_shared_pair();

        sender.send("", &values).await.unwrap();

        let received: Vec<i32> = receiver.receive("", &choices).await.unwrap();
        assert_eq!(received, vec![0, 3]);
    }
}
