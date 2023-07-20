//! Coin-toss protocol messages.

use serde::{Deserialize, Serialize};

use crate::{commit::Decommitment, hash::Hash, Block};

/// A coin-toss message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(missing_docs)]
pub enum Message {
    SenderCommitments(SenderCommitments),
    SenderPayload(SenderPayload),
    ReceiverPayload(ReceiverPayload),
}

/// The coin-toss sender's commitments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SenderCommitments {
    /// The commitments to the random seeds.
    pub commitments: Vec<Hash>,
}

/// The coin-toss sender's payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SenderPayload {
    /// The decommitments to the random seeds.
    pub decommitments: Vec<Decommitment<Block>>,
}

/// The coin-toss receiver's payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiverPayload {
    /// The receiver's random seeds.
    pub seeds: Vec<Block>,
}
