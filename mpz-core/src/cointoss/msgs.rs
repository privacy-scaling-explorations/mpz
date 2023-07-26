//! Coin-toss protocol messages.

use serde::{Deserialize, Serialize};

use crate::{commit::Decommitment, hash::Hash, Block};

/// A coin-toss message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(missing_docs)]
pub enum Message {
    SenderCommitments(SenderCommitment),
    SenderPayload(SenderPayload),
    ReceiverPayload(ReceiverPayload),
}

/// The coin-toss sender's commitment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SenderCommitment {
    /// The commitment to the random seeds.
    pub commitment: Hash,
}

/// The coin-toss sender's payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SenderPayload {
    /// The decommitment to the random seeds.
    pub decommitment: Decommitment<Vec<Block>>,
}

/// The coin-toss receiver's payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiverPayload {
    /// The receiver's random seeds.
    pub seeds: Vec<Block>,
}
