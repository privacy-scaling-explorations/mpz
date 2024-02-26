use async_trait::async_trait;
use futures::SinkExt;

use itybity::BitIterable;
use mpz_core::{cointoss, Block};
use mpz_ot_core::chou_orlandi::{
    msgs::Message, receiver_state as state, Receiver as ReceiverCore, ReceiverConfig,
};

use enum_try_as_inner::EnumTryAsInner;
use rand::{thread_rng, Rng};
use utils_aio::{
    duplex::Duplex,
    non_blocking_backend::{Backend, NonBlockingBackend},
    stream::ExpectStreamExt,
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
pub struct Receiver<Io> {
    io: Io,
    state: State,
    cointoss_payload: Option<cointoss::msgs::SenderPayload>,
}

impl<Io> Receiver<Io>
where
    Io: Duplex<Message>,
{
    /// Creates a new receiver.
    ///
    /// # Arguments
    ///
    /// * `config` - The receiver's configuration
    pub fn new(config: ReceiverConfig, io: Io) -> Self {
        Self {
            io,
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
    pub fn new_with_seed(config: ReceiverConfig, io: Io, seed: [u8; 32]) -> Self {
        Self {
            io,
            state: State::Initialized {
                config,
                seed: Some(seed),
            },
            cointoss_payload: None,
        }
    }
}

#[async_trait]
impl<Io> OTSetup for Receiver<Io>
where
    Io: Duplex<Message>,
{
    async fn setup(&mut self) -> Result<(), OTError> {
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

            let (seed, cointoss_payload) = execute_cointoss(&mut self.io).await?;

            self.cointoss_payload = Some(cointoss_payload);

            ReceiverCore::new_with_seed(config, seed)
        } else {
            ReceiverCore::new_with_seed(config, seed.unwrap_or_else(|| thread_rng().gen()))
        };

        let sender_setup = self
            .io
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
async fn execute_cointoss<Io: Duplex<Message>>(
    io: &mut Io,
) -> Result<([u8; 32], cointoss::msgs::SenderPayload), ReceiverError> {
    let (sender, commitment) = cointoss::Sender::new(vec![thread_rng().gen()]).send();

    io.send(Message::CointossSenderCommitment(commitment))
        .await?;

    let payload = io
        .expect_next()
        .await?
        .try_into_cointoss_receiver_payload()?;

    let (seeds, payload) = sender.finalize(payload)?;

    let mut seed = [0u8; 32];
    seed[..16].copy_from_slice(&seeds[0].to_bytes());
    seed[16..].copy_from_slice(&seeds[0].to_bytes());

    Ok((seed, payload))
}

#[async_trait]
impl<Io, T> OTReceiver<T, Block> for Receiver<Io>
where
    Io: Duplex<Message>,
    T: BitIterable + Send + Sync + Clone + 'static,
{
    async fn receive(&mut self, choices: &[T]) -> Result<Vec<Block>, OTError> {
        let mut receiver = std::mem::replace(&mut self.state, State::Error)
            .try_into_setup()
            .map_err(ReceiverError::from)?;

        let choices = choices.to_vec();
        let (mut receiver, receiver_payload) = Backend::spawn(move || {
            let payload = receiver.receive_random(&choices);
            (receiver, payload)
        })
        .await;

        self.io
            .send(Message::ReceiverPayload(receiver_payload))
            .await?;

        let sender_payload = self
            .io
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
impl<Io> CommittedOTReceiver<bool, Block> for Receiver<Io>
where
    Io: Duplex<Message>,
{
    async fn reveal_choices(&mut self) -> Result<(), OTError> {
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

        self.io
            .feed(Message::CointossSenderPayload(cointoss_payload))
            .await?;
        self.io.feed(Message::ReceiverReveal(reveal)).await?;
        self.io.flush().await?;

        self.state = State::Complete;

        Ok(())
    }
}
