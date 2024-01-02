//! An implementation of the [`Ferret`](https://eprint.iacr.org/2020/924.pdf) protocol.

use mpz_core::lpn::LpnParameters;

pub mod error;
pub mod mpcot;
pub mod msgs;
pub mod receiver;
pub mod sender;
pub mod spcot;
pub mod utils;

/// Computational security parameter
pub const CSP: usize = 128;

/// Number of hashes in Cuckoo hash.
pub const CUCKOO_HASH_NUM: usize = 3;

/// Trial numbers in Cuckoo hash insertion.
pub const CUCKOO_TRIAL_NUM: usize = 100;

/// Large LPN parameters
/// Derived from https://github.com/emp-toolkit/emp-ot/blob/master/emp-ot/ferret/constants.h
pub const LPN_PARAMETERS_LARGE: LpnParameters = LpnParameters {
    n: 10180608,
    k: 124000,
    t: 4971,
};

/// Medium LPN parameters.
/// Derived from https://github.com/emp-toolkit/emp-ot/blob/master/emp-ot/ferret/constants.h
pub const LPN_PARAMETERS_MEDIUM: LpnParameters = LpnParameters {
    n: 470016,
    k: 32768,
    t: 918,
};

/// Small LPN parameters.
/// Derived from https://github.com/emp-toolkit/emp-ot/blob/master/emp-ot/ferret/constants.h
pub const LPN_PARAMETERS_SMALL: LpnParameters = LpnParameters {
    n: 178944,
    k: 17384,
    t: 699,
};

/// The type of Lpn parameters.
pub enum LpnType {
    /// Uniform error distribution.
    Uniform,
    /// Regular error distribution.
    Regular,
}
