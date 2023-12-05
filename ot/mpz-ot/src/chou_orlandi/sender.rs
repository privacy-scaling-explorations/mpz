use crate::{chou_orlandi::SenderError, OTError, OTSender, OTSetup, VerifiableOTSender};

use async_trait::async_trait;
use futures_util::SinkExt;
use mpz_core::{cointoss, Block, ProtocolMessage};
use mpz_ot_core::chou_orlandi::{
    msgs::Message, sender_state as state, Sender as SenderCore, SenderConfig,
};
use rand::{thread_rng, Rng};
use utils_aio::{
    non_blocking_backend::{Backend, NonBlockingBackend},
    sink::IoSink,
    stream::{ExpectStreamExt, IoStream},
};

use enum_try_as_inner::EnumTryAsInner;

#[derive(Debug, EnumTryAsInner)]
#[derive_err(Debug)]
pub(crate) enum State {
    Initialized(SenderCore<state::Initialized>),
    Setup(SenderCore<state::Setup>),
    Complete,
    Error,
}

/// Chou-Orlandi sender.
#[derive(Debug)]
pub struct Sender {
    state: State,
    /// The coin toss receiver after revealing one's own seed but before receiving a decommitment
    /// from the coin toss sender.
    cointoss_receiver: Option<cointoss::Receiver<cointoss::receiver_state::Received>>,
}

impl Sender {
    /// Creates a new Sender
    ///
    /// # Arguments
    ///
    /// * `config` - The sender's configuration
    pub fn new(config: SenderConfig) -> Self {
        Self {
            state: State::Initialized(SenderCore::new(config)),
            cointoss_receiver: None,
        }
    }

    /// Creates a new Sender with the provided RNG seed
    ///
    /// # Arguments
    ///
    /// * `config` - The sender's configuration
    /// * `seed` - The RNG seed used to generate the sender's keys
    pub fn new_with_seed(config: SenderConfig, seed: [u8; 32]) -> Self {
        Self {
            state: State::Initialized(SenderCore::new_with_seed(config, seed)),
            cointoss_receiver: None,
        }
    }
}

#[async_trait]
impl OTSetup for Sender {
    async fn setup<Si: IoSink<Message> + Send + Unpin, St: IoStream<Message> + Send + Unpin>(
        &mut self,
        sink: &mut Si,
        stream: &mut St,
    ) -> Result<(), OTError> {
        if self.state.is_setup() {
            return Ok(());
        }

        let sender = std::mem::replace(&mut self.state, State::Error)
            .try_into_initialized()
            .map_err(SenderError::from)?;

        // If the receiver is committed, we run the cointoss protocol
        if sender.config().receiver_commit() {
            self.cointoss_receiver = Some(execute_cointoss(sink, stream).await?);
        }

        let (msg, sender) = sender.setup();

        sink.send(Message::SenderSetup(msg)).await?;

        self.state = State::Setup(sender);

        Ok(())
    }
}

/// Executes the coin toss protocol as the receiver up until the point when the sender should send
/// a decommitment. The decommitment will be sent later during verification.
async fn execute_cointoss<
    Si: IoSink<Message> + Send + Unpin,
    St: IoStream<Message> + Send + Unpin,
>(
    sink: &mut Si,
    stream: &mut St,
) -> Result<cointoss::Receiver<cointoss::receiver_state::Received>, SenderError> {
    let receiver = cointoss::Receiver::new(vec![thread_rng().gen()]);

    let commitment = stream
        .expect_next()
        .await?
        .try_into_cointoss_sender_commitment()?;

    let (receiver, payload) = receiver.reveal(commitment)?;

    sink.send(Message::CointossReceiverPayload(payload)).await?;

    Ok(receiver)
}

impl ProtocolMessage for Sender {
    type Msg = Message;
}

#[async_trait]
impl OTSender<[Block; 2]> for Sender {
    async fn send<Si: IoSink<Message> + Send + Unpin, St: IoStream<Message> + Send + Unpin>(
        &mut self,
        sink: &mut Si,
        stream: &mut St,
        input: &[[Block; 2]],
    ) -> Result<(), OTError> {
        let mut sender = std::mem::replace(&mut self.state, State::Error)
            .try_into_setup()
            .map_err(SenderError::from)?;

        let receiver_payload = stream
            .expect_next()
            .await?
            .try_into_receiver_payload()
            .map_err(SenderError::from)?;

        let input = input.to_vec();
        let (sender, payload) = Backend::spawn(move || {
            let payload = sender.send(&input, receiver_payload);
            (sender, payload)
        })
        .await;

        let payload = payload.map_err(SenderError::from)?;

        sink.send(Message::SenderPayload(payload)).await?;

        self.state = State::Setup(sender);

        Ok(())
    }
}

#[async_trait]
impl VerifiableOTSender<bool, [Block; 2]> for Sender {
    async fn verify_choices<
        Si: IoSink<Message> + Send + Unpin,
        St: IoStream<Message> + Send + Unpin,
    >(
        &mut self,
        _sink: &mut Si,
        stream: &mut St,
    ) -> Result<Vec<bool>, OTError> {
        let sender = std::mem::replace(&mut self.state, State::Error)
            .try_into_setup()
            .map_err(SenderError::from)?;

        let Some(cointoss_receiver) = self.cointoss_receiver.take() else {
            Err(SenderError::InvalidConfig(
                "receiver commitment not enabled".to_string(),
            ))?
        };

        let cointoss_payload = stream
            .expect_next()
            .await?
            .try_into_cointoss_sender_payload()
            .map_err(SenderError::from)?;

        let receiver_reveal = stream
            .expect_next()
            .await?
            .try_into_receiver_reveal()
            .map_err(SenderError::from)?;

        let cointoss_seed = cointoss_receiver
            .finalize(cointoss_payload)
            .map_err(SenderError::from)?[0];
        let mut receiver_seed = [0u8; 32];
        receiver_seed[..16].copy_from_slice(&cointoss_seed.to_bytes());
        receiver_seed[16..].copy_from_slice(&cointoss_seed.to_bytes());

        let verified_choices =
            Backend::spawn(move || sender.verify_choices(receiver_seed, receiver_reveal))
                .await
                .map_err(SenderError::from)?;

        self.state = State::Complete;

        Ok(verified_choices)
    }
}
