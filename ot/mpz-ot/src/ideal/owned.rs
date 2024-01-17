use crate::{
    CommittedOTReceiver, CommittedOTSender, OTError, OTReceiver, OTSender, OTSetup,
    VerifiableOTReceiver, VerifiableOTSender,
};
use async_trait::async_trait;
use futures::{
    channel::{mpsc, oneshot},
    StreamExt,
};
use mpz_core::ProtocolMessage;
use utils_aio::{sink::IoSink, stream::IoStream};

/// Ideal OT sender.
#[derive(Debug)]
pub struct IdealOTSender<T> {
    sender: mpsc::Sender<Vec<[T; 2]>>,
    msgs: Vec<[T; 2]>,
    choices_receiver: Option<oneshot::Receiver<Vec<bool>>>,
}

/// Ideal OT receiver.
#[derive(Debug)]
pub struct IdealOTReceiver<T> {
    receiver: mpsc::Receiver<Vec<[T; 2]>>,
    choices: Vec<bool>,
    choices_sender: Option<oneshot::Sender<Vec<bool>>>,
}

impl<T> ProtocolMessage for IdealOTSender<T> {
    type Msg = ();
}

impl<T> ProtocolMessage for IdealOTReceiver<T> {
    type Msg = ();
}

/// Creates a pair of ideal OT sender and receiver.
pub fn ideal_ot_pair<T: Send + Sync + 'static>() -> (IdealOTSender<T>, IdealOTReceiver<T>) {
    let (sender, receiver) = mpsc::channel(10);
    let (choices_sender, choices_receiver) = oneshot::channel();

    (
        IdealOTSender {
            sender,
            msgs: Vec::default(),
            choices_receiver: Some(choices_receiver),
        },
        IdealOTReceiver {
            receiver,
            choices: Vec::default(),
            choices_sender: Some(choices_sender),
        },
    )
}

#[async_trait]
impl<T> OTSetup for IdealOTSender<T>
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
impl<T> OTSender<[T; 2]> for IdealOTSender<T>
where
    T: Send + Sync + Clone + 'static,
{
    async fn send<Si: IoSink<()> + Send + Unpin, St: IoStream<()> + Send + Unpin>(
        &mut self,
        _sink: &mut Si,
        _stream: &mut St,
        msgs: &[[T; 2]],
    ) -> Result<(), OTError> {
        self.msgs.extend(msgs.iter().cloned());

        self.sender
            .try_send(msgs.to_vec())
            .expect("DummySender should be able to send");

        Ok(())
    }
}

#[async_trait]
impl<T> OTSetup for IdealOTReceiver<T>
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
impl<T> OTReceiver<bool, T> for IdealOTReceiver<T>
where
    T: Send + Sync + 'static,
{
    async fn receive<Si: IoSink<()> + Send + Unpin, St: IoStream<()> + Send + Unpin>(
        &mut self,
        _sink: &mut Si,
        _stream: &mut St,
        choices: &[bool],
    ) -> Result<Vec<T>, OTError> {
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
impl<U, V> VerifiableOTReceiver<bool, U, V> for IdealOTReceiver<U>
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
impl<T> CommittedOTSender<[T; 2]> for IdealOTSender<T>
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
impl<T> CommittedOTReceiver<bool, T> for IdealOTReceiver<T>
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
impl<T> VerifiableOTSender<bool, [T; 2]> for IdealOTSender<T>
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
    use utils_aio::duplex::MemoryDuplex;

    use super::*;

    // Test that the sender and receiver can be used to send and receive values
    #[tokio::test]
    async fn test_ideal_ot_owned() {
        let (send_channel, recv_channel) = MemoryDuplex::<()>::new();

        let (mut send_sink, mut send_stream) = send_channel.split();
        let (mut recv_sink, mut recv_stream) = recv_channel.split();

        let values = vec![[0, 1], [2, 3]];
        let choices = vec![false, true];
        let (mut sender, mut receiver) = ideal_ot_pair::<u8>();

        sender
            .send(&mut send_sink, &mut send_stream, &values)
            .await
            .unwrap();

        let received = receiver
            .receive(&mut recv_sink, &mut recv_stream, &choices)
            .await
            .unwrap();

        assert_eq!(received, vec![0, 3]);
    }
}
