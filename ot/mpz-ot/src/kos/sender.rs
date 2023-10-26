use crate::{
    kos::SenderError, CommittedOTReceiver, CommittedOTSender, OTError, OTReceiver, OTSender,
    OTSetup,
};

use async_trait::async_trait;
use futures_util::SinkExt;
use itybity::IntoBits;
use mpz_core::{cointoss, Block, ProtocolMessage};
use mpz_ot_core::kos::{
    msgs::Message, sender_state as state, Sender as SenderCore, SenderConfig, CSP, SSP,
};
use rand::{thread_rng, Rng};
use utils_aio::{
    non_blocking_backend::{Backend, NonBlockingBackend},
    sink::IoSink,
    stream::{ExpectStreamExt, IoStream},
};

use enum_try_as_inner::EnumTryAsInner;

use super::{into_base_sink, into_base_stream};

#[derive(Debug, EnumTryAsInner)]
#[derive_err(Debug)]
pub(crate) enum State {
    Initialized(SenderCore<state::Initialized>),
    Extension(SenderCore<state::Extension>),
    Complete,
    Error,
}

/// KOS sender.
#[derive(Debug)]
pub struct Sender<BaseOT> {
    state: State,
    base: BaseOT,

    cointoss_payload: Option<cointoss::msgs::SenderPayload>,
}

impl<BaseOT> Sender<BaseOT>
where
    BaseOT: OTReceiver<bool, Block> + Send,
{
    /// Creates a new Sender
    ///
    /// # Arguments
    ///
    /// * `config` - The Sender's configuration
    pub fn new(config: SenderConfig, base: BaseOT) -> Self {
        Self {
            state: State::Initialized(SenderCore::new(config)),
            base,
            cointoss_payload: None,
        }
    }

    /// The number of remaining OTs which can be consumed.
    pub fn remaining(&self) -> Result<usize, SenderError> {
        Ok(self.state.try_as_extension()?.remaining())
    }

    /// Returns a mutable reference to the inner sender state.
    pub(crate) fn state_mut(&mut self) -> &mut State {
        &mut self.state
    }

    /// Performs the base OT setup with the provided delta.
    ///
    /// # Arguments
    ///
    /// * `sink` - The sink to send messages to the base OT sender
    /// * `stream` - The stream to receive messages from the base OT sender
    /// * `delta` - The delta value to use for the base OT setup.
    pub async fn setup_with_delta<
        Si: IoSink<Message<BaseOT::Msg>> + Send + Unpin,
        St: IoStream<Message<BaseOT::Msg>> + Send + Unpin,
    >(
        &mut self,
        sink: &mut Si,
        stream: &mut St,
        delta: Block,
    ) -> Result<(), SenderError> {
        if self.state.try_as_initialized()?.config().sender_commit() {
            return Err(SenderError::ConfigError(
                "committed sender can not choose delta".to_string(),
            ));
        }

        self._setup_with_delta(sink, stream, delta).await
    }

    async fn _setup_with_delta<
        Si: IoSink<Message<BaseOT::Msg>> + Send + Unpin,
        St: IoStream<Message<BaseOT::Msg>> + Send + Unpin,
    >(
        &mut self,
        sink: &mut Si,
        stream: &mut St,
        delta: Block,
    ) -> Result<(), SenderError> {
        let ext_sender = std::mem::replace(&mut self.state, State::Error).try_into_initialized()?;

        let choices = delta.into_lsb0_vec();
        let seeds = self
            .base
            .receive(
                &mut into_base_sink(sink),
                &mut into_base_stream(stream),
                &choices,
            )
            .await?;

        let seeds: [Block; CSP] = seeds.try_into().expect("seeds should be CSP length");

        let ext_sender = ext_sender.setup(delta, seeds);

        self.state = State::Extension(ext_sender);

        Ok(())
    }

    /// Performs OT extension.
    ///
    /// # Arguments
    ///
    /// * `channel` - The channel to communicate with the receiver.
    /// * `count` - The number of OTs to extend.
    pub async fn extend<
        Si: IoSink<Message<BaseOT::Msg>> + Send + Unpin,
        St: IoStream<Message<BaseOT::Msg>> + Send + Unpin,
    >(
        &mut self,
        sink: &mut Si,
        stream: &mut St,
        count: usize,
    ) -> Result<(), SenderError> {
        let mut ext_sender =
            std::mem::replace(&mut self.state, State::Error).try_into_extension()?;

        // Receive extend message from the receiver.
        let extend = stream
            .expect_next()
            .await?
            .try_into_extend()
            .map_err(SenderError::from)?;

        // Receive coin toss commitments from the receiver.
        let commitment = stream.expect_next().await?.try_into_cointoss_commit()?;

        // Extend the OTs, adding padding for the consistency check.
        let mut ext_sender = Backend::spawn(move || {
            ext_sender
                .extend(count + CSP + SSP, extend)
                .map(|_| ext_sender)
        })
        .await?;

        // Execute coin toss protocol for consistency check.
        let seed: Block = thread_rng().gen();
        let cointoss_receiver = cointoss::Receiver::new(vec![seed]);

        let (cointoss_receiver, cointoss_payload) = cointoss_receiver.reveal(commitment)?;

        // Send coin toss payload to the receiver.
        sink.send(Message::CointossReceiverPayload(cointoss_payload))
            .await?;

        // Receive coin toss sender payload from the receiver.
        let cointoss_sender_payload = stream
            .expect_next()
            .await?
            .try_into_cointoss_sender_payload()?;

        // Receive consistency check from the receiver.
        let receiver_check = stream.expect_next().await?.try_into_check()?;

        // Derive chi seed for the consistency check.
        let chi_seed = cointoss_receiver.finalize(cointoss_sender_payload)?[0];

        // Check consistency of extension.
        let ext_sender = Backend::spawn(move || {
            ext_sender
                .check(chi_seed, receiver_check)
                .map(|_| ext_sender)
        })
        .await?;

        self.state = State::Extension(ext_sender);

        Ok(())
    }
}

