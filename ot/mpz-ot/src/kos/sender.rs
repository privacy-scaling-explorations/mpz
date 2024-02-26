use async_trait::async_trait;
use enum_try_as_inner::EnumTryAsInner;
use futures_util::SinkExt;
use itybity::IntoBits;
use mpz_core::{cointoss, prg::Prg, Block};
use mpz_ot_core::kos::{
    extension_matrix_size,
    msgs::{Extend, Message, StartExtend},
    pad_ot_count, sender_state as state, Sender as SenderCore, SenderConfig, CSP,
};
use rand::{thread_rng, Rng};
use rand_core::{RngCore, SeedableRng};
use utils_aio::{
    duplex::Duplex,
    non_blocking_backend::{Backend, NonBlockingBackend},
    stream::ExpectStreamExt,
};

use crate::{
    kos::SenderError, CommittedOTReceiver, CommittedOTSender, OTError, OTReceiver, OTSender,
    OTSetup, RandomOTSender,
};

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
pub struct Sender<Io, BaseOT> {
    io: Io,
    state: State,
    base: BaseOT,

    cointoss_payload: Option<cointoss::msgs::SenderPayload>,
}

impl<Io, BaseOT> Sender<Io, BaseOT>
where
    Io: Duplex<Message>,
    BaseOT: OTReceiver<bool, Block> + Send,
{
    /// Creates a new Sender
    ///
    /// # Arguments
    ///
    /// * `config` - The Sender's configuration
    /// * `io` - The IO channel used to communicate with the receiver
    /// * `base` - The base OT receiver used to perform the base OTs
    pub fn new(config: SenderConfig, io: Io, base: BaseOT) -> Self {
        Self {
            io,
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
    /// * `self.io` - The self.io to receive messages from the base OT sender
    /// * `delta` - The delta value to use for the base OT setup.
    pub async fn setup_with_delta(&mut self, delta: Block) -> Result<(), SenderError> {
        if self.state.try_as_initialized()?.config().sender_commit() {
            return Err(SenderError::ConfigError(
                "committed sender can not choose delta".to_string(),
            ));
        }

        self._setup_with_delta(delta).await
    }

    async fn _setup_with_delta(&mut self, delta: Block) -> Result<(), SenderError> {
        let ext_sender = std::mem::replace(&mut self.state, State::Error).try_into_initialized()?;

        let choices = delta.into_lsb0_vec();
        let seeds = self.base.receive(&choices).await?;

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
    pub async fn extend(&mut self, count: usize) -> Result<(), SenderError> {
        let mut ext_sender =
            std::mem::replace(&mut self.state, State::Error).try_into_extension()?;

        let count = pad_ot_count(count);

        let StartExtend {
            count: receiver_count,
        } = self
            .io
            .expect_next()
            .await?
            .try_into_start_extend()
            .map_err(SenderError::from)?;

        if count != receiver_count {
            return Err(SenderError::ConfigError(
                "sender and receiver count mismatch".to_string(),
            ));
        }

        let expected_us = extension_matrix_size(count);
        let mut extend = Extend {
            us: Vec::with_capacity(expected_us),
        };

        // Receive extension matrix from the receiver.
        while extend.us.len() < expected_us {
            let Extend { us: chunk } = self
                .io
                .expect_next()
                .await?
                .try_into_extend()
                .map_err(SenderError::from)?;

            extend.us.extend(chunk);
        }

        // Receive coin toss commitments from the receiver.
        let commitment = self.io.expect_next().await?.try_into_cointoss_commit()?;

        // Extend the OTs.
        let mut ext_sender =
            Backend::spawn(move || ext_sender.extend(count, extend).map(|_| ext_sender)).await?;

        // Execute coin toss protocol for consistency check.
        let seed: Block = thread_rng().gen();
        let cointoss_receiver = cointoss::Receiver::new(vec![seed]);

        let (cointoss_receiver, cointoss_payload) = cointoss_receiver.reveal(commitment)?;

        // Send coin toss payload to the receiver.
        self.io
            .send(Message::CointossReceiverPayload(cointoss_payload))
            .await?;

        // Receive coin toss sender payload from the receiver.
        let cointoss_sender_payload = self
            .io
            .expect_next()
            .await?
            .try_into_cointoss_sender_payload()?;

        // Receive consistency check from the receiver.
        let receiver_check = self.io.expect_next().await?.try_into_check()?;

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

impl<Io, BaseOT> Sender<Io, BaseOT>
where
    Io: Duplex<Message>,
    BaseOT: CommittedOTReceiver<bool, Block> + Send,
{
    pub(crate) async fn reveal(&mut self) -> Result<(), SenderError> {
        std::mem::replace(&mut self.state, State::Error).try_into_extension()?;

        // Reveal coin toss payload
        let Some(payload) = self.cointoss_payload.take() else {
            return Err(SenderError::ConfigError(
                "committed sender not configured".to_string(),
            ))?;
        };

        self.io
            .send(Message::CointossSenderPayload(payload))
            .await
            .map_err(SenderError::from)?;

        // Reveal base OT choices
        self.base.reveal_choices().await?;

        // This sender is no longer usable, so mark it as complete.
        self.state = State::Complete;

        Ok(())
    }
}

#[async_trait]
impl<Io, BaseOT> OTSetup for Sender<Io, BaseOT>
where
    Io: Duplex<Message>,
    BaseOT: OTSetup + OTReceiver<bool, Block> + Send,
{
    async fn setup(&mut self) -> Result<(), OTError> {
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

            self.io.send(Message::CointossCommit(commitment)).await?;
            let payload = self
                .io
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
        self.base.setup().await?;

        self.state = State::Initialized(sender);

        self._setup_with_delta(delta).await.map_err(OTError::from)
    }
}

#[async_trait]
impl<Io, BaseOT> OTSender<[Block; 2]> for Sender<Io, BaseOT>
where
    Io: Duplex<Message>,
    BaseOT: Send,
{
    async fn send(&mut self, msgs: &[[Block; 2]]) -> Result<(), OTError> {
        let sender = self
            .state
            .try_as_extension_mut()
            .map_err(SenderError::from)?;

        let derandomize = self
            .io
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

        self.io
            .send(Message::SenderPayload(payload))
            .await
            .map_err(SenderError::from)?;

        Ok(())
    }
}

#[async_trait]
impl<Io, BaseOT> RandomOTSender<[Block; 2]> for Sender<Io, BaseOT>
where
    Io: Duplex<Message>,
    BaseOT: Send,
{
    async fn send_random(&mut self, count: usize) -> Result<Vec<[Block; 2]>, OTError> {
        let sender = self
            .state
            .try_as_extension_mut()
            .map_err(SenderError::from)?;

        let random_outputs = sender.keys(count).map_err(SenderError::from)?;
        Ok(random_outputs.take_keys())
    }
}

#[async_trait]
impl<const N: usize, Io, BaseOT> OTSender<[[u8; N]; 2]> for Sender<Io, BaseOT>
where
    Io: Duplex<Message>,
    BaseOT: Send,
{
    async fn send(&mut self, msgs: &[[[u8; N]; 2]]) -> Result<(), OTError> {
        let sender = self
            .state
            .try_as_extension_mut()
            .map_err(SenderError::from)?;

        let derandomize = self
            .io
            .expect_next()
            .await?
            .try_into_derandomize()
            .map_err(SenderError::from)?;

        let mut sender_keys = sender.keys(msgs.len()).map_err(SenderError::from)?;
        sender_keys
            .derandomize(derandomize)
            .map_err(SenderError::from)?;
        let payload = sender_keys.encrypt_bytes(msgs).map_err(SenderError::from)?;

        self.io
            .send(Message::SenderPayload(payload))
            .await
            .map_err(SenderError::from)?;

        Ok(())
    }
}

#[async_trait]
impl<const N: usize, Io, BaseOT> RandomOTSender<[[u8; N]; 2]> for Sender<Io, BaseOT>
where
    Io: Duplex<Message>,
    BaseOT: Send,
{
    async fn send_random(&mut self, count: usize) -> Result<Vec<[[u8; N]; 2]>, OTError> {
        let sender = self
            .state
            .try_as_extension_mut()
            .map_err(SenderError::from)?;

        let random_outputs = sender.keys(count).map_err(SenderError::from)?;

        let prng = |block| {
            let mut prg = Prg::from_seed(block);
            let mut out = [0_u8; N];
            prg.fill_bytes(&mut out);
            out
        };

        Ok(random_outputs
            .take_keys()
            .into_iter()
            .map(|[a, b]| [prng(a), prng(b)])
            .collect())
    }
}

#[async_trait]
impl<Io, BaseOT> CommittedOTSender<[Block; 2]> for Sender<Io, BaseOT>
where
    Io: Duplex<Message>,
    BaseOT: CommittedOTReceiver<bool, Block> + Send,
{
    async fn reveal(&mut self) -> Result<(), OTError> {
        self.reveal().await.map_err(OTError::from)
    }
}
