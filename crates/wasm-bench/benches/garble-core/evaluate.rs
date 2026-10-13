//! Browser port of `crates/garble-core/benches/evaluate.rs`.
//!
//! Scenarios shared verbatim via `#[path]` from
//! `crates/garble-core/benches/support/evaluate_body.rs`.

use core::time::Duration;

use mpz_circuits::AES128;
use wasm_bindgen_test::{Criterion, Instant, wasm_bindgen_bench, wasm_bindgen_test_configure};

#[path = "../../../garble-core/benches/support/evaluate_body.rs"]
mod body;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_bench]
fn evaluate(_c: &mut Criterion) {
    // Native benches use sample_size(10); the macro's criterion can't be reconfigured.
    let mut c = Criterion::default()
        .with_location(file!(), module_path!())
        .sample_size(10);

    let gates_per_circuit = AES128.and_count() as u64;

    for &(threshold, name) in body::THRESHOLDS {
        let iterations = threshold.div_ceil(gates_per_circuit) as usize;

        let mut iter_bench = body::EvaluateBench::new(iterations);
        c.bench_function(&format!("evaluate/iter/{name}"), |b| {
            b.iter(|| iter_bench.run_iter());
        });

        let mut batched_bench = body::EvaluateBench::new(iterations);
        c.bench_function(&format!("evaluate/batched/{name}"), |b| {
            b.iter(|| batched_bench.run_batched());
        });
    }
}

#[wasm_bindgen_bench]
fn evaluate_parallel(_c: &mut Criterion) {
    // Native benches use sample_size(10); the macro's criterion can't be reconfigured.
    let mut c = Criterion::default()
        .with_location(file!(), module_path!())
        .sample_size(10);

    let gates_per_circuit = AES128.and_count() as u64;

    for &(threshold, name) in body::THRESHOLDS {
        let circuit_count = threshold.div_ceil(gates_per_circuit) as usize;
        let bench = body::EvaluateParallelBench::new(circuit_count);

        c.bench_function(&format!("evaluate_parallel/rayon/{name}"), |b| {
            // Setup (building per-circuit inputs) is untimed.
            b.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let circs = bench.circs();
                    let start = Instant::now();
                    body::evaluate(circs);
                    total += start.elapsed();
                }
                total
            });
        });
    }
}
