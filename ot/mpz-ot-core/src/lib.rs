//! Low-level crate containing core functionalities for oblivious transfer protocols.
//!
//! This crate is not intended to be used directly. Instead, use the higher-level APIs provided by
//! the `mpz-ot` crate.
//!
//! # ⚠️ Warning ⚠️
//!
//! Some implementations make assumptions about invariants which may not be checked if using these
//! low-level APIs naively. Failing to uphold these invariants may result in security vulnerabilities.
//!
//! USE AT YOUR OWN RISK.

#![deny(missing_docs, unreachable_pub, unused_must_use)]
#![deny(unsafe_code)]
#![deny(clippy::all)]

pub mod chou_orlandi;
pub mod ferret;
pub mod ideal;
pub mod kos;
pub mod msgs;
