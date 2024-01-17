//! Implementations of oblivious transfer protocols.

#![deny(missing_docs, unreachable_pub, unused_must_use)]
#![deny(unsafe_code)]
#![deny(clippy::all)]

#[cfg(feature = "actor")]
pub mod actor;
pub mod chou_orlandi;
#[cfg(feature = "ideal")]
pub mod ideal;
pub mod kos;

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

/// An oblivious transfer protocol that needs to perform a one-time setup.
#[async_trait]
pub trait OTSetup: ProtocolMessage {
    /// Runs any one-time setup for the protocol.
    ///
    /// # Arguments
    ///
    /// * `sink` - The IO sink to the peer.
    /// * `stream` - The IO stream from the peer.
    async fn setup<Si: IoSink<Self::Msg> + Send + Unpin, St: IoStream<Self::Msg> + Send + Unpin>(
        &mut self,
        sink: &mut Si,
        stream: &mut St,
    ) -> Result<(), OTError>;
}

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

/// A correlated oblivious transfer sender.
#[async_trait]
pub trait COTSender<T>: ProtocolMessage
where
    T: Send + Sync,
{
    /// Obliviously transfers the correlated messages to the receiver.
    ///
    /// # Arguments
    ///
    /// * `sink` - The IO sink to the receiver.
    /// * `stream` - The IO stream from the receiver.
    /// * `msgs` - The `0`-bit messages to use during the oblivious transfer.
    async fn send_correlated<
        Si: IoSink<Self::Msg> + Send + Unpin,
        St: IoStream<Self::Msg> + Send + Unpin,
    >(
        &mut self,
        sink: &mut Si,
        stream: &mut St,
        msgs: &[T],
    ) -> Result<(), OTError>;
}

/// A random OT sender.
#[async_trait]
pub trait RandomOTSender<T>: ProtocolMessage
where
    T: Send + Sync,
{
    /// Outputs pairs of random messages.
    ///
    /// # Arguments
    ///
    /// * `sink` - The IO sink to the receiver.
    /// * `stream` - The IO stream from the receiver.
    /// * `count` - The number of pairs of random messages to output.
    async fn send_random<
        Si: IoSink<Self::Msg> + Send + Unpin,
        St: IoStream<Self::Msg> + Send + Unpin,
    >(
        &mut self,
        sink: &mut Si,
        stream: &mut St,
        count: usize,
    ) -> Result<Vec<T>, OTError>;
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

/// A correlated oblivious transfer receiver.
#[async_trait]
pub trait COTReceiver<T, U>: ProtocolMessage
where
    T: Send + Sync,
    U: Send + Sync,
{
    /// Obliviously receives correlated messages from the sender.
    ///
    /// # Arguments
    ///
    /// * `sink` - The IO sink to the sender.
    /// * `stream` - The IO stream from the sender.
    /// * `choices` - The choices made by the receiver.
    async fn receive_correlated<
        Si: IoSink<Self::Msg> + Send + Unpin,
        St: IoStream<Self::Msg> + Send + Unpin,
    >(
        &mut self,
        sink: &mut Si,
        stream: &mut St,
        choices: &[T],
    ) -> Result<Vec<U>, OTError>;
}

/// A random OT receiver.
#[async_trait]
pub trait RandomOTReceiver<T, U>: ProtocolMessage
where
    T: Send + Sync,
    U: Send + Sync,
{
    /// Outputs the choice bits and the corresponding messages.
    ///
    /// # Arguments
    ///
    /// * `sink` - The IO sink to the sender.
    /// * `stream` - The IO stream from the sender.
    /// * `count` - The number of random messages to receive.
    async fn receive_random<
        Si: IoSink<Self::Msg> + Send + Unpin,
        St: IoStream<Self::Msg> + Send + Unpin,
    >(
        &mut self,
        sink: &mut Si,
        stream: &mut St,
        count: usize,
    ) -> Result<(Vec<T>, Vec<U>), OTError>;
}

/// An oblivious transfer sender that is committed to its messages and can reveal them
/// to the receiver to verify them.
#[async_trait]
pub trait CommittedOTSender<T>: OTSender<T>
where
    T: Send + Sync,
{
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
pub trait VerifiableOTSender<T, U>: OTSender<U>
where
    U: Send + Sync,
{
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
    ) -> Result<Vec<T>, OTError>;
}

/// An oblivious transfer receiver that is committed to its choices and can reveal them
/// to the sender to verify them.
#[async_trait]
pub trait CommittedOTReceiver<T, U>: OTReceiver<T, U>
where
    T: Send + Sync,
    U: Send + Sync,
{
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
pub trait VerifiableOTReceiver<T, U, V>: OTReceiver<T, U>
where
    T: Send + Sync,
    U: Send + Sync,
    V: Send + Sync,
{
    /// Verifies purported messages sent by the sender.
    ///
    /// # Arguments
    ///
    /// * `sink` - The IO sink to the sender.
    /// * `stream` - The IO stream from the sender.
    /// * `id` - The transfer id of the messages to verify.
    /// * `msgs` - The purported messages sent by the sender.
    async fn verify<Si: IoSink<Self::Msg> + Send + Unpin, St: IoStream<Self::Msg> + Send + Unpin>(
        &mut self,
        sink: &mut Si,
        stream: &mut St,
        id: usize,
        msgs: &[V],
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

/// A correlated oblivious transfer sender that can be used via a shared reference.
#[async_trait]
pub trait COTSenderShared<T> {
    /// Obliviously transfers correlated messages to the receiver.
    ///
    /// # Arguments
    ///
    /// * `id` - The unique identifier for this transfer.
    /// * `msgs` - The `0`-bit messages to use during the oblivious transfer.
    async fn send_correlated(&self, id: &str, msgs: &[T]) -> Result<(), OTError>;
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

/// A correlated oblivious transfer receiver that can be used via a shared reference.
#[async_trait]
pub trait COTReceiverShared<T, U> {
    /// Obliviously receives correlated messages from the sender.
    ///
    /// # Arguments
    ///
    /// * `id` - The unique identifier for this transfer.
    /// * `choices` - The choices made by the receiver.
    async fn receive_correlated(&self, id: &str, choices: &[T]) -> Result<Vec<U>, OTError>;
}

/// An oblivious transfer sender that is committed to its messages and can reveal them
/// to the receiver to verify them.
#[async_trait]
pub trait CommittedOTSenderShared<T>: OTSenderShared<T> {
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
pub trait VerifiableOTReceiverShared<T, U, V>: OTReceiverShared<T, U> {
    /// Verifies purported messages sent by the sender.
    ///
    /// # Arguments
    ///
    /// * `id` - The unique identifier for the transfer corresponding to the messages.
    /// * `msgs` - The purported messages sent by the sender.
    async fn verify(&self, id: &str, msgs: &[V]) -> Result<(), OTError>;
}