impl<BaseOT> Sender<BaseOT>
where
    BaseOT: CommittedOTReceiver<bool, Block> + ProtocolMessage + Send,
{
    pub(crate) async fn reveal<
        Si: IoSink<Message<BaseOT::Msg>> + Send + Unpin,
        St: IoStream<Message<BaseOT::Msg>> + Send + Unpin,
    >(
        &mut self,
        sink: &mut Si,
        stream: &mut St,
    ) -> Result<(), SenderError> {
        std::mem::replace(&mut self.state, State::Error).try_into_extension()?;

        // Reveal coin toss payload
        let Some(payload) = self.cointoss_payload.take() else {
            return Err(SenderError::ConfigError(
                "committed sender not configured".to_string(),
            ))?;
        };

        sink.send(Message::CointossSenderPayload(payload))
            .await
            .map_err(SenderError::from)?;

        // Reveal base OT choices
        self.base
            .reveal_choices(&mut into_base_sink(sink), &mut into_base_stream(stream))
            .await?;

        // This sender is no longer usable, so mark it as complete.
        self.state = State::Complete;

        Ok(())
    }
}

impl<BaseOT> ProtocolMessage for Sender<BaseOT>
where
    BaseOT: ProtocolMessage,
{
    type Msg = Message<BaseOT::Msg>;
}

