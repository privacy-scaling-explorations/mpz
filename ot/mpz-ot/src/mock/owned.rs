use crate::{
    CommittedOTReceiver, CommittedOTSender, CommittedOTSenderWithIo, OTError, OTReceiver,
    OTReceiverWithIo, OTSender, OTSenderWithIo, VerifiableOTReceiver, VerifiableOTReceiverWithIo,
    VerifiableOTSender,
};
use async_trait::async_trait;
use futures::{
    channel::{mpsc, oneshot},
    StreamExt,
};
use mpz_core::ProtocolMessage;
use utils_aio::{sink::IoSink, stream::IoStream};

/// Mock OT sender.
#[derive(Debug)]
pub struct MockOTSender<T> {
    sender: mpsc::Sender<Vec<[T; 2]>>,
    msgs: Vec<[T; 2]>,
    choices_receiver: Option<oneshot::Receiver<Vec<bool>>>,
}

/// Mock OT receiver.
#[derive(Debug)]
pub struct MockOTReceiver<T> {
    receiver: mpsc::Receiver<Vec<[T; 2]>>,
    choices: Vec<bool>,
    choices_sender: Option<oneshot::Sender<Vec<bool>>>,
}

impl<T> ProtocolMessage for MockOTSender<T> {
    type Msg = ();
}

impl<T> ProtocolMessage for MockOTReceiver<T> {
    type Msg = ();
}

/// Creates a pair of mock OT sender and receiver.
pub fn mock_ot_pair<T: Send + Sync + 'static>() -> (MockOTSender<T>, MockOTReceiver<T>) {
    let (sender, receiver) = mpsc::channel(10);
    let (choices_sender, choices_receiver) = oneshot::channel();

    (
        MockOTSender {
            sender,
            msgs: Vec::default(),
            choices_receiver: Some(choices_receiver),
        },
        MockOTReceiver {
            receiver,
            choices: Vec::default(),
            choices_sender: Some(choices_sender),
        },
    )
}

#[async_trait]
impl<T> OTSender<[T; 2]> for MockOTSender<T>
where
    T: Send + Sync + Clone + 'static,
{
    async fn send<Si: IoSink<()> + Send + Unpin, St: IoStream<()> + Send + Unpin>(
        &mut self,
        _sink: &mut Si,
        _stream: &mut St,
        msgs: &[[T; 2]],
    ) -> Result<(), OTError> {
        <Self as OTSenderWithIo<[T; 2]>>::send(self, msgs).await
    }
}

#[async_trait]
impl<T> OTSenderWithIo<[T; 2]> for MockOTSender<T>
where
    T: Send + Sync + Clone + 'static,
{
    async fn send(&mut self, msgs: &[[T; 2]]) -> Result<(), OTError> {
        self.msgs.extend(msgs.iter().cloned());

        self.sender
            .try_send(msgs.to_vec())
            .expect("DummySender should be able to send");

        Ok(())
    }
}

#[async_trait]
impl<T> OTReceiver<bool, T> for MockOTReceiver<T>
where
    T: Send + Sync + 'static,
{
    async fn receive<Si: IoSink<()> + Send + Unpin, St: IoStream<()> + Send + Unpin>(
        &mut self,
        _sink: &mut Si,
        _stream: &mut St,
        choices: &[bool],
    ) -> Result<Vec<T>, OTError> {
        <Self as OTReceiverWithIo<bool, T>>::receive(self, choices).await
    }
}

#[async_trait]
impl<T> OTReceiverWithIo<bool, T> for MockOTReceiver<T>
where
    T: Send + Sync + 'static,
{
    async fn receive(&mut self, choices: &[bool]) -> Result<Vec<T>, OTError> {
        self.choices.extend(choices.iter().copied());

        let payload = self
            .receiver
            .next()
            .await
            .expect("DummySender should send a value");

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

#[async_trait]
impl<U, V> VerifiableOTReceiver<bool, U, V> for MockOTReceiver<U>
where
    U: Send + Sync + 'static,
    V: Send + Sync + 'static,
{
    async fn verify<Si: IoSink<()> + Send + Unpin, St: IoStream<()> + Send + Unpin>(
        &mut self,
        _sink: &mut Si,
        _stream: &mut St,
        _index: usize,
        _msgs: &[V],
    ) -> Result<(), OTError> {
        Ok(())
    }
}

#[async_trait]
impl<T> VerifiableOTReceiverWithIo<[T; 2]> for MockOTReceiver<T>
where
    T: Send + Sync + 'static,
{
    async fn verify(&mut self, _msgs: &[[T; 2]]) -> Result<(), OTError> {
        Ok(())
    }
}

#[async_trait]
impl<T> CommittedOTSender<[T; 2]> for MockOTSender<T>
where
    T: Send + Sync + Clone + 'static,
{
    async fn reveal<Si: IoSink<()> + Send + Unpin, St: IoStream<()> + Send + Unpin>(
        &mut self,
        _sink: &mut Si,
        _stream: &mut St,
    ) -> Result<(), OTError> {
        Ok(())
    }
}

#[async_trait]
impl<T> CommittedOTSenderWithIo for MockOTSender<T>
where
    T: Send + 'static,
{
    async fn reveal(&mut self) -> Result<(), OTError> {
        Ok(())
    }
}

#[async_trait]
impl<T> CommittedOTReceiver<bool, T> for MockOTReceiver<T>
where
    T: Send + Sync + 'static,
{
    async fn reveal_choices<Si: IoSink<()> + Send + Unpin, St: IoStream<()> + Send + Unpin>(
        &mut self,
        _sink: &mut Si,
        _stream: &mut St,
    ) -> Result<(), OTError> {
        self.choices_sender
            .take()
            .expect("choices should not be revealed twice")
            .send(self.choices.clone())
            .expect("DummySender should be able to send");

        Ok(())
    }
}

#[async_trait]
impl<T> VerifiableOTSender<bool, [T; 2]> for MockOTSender<T>
where
    T: Send + Sync + Clone + 'static,
{
    async fn verify_choices<Si: IoSink<()> + Send + Unpin, St: IoStream<()> + Send + Unpin>(
        &mut self,
        _sink: &mut Si,
        _stream: &mut St,
    ) -> Result<Vec<bool>, OTError> {
        Ok(self
            .choices_receiver
            .take()
            .expect("choices should not be verified twice")
            .await
            .expect("choices sender should not be dropped"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test that the sender and receiver can be used to send and receive values
    #[tokio::test]
    async fn test_mock_ot_owned() {
        let values = vec![[0, 1], [2, 3]];
        let choices = vec![false, true];
        let (mut sender, mut receiver) = mock_ot_pair::<u8>();

        OTSenderWithIo::send(&mut sender, &values).await.unwrap();

        let received = OTReceiverWithIo::receive(&mut receiver, &choices)
            .await
            .unwrap();
        assert_eq!(received, vec![0, 3]);
    }
}
