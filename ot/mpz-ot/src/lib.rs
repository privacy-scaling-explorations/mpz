//! Implementations of oblivious transfer protocols.

#![deny(missing_docs, unreachable_pub, unused_must_use)]
#![deny(unsafe_code)]
#![deny(clippy::all)]

pub mod chou_orlandi;
#[cfg(feature = "mock")]
pub mod mock;

use async_trait::async_trait;
use mpz_core::ProtocolMessage;
use utils_aio::{sink::IoSink, stream::IoStream};

/// An oblivious transfer error.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum OTError {
    #[error(transparent)]
    IOError(#[from] std::io::Error),
    #[error("sender error: {0}")]
    SenderError(Box<dyn std::error::Error + Send + Sync>),
    #[error("receiver error: {0}")]
    ReceiverError(Box<dyn std::error::Error + Send + Sync>),
}

// ########################################################################
// ######################## Exclusive Reference ###########################
// ########################################################################

/// An oblivious transfer sender.
#[async_trait]
pub trait OTSender<T>: ProtocolMessage
where
    T: Send + Sync,
{
    /// Obliviously transfers the messages to the receiver.
    ///
    /// # Arguments
    ///
    /// * `sink` - The IO sink to the receiver.
    /// * `stream` - The IO stream from the receiver.
    /// * `msgs` - The messages to obliviously transfer.
    async fn send<Si: IoSink<Self::Msg> + Send + Unpin, St: IoStream<Self::Msg> + Send + Unpin>(
        &mut self,
        sink: &mut Si,
        stream: &mut St,
        msgs: &[T],
    ) -> Result<(), OTError>;
}

/// An oblivious transfer receiver.
#[async_trait]
pub trait OTReceiver<T, U>: ProtocolMessage
where
    T: Send + Sync,
    U: Send + Sync,
{
    /// Obliviously receives data from the sender.
    ///
    /// # Arguments
    ///
    /// * `sink` - The IO sink to the sender.
    /// * `stream` - The IO stream from the sender.
    /// * `choices` - The choices made by the receiver.
    async fn receive<Si: IoSink<Self::Msg> + Send + Unpin, St: IoStream<Self::Msg> + Send + Unpin>(
        &mut self,
        sink: &mut Si,
        stream: &mut St,
        choices: &[T],
    ) -> Result<Vec<U>, OTError>;
}

/// An oblivious transfer sender that can reveal its messages.
#[async_trait]
pub trait RevealMessages: ProtocolMessage {
    /// Reveals all messages sent to the receiver.
    ///
    /// # Warning
    ///
    /// Obviously, you should be sure you want to do this before calling this function!
    ///
    /// # Arguments
    ///
    /// * `sink` - The IO sink to the receiver.
    /// * `stream` - The IO stream from the receiver.
    async fn reveal<Si: IoSink<Self::Msg> + Send + Unpin, St: IoStream<Self::Msg> + Send + Unpin>(
        &mut self,
        sink: &mut Si,
        stream: &mut St,
    ) -> Result<(), OTError>;
}

/// An oblivious transfer sender that can verify the receiver's choices.
#[async_trait]
pub trait VerifyChoices<T>: ProtocolMessage {
    /// Receives the purported choices made by the receiver and verifies them.
    ///
    /// # Arguments
    ///
    /// * `sink` - The IO sink to the receiver.
    /// * `stream` - The IO stream from the receiver.
    async fn verify_choices<
        Si: IoSink<Self::Msg> + Send + Unpin,
        St: IoStream<Self::Msg> + Send + Unpin,
    >(
        &mut self,
        sink: &mut Si,
        stream: &mut St,
    ) -> Result<T, OTError>;
}

/// An oblivious transfer receiver that can reveal its choices.
#[async_trait]
pub trait RevealChoices: ProtocolMessage {
    /// Reveals the choices made by the receiver.
    ///
    /// # Warning
    ///
    /// Obviously, you should be sure you want to do this before calling this function!
    ///
    /// # Arguments
    ///
    /// * `sink` - The IO sink to the sender.
    /// * `stream` - The IO stream from the sender.
    async fn reveal_choices<
        Si: IoSink<Self::Msg> + Send + Unpin,
        St: IoStream<Self::Msg> + Send + Unpin,
    >(
        &mut self,
        sink: &mut Si,
        stream: &mut St,
    ) -> Result<(), OTError>;
}

