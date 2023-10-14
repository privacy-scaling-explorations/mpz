//! Messages for the SPCOT protocol

use mpz_core::{hash::Hash, Block};
use serde::{Deserialize, Serialize};

/// A SPCOT message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(missing_docs)]
pub enum Message<BaseMsg> {
    BaseMsg(BaseMsg),
    MaskBits(MaskBits),
    ExtendFromSender(ExtendFromSender),
    CotMsgForSender(CotMsgForSender),
    CotMsgForReceiver(CotMsgForReceiver),
    CheckFromReceiver(CheckFromReceiver),
    CheckFromSender(CheckFromSender),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// The message that sender receives from the COT functionality.
pub struct CotMsgForSender {
    /// The random blocks that sender receives from the COT functionality.
    pub qs: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// The message that receiver receives from the COT functionality.
pub struct CotMsgForReceiver {
    /// The random bits that receiver receives from the COT functionality.
    pub rs: Vec<bool>,
    /// The chosen blocks that receiver receivers from the COT functionality.
    pub ts: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// The mask bits sent from the receiver.
pub struct MaskBits {
    /// The mask bits sent from the receiver.
    pub bs: Vec<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// The extend messages that sent from the sender.
pub struct ExtendFromSender {
    /// The mask `m0` and `m1`.
    pub ms: Vec<[Block; 2]>,
    /// The sum of the ggm tree leaves and delta.
    pub sum: Block,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// The consistency check message sent from the receiver.
pub struct CheckFromReceiver {
    /// The random `chi`‘s from the receiver.
    pub chis: Vec<Block>,
    /// The `x'` from the receiver.
    pub x_prime: Vec<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// The consistency check message sent from the sender.
pub struct CheckFromSender {
    /// The hashed `V` from the sender.
    pub hashed_v: Hash,
}
