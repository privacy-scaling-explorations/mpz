//! General OT message types

use serde::{Deserialize, Serialize};

/// A message sent by the receiver which a sender can use to perform
/// Beaver derandomization.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Derandomize {
    /// Correction bits
    pub flip: Vec<u8>,
}
