//! Messages for the Ferret protocol.
use mpz_core::Block;
use serde::{Deserialize, Serialize};

/// A Ferret message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(missing_docs)]
pub enum Message<CotMsg, MpcotMsg> {
    CotMsg(CotMsg),
    MpcotMsg(MpcotMsg),
    LpnMatrixSeed(LpnMatrixSeed),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// The seed to generate Lpn matrix.
pub struct LpnMatrixSeed {
    /// The seed.
    pub seed: Block,
}
