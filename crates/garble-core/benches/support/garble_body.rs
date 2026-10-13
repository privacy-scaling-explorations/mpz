//! Shared, target-agnostic body for the `mpz-garble-core` garbling benchmarks.
//!
//! Included via `#[path]` by `crates/garble-core/benches/garble.rs` and
//! `crates/wasm-bench/benches/garble-core/garble.rs`.

use std::hint::black_box;
use std::sync::Arc;

use mpz_circuits::{AES128, Circuit};
use mpz_garble_core::{Garbler, Key};
use mpz_memory_core::correlated::Delta;
use rand::{RngExt, SeedableRng, rngs::StdRng};

/// Gate-count thresholds shared by the native criterion bench and the browser
/// harness.
pub const THRESHOLDS: &[(u64, &str)] =
    &[(100_000, "100K"), (1_000_000, "1M"), (10_000_000, "10M")];

/// Half-gates garbling scenario.
pub struct GarbleBench {
    circuit: Arc<Circuit>,
    delta: Delta,
    inputs: Vec<Key>,
    iterations: usize,
    garbler: Garbler,
}

impl GarbleBench {
    pub fn new(iterations: usize) -> Self {
        let mut rng = StdRng::seed_from_u64(0);
        let delta = Delta::random(&mut rng);
        let inputs: Vec<Key> = (0..256).map(|_| rng.random()).collect();

        Self {
            circuit: AES128.clone(),
            delta,
            inputs,
            iterations,
            garbler: Garbler::default(),
        }
    }

    /// Iterator-based (one gate at a time).
    pub fn run_iter(&mut self) {
        for _ in 0..self.iterations {
            let mut iter = self
                .garbler
                .generate(&self.circuit, self.delta, &self.inputs)
                .unwrap();
            let _: Vec<_> = iter.by_ref().collect();
            black_box(iter.finish().unwrap());
        }
    }

    /// Batched (multiple gates at a time).
    pub fn run_batched(&mut self) {
        for _ in 0..self.iterations {
            let mut iter = self
                .garbler
                .generate_batched(&self.circuit, self.delta, &self.inputs)
                .unwrap();
            let _: Vec<_> = iter.by_ref().collect();
            black_box(iter.finish().unwrap());
        }
    }
}
