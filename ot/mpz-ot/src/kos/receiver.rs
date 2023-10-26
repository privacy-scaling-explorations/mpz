use async_trait::async_trait;
use futures::SinkExt;
use itybity::{FromBitIterator, IntoBitIterator};
use mpz_core::{cointoss, Block, ProtocolMessage};
use mpz_ot_core::kos::{
    msgs::Message, receiver_state as state, Receiver as ReceiverCore, ReceiverConfig, CSP, SSP,
};

use enum_try_as_inner::EnumTryAsInner;
use rand::{thread_rng, Rng};
use utils_aio::{
    non_blocking_backend::{Backend, NonBlockingBackend},
    sink::IoSink,
    stream::{ExpectStreamExt, IoStream},
};

use crate::{OTError, OTReceiver, OTSender, OTSetup, VerifiableOTReceiver, VerifiableOTSender};

use super::{into_base_sink, into_base_stream, ReceiverError, ReceiverVerifyError};

#[derive(Debug, EnumTryAsInner)]
#[derive_err(Debug)]
pub(crate) enum State {
    Initialized(Box<ReceiverCore<state::Initialized>>),
    Extension(Box<ReceiverCore<state::Extension>>),
    Verify(ReceiverCore<state::Verify>),
    Error,
}

/// KOS receiver.
#[derive(Debug)]
pub struct Receiver<BaseOT> {
    state: State,
    base: BaseOT,

    cointoss_receiver: Option<cointoss::Receiver<cointoss::receiver_state::Received>>,
}

impl<BaseOT> Receiver<BaseOT>
where
    BaseOT: OTSender<[Block; 2]> + Send,
{
    /// Creates a new receiver.
    ///
    /// # Arguments
    ///
    /// * `config` - The receiver's configuration
    pub fn new(config: ReceiverConfig, base: BaseOT) -> Self {
        Self {
            state: State::Initialized(Box::new(ReceiverCore::new(config))),
            base,
            cointoss_receiver: None,
        }
    }

    /// The number of remaining OTs which can be consumed.
    pub fn remaining(&self) -> Result<usize, ReceiverError> {
        Ok(self.state.try_as_extension()?.remaining())
    }

    /// Returns a reference to the inner receiver state.
    pub(crate) fn state(&self) -> &State {
        &self.state
    }

    /// Returns a mutable reference to the inner receiver state.
    pub(crate) fn state_mut(&mut self) -> &mut State {
        &mut self.state
    }

    /// Performs OT extension.
    ///
    /// # Arguments
    ///
    /// * `sink` - The sink to send messages to the sender
    /// * `stream` - The stream to receive messages from the sender
    /// * `count` - The number of OTs to extend
    pub async fn extend<
        Si: IoSink<Message<BaseOT::Msg>> + Send + Unpin,
        St: IoStream<Message<BaseOT::Msg>> + Send + Unpin,
    >(
        &mut self,
        sink: &mut Si,
        stream: &mut St,
        count: usize,
    ) -> Result<(), ReceiverError> {
        let mut ext_receiver =
            std::mem::replace(&mut self.state, State::Error).try_into_extension()?;

        // Extend the OTs, adding padding for the consistency check.
        let (mut ext_receiver, extend) = Backend::spawn(move || {
            let extend = ext_receiver.extend(count + CSP + SSP);

            (ext_receiver, extend)
        })
        .await;

        let extend = extend?;

        // Commit to coin toss seed
        let seed: Block = thread_rng().gen();
        let (cointoss_sender, cointoss_commitment) = cointoss::Sender::new(vec![seed]).send();

        // Send the extend message and cointoss commitment
        sink.feed(Message::Extend(extend)).await?;
        sink.feed(Message::CointossCommit(cointoss_commitment))
            .await?;
        sink.flush().await?;

        // Receive coin toss
        let cointoss_payload = stream
            .expect_next()
            .await?
            .try_into_cointoss_receiver_payload()?;

        // Open commitment
        let (mut seeds, payload) = cointoss_sender.finalize(cointoss_payload)?;
        let chi_seed = seeds.pop().expect("seed is present");

        // Compute consistency check
        let (ext_receiver, check) = Backend::spawn(move || {
            let check = ext_receiver.check(chi_seed);

            (ext_receiver, check)
        })
        .await;

        let check = check?;

        // Send coin toss decommitment and correlation check value.
        sink.feed(Message::CointossSenderPayload(payload)).await?;
        sink.feed(Message::Check(check)).await?;
        sink.flush().await?;

        self.state = State::Extension(ext_receiver);

        Ok(())
    }
}

impl<BaseOT> Receiver<BaseOT>
where
    BaseOT: VerifiableOTSender<bool, [Block; 2]> + ProtocolMessage + Send,
{
    pub(crate) async fn verify_delta<
        Si: IoSink<Message<BaseOT::Msg>> + Send + Unpin,
        St: IoStream<Message<BaseOT::Msg>> + Send + Unpin,
    >(
        &mut self,
        sink: &mut Si,
        stream: &mut St,
    ) -> Result<(), ReceiverError> {
        let receiver = std::mem::replace(&mut self.state, State::Error).try_into_extension()?;

        // Finalize coin toss to determine expected delta
        let cointoss_payload = stream
            .expect_next()
            .await?
            .try_into_cointoss_sender_payload()
            .map_err(ReceiverError::from)?;

        let Some(cointoss_receiver) = self.cointoss_receiver.take() else {
            return Err(ReceiverError::ConfigError(
                "committed sender not configured".to_string(),
            ))?;
        };

        let expected_delta = cointoss_receiver
            .finalize(cointoss_payload)
            .map_err(ReceiverError::from)?[0];

        // Receive delta by verifying the sender's base OT choices.
        let choices = self
            .base
            .verify_choices(&mut into_base_sink(sink), &mut into_base_stream(stream))
            .await?;

        let actual_delta = <[u8; 16]>::from_lsb0_iter(choices).into();

        if expected_delta != actual_delta {
            return Err(ReceiverError::from(ReceiverVerifyError::InconsistentDelta));
        }

        self.state = State::Verify(receiver.start_verification(actual_delta)?);

        Ok(())
    }
}

