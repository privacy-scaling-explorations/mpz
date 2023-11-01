//! An implementation of the [`Ferret`](https://eprint.iacr.org/2020/924.pdf) protocol.

pub mod mpcot;
pub mod spcot;
pub mod utils;

/// Computational security parameter
pub const CSP: usize = 128;

/// Number of hashes in Cuckoo hash.
pub const CUCKOO_HASH_NUM: usize = 3;

/// Trial numbers in Cuckoo hash insertion.
pub const CUCKOO_TRIAL_NUM: usize = 100;
