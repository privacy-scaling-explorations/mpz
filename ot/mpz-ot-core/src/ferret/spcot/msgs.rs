//! Messages for the SPCOT protocol

use mpz_core::Block;
use serde::{Deserialize, Serialize};

/// A SPCOT message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(missing_docs)]
pub enum Message<BaseMsg> {
    BaseMsg(BaseMsg),
    ExtendFromCOT(ExtendFromCOT),
    MaskBits(MaskBits),
    CheckFromCOT(CheckFromCOT),
    CheckFromReceiver(CheckFromReceiver),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// The message received from the COT functionality.
pub struct ExtendFromCOT {
    /// The `q`s from the COT functionality.
    pub qs: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// The mask bits sent from the receiver.
pub struct MaskBits {
    /// The mask bits sent from the receiver.
    pub bs: Vec<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// The consistency check message received from the COT functionality.
pub struct CheckFromCOT {
    /// The `y*` message from the COT functionality.
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
