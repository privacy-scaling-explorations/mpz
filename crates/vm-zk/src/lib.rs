use std::collections::BTreeMap;

use mpz_vm_core::{Trap, value::Value};
use mpz_zk_core::Proof;

pub(crate) mod capture;
pub(crate) mod commit;
pub(crate) mod config;
pub(crate) mod cost;
pub(crate) mod error;
pub(crate) mod finalize;
pub(crate) mod host;
pub(crate) mod memlog;
mod prover;
pub(crate) mod replay;
pub(crate) mod reveal;
pub(crate) mod segment;
mod verifier;

use host::RevealPayload;

pub use config::{Config, ConfigBuilder};
pub use error::ZkVmError;
pub use prover::Prover;
pub use verifier::Verifier;

pub(crate) const VOPE_BITS: usize = 128;

pub const DEFAULT_CHUNK_CAP: usize = 15_000_000;

pub(crate) const TARGET_SEGMENTS: usize = 64;

pub(crate) const MIN_SEGMENT_COST: usize = 50_000;

pub(crate) fn effective_segment_cost(
    segment_cost: Option<usize>,
    chunk_cap: Option<usize>,
) -> Option<usize> {
    match segment_cost {
        Some(cost) => Some(cost),
        None => chunk_cap.map(|cap| (cap / TARGET_SEGMENTS).max(MIN_SEGMENT_COST)),
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct ProofMessage {
    pub(crate) output: Option<Value>,
    pub(crate) revealed: Vec<u8>,
    pub(crate) proof: Proof,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct ChunkOutcome {
    pub(crate) trap_at: Option<u64>,
    pub(crate) trap: Option<Trap>,
    pub(crate) revealed: BTreeMap<u32, RevealPayload>,
}

#[cfg(test)]
mod tests {
    use super::DEFAULT_CHUNK_CAP;

    #[test]
    fn default_chunk_cap_fits_ferret_iteration() {
        const CSP: usize = 128;
        let net = mpz_ot_core::ferret::REGULAR_PARAMS
            .iter()
            .map(|p| {
                let iteration_cost = p.t * (p.n / p.t).ilog2() as usize + p.k + CSP;
                p.n - iteration_cost
            })
            .max()
            .expect("Ferret defines at least one LPN parameter set");
        assert!(
            DEFAULT_CHUNK_CAP <= net,
            "DEFAULT_CHUNK_CAP {DEFAULT_CHUNK_CAP} exceeds Ferret net yield {net}"
        );
    }
}
