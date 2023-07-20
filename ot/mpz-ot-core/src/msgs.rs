//! General OT message types

use enum_try_as_inner::EnumTryAsInner;
use serde::{Deserialize, Serialize};

/// A wrapper enum containing all OT protocol messages.
#[derive(Debug, Clone, EnumTryAsInner, Serialize, Deserialize)]
pub enum OTMessage {
    /// CO15 protocol messages
    CO15(crate::chou_orlandi::msgs::Message),
}

// /// OT extension Sender plays the role of base OT Receiver and sends the
// /// second message containing base OT setup and cointoss share
// #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
// pub struct BaseReceiverSetupWrapper {
//     pub setup: ReceiverSetup,
//     // Cointoss protocol's 2nd message: Receiver reveals share
//     pub cointoss_share: [u8; 32],
// }

// /// OT extension Receiver plays the role of base OT Sender and sends the
// /// first message containing base OT setup and cointoss commitment
// #[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
// pub struct BaseSenderSetupWrapper {
//     pub setup: SenderSetup,
//     // Cointoss protocol's 1st message: blake3 commitment
//     pub cointoss_commit: [u8; 32],
// }

// #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
// pub struct BaseSenderPayloadWrapper {
//     pub payload: SenderPayload,
//     // Cointoss protocol's 3rd message: Sender reveals share
//     pub cointoss_share: [u8; 32],
// }

// #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
// pub struct ExtSenderPayload {
//     pub ciphertexts: Vec<[Block; 2]>,
// }

// #[derive(Debug, Clone, Serialize, Deserialize)]
// pub struct ExtDerandomize {
//     pub flip: Vec<bool>,
// }

// #[derive(Debug, Clone, Serialize, Deserialize)]
// pub struct ExtReceiverSetup {
//     /// The unpadded number of OTs the receiver has prepared
//     pub count: usize,
//     pub table: Vec<u8>,
//     // x, t0, t1 are used for the KOS15 check
//     pub x: [u8; 16],
//     pub t0: [u8; 16],
//     pub t1: [u8; 16],
// }

// #[derive(Debug, Clone, Serialize, Deserialize)]
// pub struct ExtSenderCommit(pub [u8; 32]);

// #[derive(Debug, Clone, Serialize, Deserialize)]
// pub struct ExtSenderReveal {
//     pub seed: [u8; 32],
//     pub salt: [u8; 32],
//     pub offset: usize,
// }

// /// We use this message when we want to send data which is longer than 128 bits
// ///
// /// This is an encrypted payload which can be decrypted by the receiver with keys
// /// he receives during the OT
// #[derive(Debug, Clone, Serialize, Deserialize)]
// pub struct ExtSenderEncryptedPayload {
//     pub ciphertexts: Vec<u8>,
// }

// #[derive(Debug, Clone, Serialize, Deserialize)]
// pub struct Split {
//     /// Child OT identifier
//     pub id: String,
//     /// Number of OTs
//     pub count: usize,
// }
