//! Message types for the KOS actors.

use enum_try_as_inner::EnumTryAsInner;
use serde::{Deserialize, Serialize};

use mpz_ot_core::{
    kos::msgs::{Message as KosMessage, SenderPayload},
    msgs::Derandomize,
};

/// KOS actor message
#[derive(Debug, Clone, EnumTryAsInner, Serialize, Deserialize)]
#[derive_err(Debug)]
#[allow(missing_docs)]
pub enum Message<BaseOT> {
    ActorMessage(ActorMessage),
    Protocol(KosMessage<BaseOT>),
}

impl<BaseOT> From<MessageError<BaseOT>> for std::io::Error {
    fn from(err: MessageError<BaseOT>) -> Self {
        std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string())
    }
}

impl<T> From<ActorMessage> for Message<T> {
    fn from(value: ActorMessage) -> Self {
        Message::ActorMessage(value)
    }
}

/// KOS actor message
#[derive(Debug, Clone, EnumTryAsInner, Serialize, Deserialize)]
#[allow(missing_docs)]
pub enum ActorMessage {
    TransferRequest(TransferRequest),
    TransferPayload(TransferPayload),
    Reveal,
}

/// A message indicating that a transfer with the provided id is expected.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferRequest {
    /// The id of the transfer.
    pub id: String,
    /// Beaver-derandomization.
    pub derandomize: Derandomize,
}

/// A message containing a payload for a transfer with the provided id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferPayload {
    /// The id of the transfer.
    pub id: String,
    /// The payload.
    pub payload: SenderPayload,
}
