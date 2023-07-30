use async_trait::async_trait;
use futures::SinkExt;

use itybity::BitIterable;
use mpz_core::{Block, ProtocolMessage};
use mpz_ot_core::chou_orlandi::{
    msgs::Message, receiver_state as state, Receiver as ReceiverCore, ReceiverConfig,
};

use enum_try_as_inner::EnumTryAsInner;
use utils_aio::{
    non_blocking_backend::{Backend, NonBlockingBackend},
    sink::IoSink,
    stream::{ExpectStreamExt, IoStream},
};

use crate::{CommittedOTReceiver, OTError, OTReceiver};

use super::ReceiverError;

#[derive(Debug, EnumTryAsInner)]
enum State {
    Initialized(Box<ReceiverCore<state::Initialized>>),
    Setup(Box<ReceiverCore<state::Setup>>),
    Complete,
    Error,
}

impl From<enum_try_as_inner::Error<State>> for ReceiverError {
    fn from(value: enum_try_as_inner::Error<State>) -> Self {
        ReceiverError::StateError(value.to_string())
    }
}

/// Chou-Orlandi receiver.
#[derive(Debug)]
pub struct Receiver {
    state: State,
}

impl Receiver {
    /// Creates a new receiver.
    ///
    /// # Arguments
    ///
    /// * `config` - The receiver's configuration
    pub fn new(config: ReceiverConfig) -> Self {
        Self {
            state: State::Initialized(Box::new(ReceiverCore::new(config))),
        }
    }

    /// Creates a new receiver with the provided RNG seed.
    ///
    /// # Arguments
    ///
    /// * `config` - The receiver's configuration
    /// * `seed` - The RNG seed used to generate the receiver's keys.
    pub fn new_with_seed(config: ReceiverConfig, seed: [u8; 32]) -> Self {
        Self {
            state: State::Initialized(Box::new(ReceiverCore::new_with_seed(config, seed))),
        }
    }

    /// Sets up the receiver.
    ///
    /// # Arguments
    ///
    /// * `sink` - The sink to send messages to the sender
    /// * `stream` - The stream to receive messages from the sender
    pub async fn setup<Si: IoSink<Message> + Send + Unpin, St: IoStream<Message> + Send + Unpin>(
        &mut self,
        sink: &mut Si,
        stream: &mut St,
    ) -> Result<(), ReceiverError> {
        let receiver = self.state.replace(State::Error).into_initialized()?;

        let sender_setup = stream.expect_next().await?.into_sender_setup()?;

        let (receiver_setup, receiver) = Backend::spawn(move || receiver.setup(sender_setup)).await;

        sink.send(Message::ReceiverSetup(receiver_setup)).await?;

        self.state = State::Setup(Box::new(receiver));

        Ok(())
    }
}

impl ProtocolMessage for Receiver {
    type Msg = Message;
}

#[async_trait]
impl<T> OTReceiver<T, Block> for Receiver
where
    T: BitIterable + Send + Sync + Clone + 'static,
{
    async fn receive<
        Si: IoSink<Self::Msg> + Send + Unpin,
        St: IoStream<Self::Msg> + Send + Unpin,
    >(
        &mut self,
        sink: &mut Si,
        stream: &mut St,
        choices: &[T],
    ) -> Result<Vec<Block>, OTError> {
        let mut receiver = self
            .state
            .replace(State::Error)
            .into_setup()
            .map_err(ReceiverError::from)?;

        let choices = choices.to_vec();
        let (mut receiver, receiver_payload) = Backend::spawn(move || {
            let payload = receiver.receive_random(&choices);
            (receiver, payload)
        })
        .await;

        sink.send(Message::ReceiverPayload(receiver_payload))
            .await?;

        let sender_payload = stream
            .expect_next()
            .await?
            .into_sender_payload()
            .map_err(ReceiverError::from)?;

        let (receiver, data) = Backend::spawn(move || {
            let data = receiver.receive(sender_payload);
            (receiver, data)
        })
        .await;

        let data = data.map_err(ReceiverError::from)?;

        self.state = State::Setup(receiver);

        Ok(data)
    }
}

#[async_trait]
impl CommittedOTReceiver<bool, Block> for Receiver {
    async fn reveal_choices<
        Si: IoSink<Self::Msg> + Send + Unpin,
        St: IoStream<Self::Msg> + Send + Unpin,
    >(
        &mut self,
        sink: &mut Si,
        _stream: &mut St,
    ) -> Result<(), OTError> {
        let receiver = self
            .state
            .replace(State::Complete)
            .into_setup()
            .map_err(ReceiverError::from)?;

        let reveal = receiver.reveal_choices().map_err(ReceiverError::from)?;

        sink.send(Message::ReceiverReveal(reveal)).await?;

        Ok(())
    }
}
