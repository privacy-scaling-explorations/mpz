use async_trait::async_trait;
use futures::SinkExt;
use itybity::{FromBitIterator, IntoBitIterator};
use mpz_core::{cointoss, prg::Prg, Block};
use mpz_ot_core::kos::{
    msgs::{Message, StartExtend},
    pad_ot_count, receiver_state as state, Receiver as ReceiverCore, ReceiverConfig, CSP,
};

use enum_try_as_inner::EnumTryAsInner;
use rand::{thread_rng, Rng};
use rand_core::{RngCore, SeedableRng};
use utils_aio::{
    duplex::Duplex,
    non_blocking_backend::{Backend, NonBlockingBackend},
    stream::ExpectStreamExt,
};

use super::{ReceiverError, ReceiverVerifyError, EXTEND_CHUNK_SIZE};
use crate::{
    OTError, OTReceiver, OTSender, OTSetup, RandomOTReceiver, VerifiableOTReceiver,
    VerifiableOTSender,
};

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
pub struct Receiver<Io, BaseOT> {
    io: Io,
    state: State,
    base: BaseOT,

    cointoss_receiver: Option<cointoss::Receiver<cointoss::receiver_state::Received>>,
}

impl<Io, BaseOT> Receiver<Io, BaseOT>
where
    Io: Duplex<Message>,
    BaseOT: OTSender<[Block; 2]> + Send,
{
    /// Creates a new receiver.
    ///
    /// # Arguments
    ///
    /// * `config` - The receiver's configuration
    pub fn new(config: ReceiverConfig, io: Io, base: BaseOT) -> Self {
        Self {
            io,
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
    pub async fn extend(&mut self, count: usize) -> Result<(), ReceiverError> {
        let mut ext_receiver =
            std::mem::replace(&mut self.state, State::Error).try_into_extension()?;

        let count = pad_ot_count(count);

        // Extend the OTs.
        let (mut ext_receiver, extend) = Backend::spawn(move || {
            let extend = ext_receiver.extend(count);

            (ext_receiver, extend)
        })
        .await;

        let extend = extend?;

        // Commit to coin toss seed
        let seed: Block = thread_rng().gen();
        let (cointoss_sender, cointoss_commitment) = cointoss::Sender::new(vec![seed]).send();

        // Send the extend message and cointoss commitment
        self.io
            .feed(Message::StartExtend(StartExtend { count }))
            .await?;
        for extend in extend.into_chunks(EXTEND_CHUNK_SIZE) {
            self.io.feed(Message::Extend(extend)).await?;
        }
        self.io
            .feed(Message::CointossCommit(cointoss_commitment))
            .await?;
        self.io.flush().await?;

        // Receive coin toss
        let cointoss_payload = self
            .io
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
        self.io
            .feed(Message::CointossSenderPayload(payload))
            .await?;
        self.io.feed(Message::Check(check)).await?;
        self.io.flush().await?;

        self.state = State::Extension(ext_receiver);

        Ok(())
    }
}

impl<Io, BaseOT> Receiver<Io, BaseOT>
where
    Io: Duplex<Message>,
    BaseOT: VerifiableOTSender<bool, [Block; 2]> + Send,
{
    pub(crate) async fn verify_delta(&mut self) -> Result<(), ReceiverError> {
        let receiver = std::mem::replace(&mut self.state, State::Error).try_into_extension()?;

        // Finalize coin toss to determine expected delta
        let cointoss_payload = self
            .io
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
        let choices = self.base.verify_choices().await?;

        let actual_delta = <[u8; 16]>::from_lsb0_iter(choices).into();

        if expected_delta != actual_delta {
            return Err(ReceiverError::from(ReceiverVerifyError::InconsistentDelta));
        }

        self.state = State::Verify(receiver.start_verification(actual_delta)?);

        Ok(())
    }
}

#[async_trait]
impl<Io, BaseOT> OTSetup for Receiver<Io, BaseOT>
where
    Io: Duplex<Message>,
    BaseOT: OTSetup + OTSender<[Block; 2]> + Send,
{
    async fn setup(&mut self) -> Result<(), OTError> {
        if self.state.is_extension() {
            return Ok(());
        }

        let ext_receiver = std::mem::replace(&mut self.state, State::Error)
            .try_into_initialized()
            .map_err(ReceiverError::from)?;

        // If the sender is committed, we run a coin toss
        if ext_receiver.config().sender_commit() {
            let commitment = self
                .io
                .expect_next()
                .await?
                .try_into_cointoss_commit()
                .map_err(ReceiverError::from)?;

            let (cointoss_receiver, payload) = cointoss::Receiver::new(vec![thread_rng().gen()])
                .reveal(commitment)
                .map_err(ReceiverError::from)?;

            self.io
                .send(Message::CointossReceiverPayload(payload))
                .await?;

            self.cointoss_receiver = Some(cointoss_receiver);
        }

        // Set up base OT
        self.base.setup().await?;

        let seeds: [[Block; 2]; CSP] = std::array::from_fn(|_| thread_rng().gen());

        // Send seeds to sender
        self.base.send(&seeds).await?;

        let ext_receiver = ext_receiver.setup(seeds);

        self.state = State::Extension(Box::new(ext_receiver));

        Ok(())
    }
}

#[async_trait]
impl<Io, BaseOT> OTReceiver<bool, Block> for Receiver<Io, BaseOT>
where
    Io: Duplex<Message>,
    BaseOT: Send,
{
    async fn receive(&mut self, choices: &[bool]) -> Result<Vec<Block>, OTError> {
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
        self.io.send(Message::Derandomize(derandomize)).await?;

        // Receive payload
        let payload = self
            .io
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
impl<Io, BaseOT> RandomOTReceiver<bool, Block> for Receiver<Io, BaseOT>
where
    Io: Duplex<Message>,
    BaseOT: Send,
{
    async fn receive_random(&mut self, count: usize) -> Result<(Vec<bool>, Vec<Block>), OTError> {
        let receiver = self
            .state
            .try_as_extension_mut()
            .map_err(ReceiverError::from)?;

        let (choices, random_outputs) = receiver
            .keys(count)
            .map_err(ReceiverError::from)?
            .take_choices_and_keys();

        Ok((choices, random_outputs))
    }
}

#[async_trait]
impl<const N: usize, Io, BaseOT> OTReceiver<bool, [u8; N]> for Receiver<Io, BaseOT>
where
    Io: Duplex<Message>,
    BaseOT: Send,
{
    async fn receive(&mut self, choices: &[bool]) -> Result<Vec<[u8; N]>, OTError> {
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
        self.io.send(Message::Derandomize(derandomize)).await?;

        // Receive payload
        let payload = self
            .io
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
impl<const N: usize, Io, BaseOT> RandomOTReceiver<bool, [u8; N]> for Receiver<Io, BaseOT>
where
    Io: Duplex<Message>,
    BaseOT: Send,
{
    async fn receive_random(&mut self, count: usize) -> Result<(Vec<bool>, Vec<[u8; N]>), OTError> {
        let receiver = self
            .state
            .try_as_extension_mut()
            .map_err(ReceiverError::from)?;

        let (choices, random_outputs) = receiver
            .keys(count)
            .map_err(ReceiverError::from)?
            .take_choices_and_keys();

        Ok((
            choices,
            random_outputs
                .into_iter()
                .map(|block| {
                    let mut prg = Prg::from_seed(block);
                    let mut out = [0_u8; N];
                    prg.fill_bytes(&mut out);
                    out
                })
                .collect(),
        ))
    }
}

#[async_trait]
impl<Io, BaseOT> VerifiableOTReceiver<bool, Block, [Block; 2]> for Receiver<Io, BaseOT>
where
    Io: Duplex<Message>,
    BaseOT: VerifiableOTSender<bool, [Block; 2]> + Send,
{
    async fn verify(&mut self, id: usize, msgs: &[[Block; 2]]) -> Result<(), OTError> {
        // Verify delta if we haven't yet.
        if self.state.is_extension() {
            self.verify_delta().await?;
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
