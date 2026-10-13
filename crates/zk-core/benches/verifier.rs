//! Benchmarks for QuickSilver ZK verifier.
//!
//! Body shared verbatim with `crates/wasm-bench/benches/zk-core/verifier.rs` via
//! the `#[path]`-included `support/verifier_body.rs`.
//!
//! Run with: cargo bench -p mpz-zk-core --bench verifier

use std::hint::black_box;
use std::sync::Arc;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use mpz_circuits::AES128;

#[path = "support/verifier_body.rs"]
mod body;

use body::{CHECK_THRESHOLDS, EXECUTE_THRESHOLDS, VerifierBench};

/// Benchmarks only the execute phase (no check).
fn bench_verifier_execute(c: &mut Criterion) {
    let mut group = c.benchmark_group("verifier");
    group.sample_size(10);

    let circuit: Arc<mpz_circuits::Circuit> = AES128.clone();
    let gates_per_circuit = circuit.and_count() as u64;

    for &(threshold, name) in EXECUTE_THRESHOLDS {
        let circuit_count = threshold.div_ceil(gates_per_circuit) as usize;
        let actual_gates = circuit_count as u64 * gates_per_circuit;

        group.throughput(Throughput::Elements(actual_gates));

        let bench = VerifierBench::new(circuit.clone(), circuit_count);
        group.bench_function(BenchmarkId::new("execute", name), |b| {
            b.iter(|| {
                bench.execute();
                black_box(())
            })
        });
    }

    group.finish();
}

/// Benchmarks only the check phase (execute in untimed setup).
fn bench_verifier_check(c: &mut Criterion) {
    let mut group = c.benchmark_group("verifier");
    group.sample_size(10);

    let circuit: Arc<mpz_circuits::Circuit> = AES128.clone();
    let gates_per_circuit = circuit.and_count() as u64;

    for &(threshold, name) in CHECK_THRESHOLDS {
        let circuit_count = threshold.div_ceil(gates_per_circuit) as usize;
        let actual_gates = circuit_count as u64 * gates_per_circuit;

        group.throughput(Throughput::Elements(actual_gates));

        let bench = VerifierBench::new(circuit.clone(), circuit_count);
        group.bench_function(BenchmarkId::new("check", name), |b| {
            b.iter_batched(
                || bench.check_setup(),
                |(mut verifier, mut transcript, uv)| {
                    bench.check_run(&mut verifier, &mut transcript, uv);
                },
                criterion::BatchSize::LargeInput,
            )
        });
    }

    group.finish();
}

criterion_group!(benches, bench_verifier_execute, bench_verifier_check);
criterion_main!(benches);
