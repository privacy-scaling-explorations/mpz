//! Oblivious transfer protocols.

#![deny(
    unsafe_code,
    missing_docs,
    unused_imports,
    unused_must_use,
    unreachable_pub,
    clippy::all
)]

pub mod chou_orlandi;
pub mod cot;
pub mod ferret;
#[cfg(any(test, feature = "ideal"))]
pub mod ideal;
pub mod ot;
pub mod rcot;
pub mod rot;
pub mod softspoken;
#[cfg(any(test, feature = "test-utils"))]
pub mod test;

pub use mpz_ot_core::TransferId;
