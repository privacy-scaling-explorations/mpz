//! Benchmarks for half-gates garbling.
//!
//! The scenario lives in `support/garble_body.rs` (shared verbatim with
//! `crates/wasm-bench` via `#[path]`).
//!
//! Run with: `cargo bench -p mpz-garble-core --bench garble`

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use mpz_circuits::AES128;

#[path = "support/garble_body.rs"]
mod body;

use body::{GarbleBench, THRESHOLDS};

fn bench_garble(c: &mut Criterion) {
    let mut group = c.benchmark_group("garble");
    group.sample_size(10);

    let gates_per_circuit = AES128.and_count() as u64;

    for &(threshold, name) in THRESHOLDS {
        let iterations = threshold.div_ceil(gates_per_circuit) as usize;
        let actual_gates = iterations as u64 * gates_per_circuit;

        group.throughput(Throughput::Elements(actual_gates));

        // Iterator-based (one gate at a time)
        let mut iter_bench = GarbleBench::new(iterations);
        group.bench_function(BenchmarkId::new("iter", name), |b| {
            b.iter(|| iter_bench.run_iter());
        });

        // Batched (multiple gates at a time)
        let mut batched_bench = GarbleBench::new(iterations);
        group.bench_function(BenchmarkId::new("batched", name), |b| {
            b.iter(|| batched_bench.run_batched());
        });
    }

    group.finish();
}

criterion_group!(benches, bench_garble);
criterion_main!(benches);
