use criterion::{black_box, criterion_group, criterion_main, Criterion};
use mpz_circuits::circuits::{build_sha256, AES128};
use mpz_garble_core::{ChaChaEncoder, Encoder, Generator};
use std::sync::Arc;

fn criterion_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("garble_circuits");

    let encoder = ChaChaEncoder::new([0u8; 32]);
    let inputs = AES128
        .inputs()
        .iter()
        .map(|value| encoder.encode_by_type(0, &value.value_type()))
        .collect::<Vec<_>>();

    group.bench_function("aes128", |b| {
        b.iter(|| {
            let mut gen = Generator::new(Arc::clone(&AES128).into_gates_iterator(), encoder.delta(), &inputs).unwrap();

            let mut enc_gates = Vec::with_capacity(AES128.and_count());
            for gate in gen.by_ref() {
                enc_gates.push(gate);
            }

            black_box(gen.outputs().unwrap())
        })
    });
    group.bench_function("aes128_with_hash", |b| {
        b.iter(|| {
            let mut gen =
                Generator::new_with_hasher(Arc::clone(&AES128).into_gates_iterator(), encoder.delta(), &inputs).unwrap();

            let mut enc_gates = Vec::with_capacity(AES128.and_count());
            for gate in gen.by_ref() {
                enc_gates.push(gate);
            }

            black_box(gen.outputs().unwrap())
        })
    });

    let length = 512;
    let sha256 = Arc::new(build_sha256(0, length));

    let inputs = sha256
        .inputs()
        .iter()
        .map(|value| encoder.encode_by_type(0, &value.value_type()))
        .collect::<Vec<_>>();

    group.bench_function(format!("sha256_with_length_{}", length), |b| {
        b.iter(|| {
            let mut gen =
                Generator::new_with_hasher(Arc::clone(&sha256).into_gates_iterator(), encoder.delta(), &inputs).unwrap();

            let mut enc_gates = Vec::with_capacity(sha256.and_count());
            for gate in gen.by_ref() {
                enc_gates.push(gate);
            }

            black_box(gen.outputs().unwrap())
        })
    });
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
