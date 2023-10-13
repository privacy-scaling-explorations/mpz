//! Messages for the SPCOT protocol

use mpz_core::{hash::Hash, Block};
use serde::{Deserialize, Serialize};

/// A SPCOT message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(missing_docs)]
pub enum Message<BaseMsg> {
    BaseMsg(BaseMsg),
    ExtendSenderFromCOT(ExtendSenderFromCOT),
    ExtendReceiverFromCOT(ExtendReceiverFromCOT),
    MaskBits(MaskBits),
    ExtendFromSender(ExtendFromSender),
    CheckSenderFromCOT(CheckSenderFromCOT),
    CheckFromReceiver(CheckFromReceiver),
    CheckFromSender(CheckFromSender),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// The message that sender receives from the COT functionality.
pub struct ExtendSenderFromCOT {
    /// The `q`s that sender receives from the COT functionality.
    pub qs: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// The message that receiver receives from the COT functionality.
pub struct ExtendReceiverFromCOT {
    /// The `r`s that receiver receives from the COT functionality.
    pub rs: Vec<bool>,
    /// The `t`s that receiver receivers from the COT functionality.
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
    /// The mask `k0` and `k1`.
    pub ks: Vec<[Block; 2]>,
    /// The sum of the ggm tree leaves and delta.
    pub sum: Block,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// The consistency check message received from the COT functionality.
pub struct CheckSenderFromCOT {
    /// The `y*` message that sender receives from the COT functionality.
    pub y_star: Vec<Block>,
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
