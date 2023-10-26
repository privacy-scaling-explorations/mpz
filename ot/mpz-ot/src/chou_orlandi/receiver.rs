use async_trait::async_trait;
use futures::SinkExt;

use itybity::BitIterable;
use mpz_core::{cointoss, Block, ProtocolMessage};
use mpz_ot_core::chou_orlandi::{
    msgs::Message, receiver_state as state, Receiver as ReceiverCore, ReceiverConfig,
};

use enum_try_as_inner::EnumTryAsInner;
use rand::{thread_rng, Rng};
use utils_aio::{
    non_blocking_backend::{Backend, NonBlockingBackend},
    sink::IoSink,
    stream::{ExpectStreamExt, IoStream},
};

use crate::{CommittedOTReceiver, OTError, OTReceiver, OTSetup};

use super::ReceiverError;

#[derive(Debug, EnumTryAsInner)]
#[derive_err(Debug)]
pub(crate) enum State {
    Initialized {
        config: ReceiverConfig,
        seed: Option<[u8; 32]>,
    },
    Setup(Box<ReceiverCore<state::Setup>>),
    Complete,
    Error,
}

/// Chou-Orlandi receiver.
#[derive(Debug)]
pub struct Receiver {
    state: State,
    cointoss_payload: Option<cointoss::msgs::SenderPayload>,
}

impl Receiver {
    /// Creates a new receiver.
    ///
    /// # Arguments
    ///
    /// * `config` - The receiver's configuration
    pub fn new(config: ReceiverConfig) -> Self {
        Self {
            state: State::Initialized { config, seed: None },
            cointoss_payload: None,
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
            state: State::Initialized {
                config,
                seed: Some(seed),
            },
            cointoss_payload: None,
        }
    }
}

#[async_trait]
impl OTSetup for Receiver {
    async fn setup<Si: IoSink<Message> + Send + Unpin, St: IoStream<Message> + Send + Unpin>(
        &mut self,
        sink: &mut Si,
        stream: &mut St,
    ) -> Result<(), OTError> {
        if self.state.is_setup() {
            return Ok(());
        }

        let (config, seed) = std::mem::replace(&mut self.state, State::Error)
            .try_into_initialized()
            .map_err(ReceiverError::from)?;

        // If the receiver is committed, we generate the seed using a cointoss.
        let receiver = if config.receiver_commit() {
            if seed.is_some() {
                return Err(ReceiverError::InvalidConfig(
                    "committed receiver seed must be generated using coin toss".to_string(),
                ))?;
            }

            let (seed, cointoss_payload) = execute_cointoss(sink, stream).await?;

            self.cointoss_payload = Some(cointoss_payload);

            ReceiverCore::new_with_seed(config, seed)
        } else {
            ReceiverCore::new_with_seed(config, seed.unwrap_or_else(|| thread_rng().gen()))
        };

        let sender_setup = stream
            .expect_next()
            .await?
            .try_into_sender_setup()
            .map_err(ReceiverError::from)?;

        let receiver = Backend::spawn(move || receiver.setup(sender_setup)).await;

        self.state = State::Setup(Box::new(receiver));

        Ok(())
    }
}

/// Executes the coin toss protocol as the sender up until the point when we should send
/// a decommitment. The decommitment will be sent later during verification.
async fn execute_cointoss<
    Si: IoSink<Message> + Send + Unpin,
    St: IoStream<Message> + Send + Unpin,
>(
    sink: &mut Si,
    stream: &mut St,
) -> Result<([u8; 32], cointoss::msgs::SenderPayload), ReceiverError> {
    let (sender, commitment) = cointoss::Sender::new(vec![thread_rng().gen()]).send();

    sink.send(Message::CointossSenderCommitment(commitment))
        .await?;

    let payload = stream
        .expect_next()
        .await?
        .try_into_cointoss_receiver_payload()?;

    let (seeds, payload) = sender.finalize(payload)?;

    let mut seed = [0u8; 32];
    seed[..16].copy_from_slice(&seeds[0].to_bytes());
    seed[16..].copy_from_slice(&seeds[0].to_bytes());

    Ok((seed, payload))
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
        let mut receiver = std::mem::replace(&mut self.state, State::Error)
            .try_into_setup()
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
            .try_into_sender_payload()
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
        let receiver = std::mem::replace(&mut self.state, State::Error)
            .try_into_setup()
            .map_err(ReceiverError::from)?;

        let Some(cointoss_payload) = self.cointoss_payload.take() else {
            return Err(ReceiverError::InvalidConfig(
                "receiver not configured to commit".to_string(),
            )
            .into());
        };

        let reveal = receiver.reveal_choices().map_err(ReceiverError::from)?;

        sink.feed(Message::CointossSenderPayload(cointoss_payload))
            .await?;
        sink.feed(Message::ReceiverReveal(reveal)).await?;
        sink.flush().await?;

        self.state = State::Complete;

        Ok(())
    }
}
