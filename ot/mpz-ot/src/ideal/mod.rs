//! Ideal implementations of the OT protocols.

mod owned;
mod shared;

pub use owned::{
    ideal_cot_pair, ideal_ot_pair, IdealCOTReceiver, IdealCOTSender, IdealOTReceiver, IdealOTSender,
};
pub use shared::{
    ideal_cot_shared_pair, ideal_ot_shared_pair, IdealSharedCOTReceiver, IdealSharedCOTSender,
    IdealSharedOTReceiver, IdealSharedOTSender,
};
