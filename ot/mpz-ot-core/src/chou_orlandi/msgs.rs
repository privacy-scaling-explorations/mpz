//! Messages for the Chou-Orlandi protocol.

use curve25519_dalek::RistrettoPoint;
use enum_try_as_inner::EnumTryAsInner;
use mpz_core::{cointoss, Block};
use serde::{Deserialize, Serialize};

/// A CO15 protocol message.
#[derive(Debug, Clone, EnumTryAsInner, Serialize, Deserialize)]
#[derive_err(Debug)]
#[allow(missing_docs)]
pub enum Message {
    SenderSetup(SenderSetup),
    SenderPayload(SenderPayload),
    ReceiverPayload(ReceiverPayload),
    ReceiverReveal(ReceiverReveal),
    CointossSenderCommitment(cointoss::msgs::SenderCommitment),
    CointossSenderPayload(cointoss::msgs::SenderPayload),
    CointossReceiverPayload(cointoss::msgs::ReceiverPayload),
}

impl From<MessageError> for std::io::Error {
    fn from(err: MessageError) -> Self {
        std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string())
    }
}

/// Sender setup message.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SenderSetup {
    /// The sender's public key
    pub public_key: RistrettoPoint,
}

/// Sender payload message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SenderPayload {
    /// The sender's ciphertexts
    pub payload: Vec<[Block; 2]>,
}

/// Receiver payload message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReceiverPayload {
    /// The receiver's blinded choices.
    pub blinded_choices: Vec<RistrettoPoint>,
}

/// Receiver reveal message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiverReveal {
    /// The receiver's choices.
    pub choices: Vec<u8>,
}
