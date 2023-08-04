//! Mock implementations of the OT protocols.

mod owned;
mod shared;

pub use owned::{mock_ot_pair, MockOTReceiver, MockOTSender};
pub use shared::{mock_ot_shared_pair, MockSharedOTReceiver, MockSharedOTSender};
