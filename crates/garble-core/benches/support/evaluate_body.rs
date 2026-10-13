//! Shared, target-agnostic body for the `mpz-garble-core` evaluation
//! benchmarks.
//!
//! Included via `#[path]` by `crates/garble-core/benches/evaluate.rs` and
//! `crates/wasm-bench/benches/garble-core/evaluate.rs`.

use std::{hint::black_box, sync::Arc};

use mpz_circuits::{AES128, Circuit};
use mpz_garble_core::{
    EncryptedGate, Evaluator, GarbledCircuit, Garbler, Key, Mac, evaluate_garbled_circuits,
};
use mpz_memory_core::correlated::Delta;
use rand::{RngExt, SeedableRng, rngs::StdRng};

/// Gate-count thresholds shared by the native criterion bench and the browser
/// harness.
pub const THRESHOLDS: &[(u64, &str)] = &[(100_000, "100K"), (1_000_000, "1M"), (10_000_000, "10M")];

/// Prepared argument for [`evaluate`].
pub type GarbledInput = (Arc<Circuit>, Vec<Mac>, GarbledCircuit);

fn inputs_and_eval_inputs(delta: &Delta) -> (Vec<Key>, Vec<Mac>) {
    let mut rng = StdRng::seed_from_u64(0);
    let inputs: Vec<Key> = (0..256).map(|_| rng.random()).collect();
    let choices: Vec<bool> = (0..256).map(|_| rng.random()).collect();
    let eval_inputs: Vec<Mac> = inputs
        .iter()
        .zip(&choices)
        .map(|(k, &c)| k.auth(c, delta))
        .collect();
    (inputs, eval_inputs)
}

/// Half-gates evaluation scenario (iter + batched).
pub struct EvaluateBench {
    circuit: Arc<Circuit>,
    delta: Delta,
    inputs: Vec<Key>,
    eval_inputs: Vec<Mac>,
    iterations: usize,
    all_gates: Vec<Vec<EncryptedGate>>,
    evaluator: Evaluator,
    garbler: Garbler,
}

impl EvaluateBench {
    pub fn new(iterations: usize) -> Self {
        let mut rng = StdRng::seed_from_u64(0);
        let delta = Delta::random(&mut rng);
        let (inputs, eval_inputs) = inputs_and_eval_inputs(&delta);

        let circuit = AES128.clone();

        // Pre-generate garbled circuits (single gates).
        let mut gb = Garbler::default();
        let all_gates: Vec<Vec<EncryptedGate>> = (0..iterations)
            .map(|_| {
                let mut iter = gb.generate(&circuit, delta, &inputs).unwrap();
                let gates: Vec<_> = iter.by_ref().collect();
                let _ = iter.finish().unwrap();
                gates
            })
            .collect();

        Self {
            circuit,
            delta,
            inputs,
            eval_inputs,
            iterations,
            all_gates,
            evaluator: Evaluator::default(),
            garbler: Garbler::default(),
        }
    }

    /// Iterator-based (one gate at a time).
    pub fn run_iter(&mut self) {
        for gates in &self.all_gates {
            let mut consumer = self
                .evaluator
                .evaluate(&self.circuit, &self.eval_inputs)
                .unwrap();
            for gate in gates {
                consumer.next(*gate);
            }
            black_box(consumer.finish().unwrap());
        }
    }

    /// Batched (multiple gates at a time).
    pub fn run_batched(&mut self) {
        for _ in 0..self.iterations {
            let mut iter = self
                .garbler
                .generate_batched(&self.circuit, self.delta, &self.inputs)
                .unwrap();
            let batches: Vec<_> = iter.by_ref().collect();
            let _ = iter.finish().unwrap();

            let mut consumer = self
                .evaluator
                .evaluate_batched(&self.circuit, &self.eval_inputs)
                .unwrap();
            for batch in batches {
                consumer.next(batch);
            }
            black_box(consumer.finish().unwrap());
        }
    }
}

/// Half-gates evaluation scenario over many circuits
/// (`evaluate_garbled_circuits`).
pub struct EvaluateParallelBench {
    circuit: Arc<Circuit>,
    eval_inputs: Vec<Mac>,
    garbled_circuits: Vec<GarbledCircuit>,
}

impl EvaluateParallelBench {
    pub fn new(circuit_count: usize) -> Self {
        let mut rng = StdRng::seed_from_u64(0);
        let delta = Delta::random(&mut rng);
        let (inputs, eval_inputs) = inputs_and_eval_inputs(&delta);

        let circuit = AES128.clone();

        let mut gb = Garbler::default();
        let garbled_circuits: Vec<GarbledCircuit> = (0..circuit_count)
            .map(|_| {
                let mut iter = gb.generate(&circuit, delta, &inputs).unwrap();
                let gates: Vec<_> = iter.by_ref().collect();
                let _ = iter.finish().unwrap();
                GarbledCircuit { gates }
            })
            .collect();

        Self {
            circuit,
            eval_inputs,
            garbled_circuits,
        }
    }

    /// Builds the per-circuit inputs (setup, ideally untimed).
    pub fn circs(&self) -> Vec<GarbledInput> {
        self.garbled_circuits
            .iter()
            .map(|gc| (self.circuit.clone(), self.eval_inputs.clone(), gc.clone()))
            .collect()
    }
}

/// Evaluates the prepared circuits.
pub fn evaluate(circs: Vec<GarbledInput>) {
    black_box(evaluate_garbled_circuits(circs).unwrap());
}
