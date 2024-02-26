//! Implementations of oblivious transfer protocols.

#![deny(missing_docs, unreachable_pub, unused_must_use)]
#![deny(unsafe_code)]
#![deny(clippy::all)]

// #[cfg(feature = "actor")]
// pub mod actor;
pub mod chou_orlandi;
#[cfg(feature = "ideal")]
pub mod ideal;
//pub mod kos;
use async_trait::async_trait;

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
pub trait OTSetup {
    /// Runs any one-time setup for the protocol.
    async fn setup(&mut self) -> Result<(), OTError>;
}

/// An oblivious transfer sender.
#[async_trait]
pub trait OTSender<T>
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

/// A correlated oblivious transfer sender.
#[async_trait]
pub trait COTSender<T>
where
    T: Send + Sync,
{
    /// Obliviously transfers the correlated messages to the receiver.
    ///
    /// # Arguments
    ///
    /// * `msgs` - The `0`-bit messages to use during the oblivious transfer.
    async fn send_correlated(&mut self, msgs: &[T]) -> Result<(), OTError>;
}

/// A random OT sender.
#[async_trait]
pub trait RandomOTSender<T>
where
    T: Send + Sync,
{
    /// Outputs pairs of random messages.
    ///
    /// # Arguments
    ///
    /// * `count` - The number of pairs of random messages to output.
    async fn send_random(&mut self, count: usize) -> Result<Vec<T>, OTError>;
}

/// A random correlated oblivious transfer sender.
#[async_trait]
pub trait RandomCOTSender<T>
where
    T: Send + Sync,
{
    /// Obliviously transfers the correlated messages to the receiver.
    ///
    /// Returns the `0`-bit messages that were obliviously transferred.
    ///
    /// # Arguments
    ///
    /// * `count` - The number of correlated messages to obliviously transfer.
    async fn send_random_correlated(&mut self, count: usize) -> Result<Vec<T>, OTError>;
}

/// An oblivious transfer receiver.
#[async_trait]
pub trait OTReceiver<T, U>
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

/// A correlated oblivious transfer receiver.
#[async_trait]
pub trait COTReceiver<T, U>
where
    T: Send + Sync,
    U: Send + Sync,
{
    /// Obliviously receives correlated messages from the sender.
    ///
    /// # Arguments
    ///
    /// * `choices` - The choices made by the receiver.
    async fn receive_correlated(&mut self, choices: &[T]) -> Result<Vec<U>, OTError>;
}

/// A random OT receiver.
#[async_trait]
pub trait RandomOTReceiver<T, U>
where
    T: Send + Sync,
    U: Send + Sync,
{
    /// Outputs the choice bits and the corresponding messages.
    ///
    /// # Arguments
    ///
    /// * `count` - The number of random messages to receive.
    async fn receive_random(&mut self, count: usize) -> Result<(Vec<T>, Vec<U>), OTError>;
}

/// A random correlated oblivious transfer receiver.
#[async_trait]
pub trait RandomCOTReceiver<T, U>
where
    T: Send + Sync,
    U: Send + Sync,
{
    /// Obliviously receives correlated messages with random choices.
    ///
    /// Returns a tuple of the choices and the messages, respectively.
    ///
    /// # Arguments
    ///
    /// * `count` - The number of correlated messages to obliviously receive.
    async fn receive_random_correlated(
        &mut self,
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
    async fn reveal(&mut self) -> Result<(), OTError>;
}

/// An oblivious transfer sender that can verify the receiver's choices.
#[async_trait]
pub trait VerifiableOTSender<T, U>: OTSender<U>
where
    U: Send + Sync,
{
    /// Receives the purported choices made by the receiver and verifies them.
    async fn verify_choices(&mut self) -> Result<Vec<T>, OTError>;
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
    async fn reveal_choices(&mut self) -> Result<(), OTError>;
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
    /// * `id` - The transfer id of the messages to verify.
    /// * `msgs` - The purported messages sent by the sender.
    async fn verify(&mut self, id: usize, msgs: &[V]) -> Result<(), OTError>;
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

/// A random oblivious transfer sender that can be used via a shared reference.
#[async_trait]
pub trait RandomOTSenderShared<T> {
    /// Outputs pairs of random messages.
    ///
    /// # Arguments
    ///
    /// * `id` - The unique identifier for this transfer.
    /// * `count` - The number of pairs of random messages to output.
    async fn send_random(&self, id: &str, count: usize) -> Result<Vec<T>, OTError>;
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

/// A random correlated oblivious transfer sender that can be used via a shared reference.
#[async_trait]
pub trait RandomCOTSenderShared<T> {
    /// Obliviously transfers correlated messages to the receiver.
    ///
    /// Returns the `0`-bit messages that were obliviously transferred.
    ///
    /// # Arguments
    ///
    /// * `id` - The unique identifier for this transfer.
    /// * `count` - The number of correlated messages to obliviously transfer.
    async fn send_random_correlated(&self, id: &str, count: usize) -> Result<Vec<T>, OTError>;
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

/// A random oblivious transfer receiver that can be used via a shared reference.
#[async_trait]
pub trait RandomOTReceiverShared<T, U> {
    /// Outputs the choice bits and the corresponding messages.
    ///
    /// # Arguments
    ///
    /// * `id` - The unique identifier for this transfer.
    /// * `count` - The number of random messages to receive.
    async fn receive_random(&self, id: &str, count: usize) -> Result<(Vec<T>, Vec<U>), OTError>;
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

/// A random correlated oblivious transfer receiver that can be used via a shared reference.
#[async_trait]
pub trait RandomCOTReceiverShared<T, U> {
    /// Obliviously receives correlated messages with random choices.
    ///
    /// Returns a tuple of the choices and the messages, respectively.
    ///
    /// # Arguments
    ///
    /// * `id` - The unique identifier for this transfer.
    /// * `count` - The number of correlated messages to obliviously receive.
    async fn receive_random_correlated(
        &self,
        id: &str,
        count: usize,
    ) -> Result<(Vec<T>, Vec<U>), OTError>;
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