impl<BaseOT> ProtocolMessage for Receiver<BaseOT>
where
    BaseOT: ProtocolMessage,
{
    type Msg = Message<BaseOT::Msg>;
}

#[async_trait]
impl<BaseOT> OTSetup for Receiver<BaseOT>
where
    BaseOT: OTSetup + OTSender<[Block; 2]> + Send,
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

        let ext_receiver = std::mem::replace(&mut self.state, State::Error)
            .try_into_initialized()
            .map_err(ReceiverError::from)?;

        // If the sender is committed, we run a coin toss
        if ext_receiver.config().sender_commit() {
            let commitment = stream
                .expect_next()
                .await?
                .try_into_cointoss_commit()
                .map_err(ReceiverError::from)?;

            let (cointoss_receiver, payload) = cointoss::Receiver::new(vec![thread_rng().gen()])
                .reveal(commitment)
                .map_err(ReceiverError::from)?;

            sink.send(Message::CointossReceiverPayload(payload)).await?;

            self.cointoss_receiver = Some(cointoss_receiver);
        }

        // Set up base OT
        self.base
            .setup(&mut into_base_sink(sink), &mut into_base_stream(stream))
            .await?;

        let seeds: [[Block; 2]; CSP] = std::array::from_fn(|_| thread_rng().gen());

        // Send seeds to sender
        self.base
            .send(
                &mut into_base_sink(sink),
                &mut into_base_stream(stream),
                &seeds,
            )
            .await?;

        let ext_receiver = ext_receiver.setup(seeds);

        self.state = State::Extension(Box::new(ext_receiver));

        Ok(())
    }
}

#[async_trait]
impl<BaseOT> OTReceiver<bool, Block> for Receiver<BaseOT>
where
    BaseOT: ProtocolMessage + Send,
{
    async fn receive<
        Si: IoSink<Message<BaseOT::Msg>> + Send + Unpin,
        St: IoStream<Message<BaseOT::Msg>> + Send + Unpin,
    >(
        &mut self,
        sink: &mut Si,
        stream: &mut St,
        choices: &[bool],
    ) -> Result<Vec<Block>, OTError> {
        let receiver = self
            .state
            .try_as_extension_mut()
            .map_err(ReceiverError::from)?;

        let mut receiver_keys = receiver.keys(choices.len()).map_err(ReceiverError::from)?;

        let choices = choices.into_lsb0_vec();
        let derandomize = receiver_keys
            .derandomize(&choices)
            .map_err(ReceiverError::from)?;

        // Send derandomize message
        sink.send(Message::Derandomize(derandomize)).await?;

        // Receive payload
        let payload = stream
            .expect_next()
            .await?
            .try_into_sender_payload()
            .map_err(ReceiverError::from)?;

        let received = Backend::spawn(move || {
            receiver_keys
                .decrypt_blocks(payload)
                .map_err(ReceiverError::from)
        })
        .await?;

        Ok(received)
    }
}

#[async_trait]
impl<const N: usize, BaseOT> OTReceiver<bool, [u8; N]> for Receiver<BaseOT>
where
    BaseOT: ProtocolMessage + Send,
{
    async fn receive<
        Si: IoSink<Message<BaseOT::Msg>> + Send + Unpin,
        St: IoStream<Message<BaseOT::Msg>> + Send + Unpin,
    >(
        &mut self,
        sink: &mut Si,
        stream: &mut St,
        choices: &[bool],
    ) -> Result<Vec<[u8; N]>, OTError> {
        let receiver = self
            .state
            .try_as_extension_mut()
            .map_err(ReceiverError::from)?;

        let mut receiver_keys = receiver.keys(choices.len()).map_err(ReceiverError::from)?;

        let choices = choices.into_lsb0_vec();
        let derandomize = receiver_keys
            .derandomize(&choices)
            .map_err(ReceiverError::from)?;

        // Send derandomize message
        sink.send(Message::Derandomize(derandomize)).await?;

        // Receive payload
        let payload = stream
            .expect_next()
            .await?
            .try_into_sender_payload()
            .map_err(ReceiverError::from)?;

        let received = Backend::spawn(move || {
            receiver_keys
                .decrypt_bytes(payload)
                .map_err(ReceiverError::from)
        })
        .await?;

        Ok(received)
    }
}

#[async_trait]
impl<BaseOT> VerifiableOTReceiver<bool, Block, [Block; 2]> for Receiver<BaseOT>
where
    BaseOT: VerifiableOTSender<bool, [Block; 2]> + ProtocolMessage + Send,
{
    async fn verify<
        Si: IoSink<Self::Msg> + Send + Unpin,
        St: IoStream<Self::Msg> + Send + Unpin,
    >(
        &mut self,
        sink: &mut Si,
        stream: &mut St,
        id: usize,
        msgs: &[[Block; 2]],
    ) -> Result<(), OTError> {
        // Verify delta if we haven't yet.
        if self.state.is_extension() {
            self.verify_delta(sink, stream).await?;
        }

        let receiver = self.state.try_as_verify().map_err(ReceiverError::from)?;

        let record = receiver
            .remove_record(id as u32)
            .map_err(ReceiverError::from)?;

        let msgs = msgs.to_vec();
        Backend::spawn(move || record.verify(&msgs))
            .await
            .map_err(ReceiverError::from)?;

        Ok(())
    }
}
