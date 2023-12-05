//! Messages for the KOS15 protocol.

use enum_try_as_inner::EnumTryAsInner;
use mpz_core::{
    cointoss::msgs::{
        ReceiverPayload as CointossReceiverPayload, SenderCommitment,
        SenderPayload as CointossSenderPayload,
    },
    Block,
};
use serde::{Deserialize, Serialize};

use crate::msgs::Derandomize;

/// A KOS15 protocol message.
#[derive(Debug, Clone, EnumTryAsInner, Serialize, Deserialize)]
#[derive_err(Debug)]
#[allow(missing_docs)]
pub enum Message<BaseMsg> {
    BaseMsg(BaseMsg),
    Extend(Extend),
    Check(Check),
    Derandomize(Derandomize),
    SenderPayload(SenderPayload),
    CointossCommit(SenderCommitment),
    CointossReceiverPayload(CointossReceiverPayload),
    CointossSenderPayload(CointossSenderPayload),
}

impl<BaseMsg> From<MessageError<BaseMsg>> for std::io::Error {
    fn from(err: MessageError<BaseMsg>) -> Self {
        std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string())
    }
}

/// Extension message sent by the receiver.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Extend {
    /// The number of OTs to set up.
    pub count: usize,
    /// The receiver's extension vectors.
    pub us: Vec<u8>,
}

/// Values for the correlation check sent by the receiver.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(missing_docs)]
pub struct Check {
    pub x: Block,
    pub t0: Block,
    pub t1: Block,
}

/// Sender payload message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SenderPayload {
    /// Transfer ID
    pub id: u32,
    /// Sender's ciphertexts
    pub ciphertexts: Ciphertexts,
}

/// OT ciphertexts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Ciphertexts {
    /// Messages encrypted with XOR
    Blocks {
        /// Sender's ciphertexts
        ciphertexts: Vec<Block>,
    },
    /// Messages encrypted with stream cipher
    Bytes {
        /// Sender's ciphertexts
        ciphertexts: Vec<u8>,
        /// The IV used for encryption.
        iv: Vec<u8>,
        /// The length of each message in bytes.
        length: u32,
    },
}
