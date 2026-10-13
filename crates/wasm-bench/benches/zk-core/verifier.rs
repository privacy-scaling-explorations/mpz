//! Browser port of `crates/zk-core/benches/verifier.rs`.
//!
//! The benchmark body is shared verbatim via `#[path]` from
//! `crates/zk-core/benches/support/verifier_body.rs`.
//!
//! Run with:
//!   CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner \
//!     WASM_BINDGEN_TEST_TIMEOUT=1800 WASM_BINDGEN_TEST_DRIVER_TIMEOUT=1800 \
//!     cargo +nightly bench --no-default-features --target wasm32-unknown-unknown \
//!       --bench zk_core_verifier -Zbuild-std=panic_abort,std

use core::time::Duration;
use std::hint::black_box;
use std::sync::Arc;

use mpz_circuits::AES128;
use wasm_bindgen_test::{Criterion, Instant, wasm_bindgen_bench, wasm_bindgen_test_configure};

#[path = "../../../zk-core/benches/support/verifier_body.rs"]
mod body;
#[path = "../support/pools.rs"]
mod pools;

use body::VerifierBench;

wasm_bindgen_test_configure!(run_in_browser);

// Thresholds come from the shared body.

#[wasm_bindgen_bench]
fn verifier_execute(_c: &mut Criterion) {
    // Native benches use sample_size(10); the macro's criterion can't be reconfigured.
    let mut c = Criterion::default()
        .with_location(file!(), module_path!())
        .sample_size(10);

    let and_count = AES128.and_count();
    for &(gate_count, name) in body::EXECUTE_THRESHOLDS {
        let circuit_count = (gate_count as usize).div_ceil(and_count);
        let bench = VerifierBench::new(AES128.clone(), circuit_count);
        c.bench_function(&format!("verifier/execute/{name}"), |b| {
            b.iter(|| {
                bench.execute();
                black_box(());
            });
        });
    }
}

#[wasm_bindgen_bench]
async fn verifier_check(_c: &mut Criterion) {
    // Native benches use sample_size(10); the macro's criterion can't be reconfigured.
    let mut c = Criterion::default()
        .with_location(file!(), module_path!())
        .sample_size(10);

    let and_count = AES128.and_count();
    for &(gate_count, name) in body::CHECK_THRESHOLDS {
        let circuit_count = (gate_count as usize).div_ceil(and_count);
        let bench = Arc::new(VerifierBench::new(AES128.clone(), circuit_count));
        c.bench_async_function(&format!("verifier/check/{name}"), move |b| {
            let bench = bench.clone();
            Box::pin(b.iter_custom_future(move |iters| {
                let bench = bench.clone();
                async move {
                    // `check` is rayon-parallel and parks its caller, which is
                    // illegal on the main thread, so run each sample's
                    // iterations on a worker. Setup (execute + prover check) is
                    // untimed; only `check_run` is measured. The vendored
                    // criterion has no `iter_batched`, so we self-time.
                    pools::run_in_rayon_worker(move || {
                        let mut total = Duration::ZERO;
                        for _ in 0..iters {
                            let (mut verifier, mut transcript, uv) = bench.check_setup();
                            let start = Instant::now();
                            bench.check_run(&mut verifier, &mut transcript, uv);
                            total += start.elapsed();
                        }
                        total
                    })
                    .await
                }
            }))
        })
        .await;
    }
}
