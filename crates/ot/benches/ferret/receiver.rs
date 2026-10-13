//! Isolated Ferret receiver benchmark.
//!
//! The ST/MT protocol bodies live in `support/receiver_body.rs`.
//!
//! Run with: cargo bench -p mpz-ot --bench ferret_receiver

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use futures::executor::block_on;
use std::collections::HashSet;

#[path = "support/receiver_body.rs"]
mod body;

use body::OT_COUNT;

fn criterion_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("ferret_receiver_st");
    group.sample_size(10);
    group.measurement_time(std::time::Duration::from_secs(10));
    group.throughput(Throughput::Elements(OT_COUNT as u64));

    println!("Recording for Ferret receiver...");
    let recorded = block_on(body::record_for_receiver_st(0, OT_COUNT));
    println!("Recorded {} bytes", recorded.bytes.len());

    // Verify determinism.
    let recorded_2 = block_on(body::record_for_receiver_st(0, OT_COUNT));
    assert_eq!(
        recorded.bytes, recorded_2.bytes,
        "Ferret receiver recordings not deterministic"
    );

    group.bench_function("ferret_receiver_st", |b| {
        b.iter_batched(
            || body::replay_receiver_st(recorded.bytes.clone()),
            |mut ctx| block_on(body::run_receiver_with_replay_st(&mut ctx, &recorded)),
            criterion::BatchSize::SmallInput,
        );
    });

    group.finish();

    // MT isolated receiver benchmark
    let mut group_mt = c.benchmark_group("ferret_receiver_mt");
    group_mt.sample_size(10);
    group_mt.measurement_time(std::time::Duration::from_secs(10));
    group_mt.throughput(Throughput::Elements(OT_COUNT as u64));

    println!("Recording for MT Ferret receiver...");
    let recorded_mt = block_on(body::record_for_receiver_mt(0, OT_COUNT));
    let total_bytes: usize = recorded_mt.data.channels.values().map(|v| v.len()).sum();
    println!(
        "Recorded {} channels, {} total bytes",
        recorded_mt.data.channels.len(),
        total_bytes
    );

    // Verify determinism.
    let recorded_mt_2 = block_on(body::record_for_receiver_mt(0, OT_COUNT));
    assert_eq!(
        recorded_mt.data.channels.keys().collect::<HashSet<_>>(),
        recorded_mt_2.data.channels.keys().collect::<HashSet<_>>(),
        "MT Ferret receiver recordings have different channels"
    );
    for (id, data) in &recorded_mt.data.channels {
        assert_eq!(
            data,
            recorded_mt_2.data.channels.get(id).unwrap(),
            "MT Ferret receiver recordings not deterministic for channel {:?}",
            id
        );
    }

    group_mt.bench_function("ferret_receiver_mt", |b| {
        b.iter_batched(
            || body::replay_receiver_mt(recorded_mt.data.clone()),
            |mut exec| block_on(body::run_receiver_with_replay_mt(&mut exec, &recorded_mt)),
            criterion::BatchSize::SmallInput,
        );
    });

    group_mt.finish();
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