#[async_trait]
impl<BaseOT> OTSetup for Sender<BaseOT>
where
    BaseOT: OTSetup + OTReceiver<bool, Block> + Send,
{
    async fn setup<
        Si: IoSink<Message<BaseOT::Msg>> + Send + Unpin,
        St: IoStream<Message<BaseOT::Msg>> + Send + Unpin,
    >(
        &mut self,
        sink: &mut Si,
        stream: &mut St,
    ) -> Result<(), OTError> {
        if self.state.is_extension() {
            return Ok(());
        }

        let sender = std::mem::replace(&mut self.state, State::Error)
            .try_into_initialized()
            .map_err(SenderError::from)?;

        // If the sender is committed, we sample delta using a coin toss.
        let delta = if sender.config().sender_commit() {
            let (cointoss_sender, commitment) =
                cointoss::Sender::new(vec![thread_rng().gen()]).send();

            sink.send(Message::CointossCommit(commitment)).await?;
            let payload = stream
                .expect_next()
                .await?
                .try_into_cointoss_receiver_payload()
                .map_err(SenderError::from)?;

            let (seeds, payload) = cointoss_sender
                .finalize(payload)
                .map_err(SenderError::from)?;

            // Store the payload to reveal to the receiver later.
            self.cointoss_payload = Some(payload);

            seeds[0]
        } else {
            Block::random(&mut thread_rng())
        };

        // Set up base OT if not already done
        self.base
            .setup(&mut into_base_sink(sink), &mut into_base_stream(stream))
            .await?;

        self.state = State::Initialized(sender);

        self._setup_with_delta(sink, stream, delta)
            .await
            .map_err(OTError::from)
    }
}

#[async_trait]
impl<BaseOT> OTSender<[Block; 2]> for Sender<BaseOT>
where
    BaseOT: ProtocolMessage + Send,
{
    async fn send<
        Si: IoSink<Message<BaseOT::Msg>> + Send + Unpin,
        St: IoStream<Message<BaseOT::Msg>> + Send + Unpin,
    >(
        &mut self,
        sink: &mut Si,
        stream: &mut St,
        msgs: &[[Block; 2]],
    ) -> Result<(), OTError> {
        let sender = self
            .state
            .try_as_extension_mut()
            .map_err(SenderError::from)?;

        let derandomize = stream
            .expect_next()
            .await?
            .try_into_derandomize()
            .map_err(SenderError::from)?;

        let mut sender_keys = sender.keys(msgs.len()).map_err(SenderError::from)?;
        sender_keys
            .derandomize(derandomize)
            .map_err(SenderError::from)?;
        let payload = sender_keys
            .encrypt_blocks(msgs)
            .map_err(SenderError::from)?;

        sink.send(Message::SenderPayload(payload))
            .await
            .map_err(SenderError::from)?;

        Ok(())
    }
}

#[async_trait]
impl<const N: usize, BaseOT> OTSender<[[u8; N]; 2]> for Sender<BaseOT>
where
    BaseOT: ProtocolMessage + Send,
{
    async fn send<
        Si: IoSink<Message<BaseOT::Msg>> + Send + Unpin,
        St: IoStream<Message<BaseOT::Msg>> + Send + Unpin,
    >(
        &mut self,
        sink: &mut Si,
        stream: &mut St,
        msgs: &[[[u8; N]; 2]],
    ) -> Result<(), OTError> {
        let sender = self
            .state
            .try_as_extension_mut()
            .map_err(SenderError::from)?;

        let derandomize = stream
            .expect_next()
            .await?
            .try_into_derandomize()
            .map_err(SenderError::from)?;

        let mut sender_keys = sender.keys(msgs.len()).map_err(SenderError::from)?;
        sender_keys
            .derandomize(derandomize)
            .map_err(SenderError::from)?;
        let payload = sender_keys.encrypt_bytes(msgs).map_err(SenderError::from)?;

        sink.send(Message::SenderPayload(payload))
            .await
            .map_err(SenderError::from)?;

        Ok(())
    }
}

#[async_trait]
impl<BaseOT> CommittedOTSender<[Block; 2]> for Sender<BaseOT>
where
    BaseOT: CommittedOTReceiver<bool, Block> + ProtocolMessage + Send,
{
    async fn reveal<
        Si: IoSink<Self::Msg> + Send + Unpin,
        St: IoStream<Self::Msg> + Send + Unpin,
    >(
        &mut self,
        sink: &mut Si,
        stream: &mut St,
    ) -> Result<(), OTError> {
        self.reveal(sink, stream).await.map_err(OTError::from)
    }
}
