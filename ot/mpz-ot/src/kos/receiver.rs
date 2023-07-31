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

use crate::{OTError, OTReceiver, OTSender, VerifiableOTReceiver, VerifiableOTSender};

use super::{into_base_sink, into_base_stream, ReceiverError};

#[derive(Debug, EnumTryAsInner)]
enum State {
    Initialized(Box<ReceiverCore<state::Initialized>>),
    Extension(Box<ReceiverCore<state::Extension>>),
    Error,
}

impl From<enum_try_as_inner::Error<State>> for ReceiverError {
    fn from(value: enum_try_as_inner::Error<State>) -> Self {
        ReceiverError::StateError(value.to_string())
    }
}

/// KOS receiver.
#[derive(Debug)]
pub struct Receiver<BaseOT> {
    state: State,
    base: BaseOT,

    /// The verified delta value used by the sender, if revealed.
    delta: Option<Block>,
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
            delta: None,
        }
    }

    /// The number of remaining OTs which can be consumed.
    pub fn remaining(&self) -> Result<usize, ReceiverError> {
        Ok(self.state.as_extension()?.remaining())
    }

    /// Performs the base OT setup.
    ///
    /// # Arguments
    ///
    /// * `sink` - The sink to send messages to the base OT receiver
    /// * `stream` - The stream to receive messages from the base OT receiver
    pub async fn setup<
        Si: IoSink<Message<BaseOT::Msg>> + Send + Unpin,
        St: IoStream<Message<BaseOT::Msg>> + Send + Unpin,
    >(
        &mut self,
        sink: &mut Si,
        stream: &mut St,
    ) -> Result<(), ReceiverError> {
        let ext_receiver = self.state.replace(State::Error).into_initialized()?;

        let mut rng = thread_rng();
        let seeds: [[Block; 2]; CSP] = std::array::from_fn(|_| [rng.gen(), rng.gen()]);

        // Send seeds to sender
        self.base
            .send(
                &mut into_base_sink(sink),
                &mut into_base_stream(stream),
                &seeds,
            )
            .await?;

        let ext_receiver = ext_receiver.base_setup(seeds);

        self.state = State::Extension(Box::new(ext_receiver));

        Ok(())
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
        let mut ext_receiver = self.state.replace(State::Error).into_extension()?;

        // Extend the OTs, adding padding for the consistency check.
        let (mut ext_receiver, extend) = Backend::spawn(move || {
            let extend = ext_receiver.extend(count + CSP + SSP);

            (ext_receiver, extend)
        })
        .await;

        // Commit to cointoss seed
        let seed: Block = thread_rng().gen();
        let (cointoss_sender, cointoss_commitment) = cointoss::Sender::new(vec![seed]).send();

        // Send the extend message and cointoss commitment
        sink.feed(Message::Extend(extend)).await?;
        sink.feed(Message::CointossCommit(cointoss_commitment))
            .await?;
        sink.flush().await?;

        // Receive cointoss
        let cointoss_payload = stream
            .expect_next()
            .await?
            .into_cointoss_receiver_payload()?;

        // Open commitment
        let (mut seeds, payload) = cointoss_sender.finalize(cointoss_payload)?;
        let chi_seed = seeds.pop().expect("seed is present");

        // Compute consistency check
        let (ext_receiver, check) = Backend::spawn(move || {
            let check = ext_receiver.check(chi_seed);

            (ext_receiver, check)
        })
        .await;

        // Send consistency check
        sink.feed(Message::CointossSenderPayload(payload)).await?;
        sink.feed(Message::Check(check)).await?;
        sink.flush().await?;

        self.state = State::Extension(ext_receiver);

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
        let mut receiver = self
            .state
            .replace(State::Error)
            .into_extension()
            .map_err(ReceiverError::from)?;

        let choices = choices.into_lsb0_vec();
        let derandomize = receiver.derandomize(&choices);

        // Send derandomize message
        sink.send(Message::Derandomize(derandomize)).await?;

        // Receive payload
        let payload = stream
            .expect_next()
            .await?
            .into_sender_payload()
            .map_err(ReceiverError::from)?;

        let (receiver, received) = Backend::spawn(move || {
            receiver
                .receive(payload)
                .map(|received| (receiver, received))
                .map_err(ReceiverError::from)
        })
        .await?;

        self.state = State::Extension(receiver);

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
        index: usize,
        msgs: &[[Block; 2]],
    ) -> Result<(), OTError> {
        let receiver = self.state.as_extension().map_err(ReceiverError::from)?;

        let delta = if let Some(delta) = self.delta {
            delta
        } else {
            // Receive delta by verifying the sender's base OT choices.
            let choices = self
                .base
                .verify_choices(&mut into_base_sink(sink), &mut into_base_stream(stream))
                .await?;

            let delta = <[u8; 16]>::from_lsb0_iter(choices).into();

            self.delta = Some(delta);

            delta
        };

        receiver
            .verify(index, delta, msgs)
            .map_err(ReceiverError::from)?;

        Ok(())
    }
}
