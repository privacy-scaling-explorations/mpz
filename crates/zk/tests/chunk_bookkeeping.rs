//! Fuzz test for zk chunked-execution OT bookkeeping.
//!
//! Gated behind the `counters` feature, which turns on cumulative
//! allocation/consumption counters on the ideal RCOT. Each case draws random
//! call sizes and `(batch_size, chunk_cap)`, runs a real prover/verifier pair
//! over an ideal RCOT, and checks the counters against an independent model of
//! the intended bookkeeping.
//!
//! Runs a bounded number of cases by default; set `ZK_FUZZ_CASES` to a larger
//! value for a more exhaustive sweep.

mod common;

use common::{CHECK_OTS, calibrate, reference_checks, run_case};
use rand::{RngExt, SeedableRng, rngs::StdRng};

#[tokio::test]
async fn chunk_ot_bookkeeping() {
    // Calibrate the constant per-side overhead (the reused private key's MAC).
    let (base_allocated, base_consumed) = calibrate().await;

    let mut rng = StdRng::seed_from_u64(1);

    // Bounded by default; set `ZK_FUZZ_CASES` for a more exhaustive sweep.
    let cases = std::env::var("ZK_FUZZ_CASES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(256);

    for case in 0..cases {
        let batch = 1 + (rng.random::<u32>() % 128) as usize;
        let chunk_cap = batch + (rng.random::<u32>() % 256) as usize;

        let call_count = 1 + (rng.random::<u32>() % 10) as usize;
        let sizes: Vec<usize> = (0..call_count)
            .map(|_| match rng.random::<u32>() % 6 {
                // Zero-gate call.
                0 => 0,
                // Call which on its own overshoots the chunk window.
                1 => chunk_cap + 1 + (rng.random::<u32>() % 64) as usize,
                _ => 1 + (rng.random::<u32>() % 64) as usize,
            })
            .collect();

        let counts = run_case(&sizes, batch, chunk_cap, rng.random()).await;

        assert_eq!(
            counts.prover, counts.verifier,
            "case {case}: prover/verifier counter mismatch\n  sizes={sizes:?} batch={batch} chunk_cap={chunk_cap}",
        );

        let total_gates: usize = sizes.iter().sum();
        let checks = reference_checks(&sizes, batch, chunk_cap);

        let expected_consumed = base_consumed + total_gates + CHECK_OTS * checks;
        let expected_allocated =
            base_allocated + total_gates + CHECK_OTS * total_gates.div_ceil(batch).max(checks);

        assert_eq!(
            counts.prover.1, expected_consumed,
            "case {case}: consumed mismatch\n  sizes={sizes:?} batch={batch} chunk_cap={chunk_cap} checks={checks}",
        );
        assert_eq!(
            counts.prover.0, expected_allocated,
            "case {case}: allocated mismatch\n  sizes={sizes:?} batch={batch} chunk_cap={chunk_cap} checks={checks}",
        );
    }
}
