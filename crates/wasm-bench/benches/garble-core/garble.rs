//! Browser port of `crates/garble-core/benches/garble.rs`.
//!
//! Scenario shared verbatim via `#[path]` from
//! `crates/garble-core/benches/support/garble_body.rs`.

use mpz_circuits::AES128;
use wasm_bindgen_test::{Criterion, wasm_bindgen_bench, wasm_bindgen_test_configure};

#[path = "../../../garble-core/benches/support/garble_body.rs"]
mod body;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_bench]
fn garble(_c: &mut Criterion) {
    // Native benches use sample_size(10); the macro's criterion can't be
    // reconfigured.
    let mut c = Criterion::default()
        .with_location(file!(), module_path!())
        .sample_size(10);

    let gates_per_circuit = AES128.and_count() as u64;

    for &(threshold, name) in body::THRESHOLDS {
        let iterations = threshold.div_ceil(gates_per_circuit) as usize;

        let mut iter_bench = body::GarbleBench::new(iterations);
        c.bench_function(&format!("garble/iter/{name}"), |b| {
            b.iter(|| iter_bench.run_iter());
        });

        let mut batched_bench = body::GarbleBench::new(iterations);
        c.bench_function(&format!("garble/batched/{name}"), |b| {
            b.iter(|| batched_bench.run_batched());
        });
    }
}