/// An oblivious transfer receiver that can verify the sender's messages.
#[async_trait]
pub trait VerifyMessages<T>: ProtocolMessage
where
    T: Send + Sync,
{
    /// Verifies purported messages sent by the sender.
    ///
    /// # Arguments
    ///
    /// * `sink` - The IO sink to the sender.
    /// * `stream` - The IO stream from the sender.
    /// * `index` - The index of the messages to verify.
    /// * `msgs` - The purported messages sent by the sender.
    async fn verify<Si: IoSink<Self::Msg> + Send + Unpin, St: IoStream<Self::Msg> + Send + Unpin>(
        &mut self,
        sink: &mut Si,
        stream: &mut St,
        index: usize,
        msgs: &[T],
    ) -> Result<(), OTError>;
}

// ########################################################################
// ########################## Shared Reference ############################
// ########################################################################

/// An oblivious transfer sender that can be used via a shared reference.
#[async_trait]
pub trait OTSenderShared<T> {
    /// Obliviously transfers the messages to the receiver.
    ///
    /// # Arguments
    ///
    /// * `id` - The unique identifier for this transfer.
    /// * `msgs` - The messages to obliviously transfer.
    async fn send(&self, id: &str, msgs: &[T]) -> Result<(), OTError>;
}

/// An oblivious transfer receiver that can be used via a shared reference.
#[async_trait]
pub trait OTReceiverShared<T, U> {
    /// Obliviously receives data from the sender.
    ///
    /// # Arguments
    ///
    /// * `id` - The unique identifier for this transfer.
    /// * `choices` - The choices made by the receiver.
    async fn receive(&self, id: &str, choices: &[T]) -> Result<Vec<U>, OTError>;
}

/// An oblivious transfer sender that can reveal its messages and can be used via a shared reference.
#[async_trait]
pub trait RevealMessagesShared {
    /// Reveals all messages sent to the receiver.
    ///
    /// # Warning
    ///
    /// Obviously, you should be sure you want to do this before calling this function!
    ///
    /// This reveals **ALL** messages sent to the receiver, not just those for a specific transfer.
    async fn reveal(&self) -> Result<(), OTError>;
}

/// An oblivious transfer receiver that can verify the sender's messages and can be used via a shared reference.
#[async_trait]
pub trait VerifyMessagesShared<T> {
    /// Verifies purported messages sent by the sender.
    ///
    /// # Arguments
    ///
    /// * `id` - The unique identifier for the transfer corresponding to the messages.
    /// * `msgs` - The purported messages sent by the sender.
    async fn verify(&self, id: &str, msgs: &[T]) -> Result<(), OTError>;
}

// ########################################################################
// ############################## With IO #################################
// ########################################################################

/// An oblivious transfer sender that owns its own IO channels.
#[async_trait]
pub trait OTSenderWithIo<T>
where
    T: Send + Sync,
{
    /// Obliviously transfers the messages to the receiver.
    ///
    /// # Arguments
    ///
    /// * `msgs` - The messages to obliviously transfer.
    async fn send(&mut self, msgs: &[T]) -> Result<(), OTError>;
}

/// An oblivious transfer sender that owns its own IO channels, and
/// can reveal its messages.
#[async_trait]
pub trait RevealMessagesWithIo {
    /// Reveals all messages sent to the receiver.
    ///
    /// # Warning
    ///
    /// Obviously, you should be sure you want to do this before calling this function!
    async fn reveal(&mut self) -> Result<(), OTError>;
}

/// An oblivious transfer sender that owns its own IO Channels,
/// and can verify the receiver's choices.
#[async_trait]
pub trait VerifyChoicesWithIo<T> {
    /// Receives the purported choices made by the receiver and verifies them.
    async fn verify_choices(&mut self) -> Result<T, OTError>;
}

/// An oblivious transfer receiver that owns its own IO channels.
#[async_trait]
pub trait OTReceiverWithIo<T, U>
where
    T: Send + Sync,
    U: Send + Sync,
{
    /// Obliviously receives data from the sender.
    ///
    /// # Arguments
    ///
    /// * `choices` - The choices made by the receiver.
    async fn receive(&mut self, choices: &[T]) -> Result<Vec<U>, OTError>;
}

/// An oblivious transfer receiver that can reveal its choices.
#[async_trait]
pub trait RevealChoicesWithIo {
    /// Reveals the choices made by the receiver.
    ///
    /// # Warning
    ///
    /// Obviously, you should be sure you want to do this before calling this function!
    async fn reveal_choices(&mut self) -> Result<(), OTError>;
}

/// An oblivious transfer receiver that owns its own IO channels, and
/// can verify the sender's messages.
#[async_trait]
pub trait VerifyMessagesWithIo<T>
where
    T: Send + Sync,
{
    /// Verifies purported messages sent by the sender.
    ///
    /// # Arguments
    ///
    /// * `msgs` - The purported messages sent by the sender.
    async fn verify(&mut self, msgs: &[T]) -> Result<(), OTError>;
}
