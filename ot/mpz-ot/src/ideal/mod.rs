//! Ideal implementations of the OT protocols.

mod owned;
mod shared;

pub use owned::{ideal_ot_pair, IdealOTReceiver, IdealOTSender};
pub use shared::{ideal_ot_shared_pair, IdealSharedOTReceiver, IdealSharedOTSender};
