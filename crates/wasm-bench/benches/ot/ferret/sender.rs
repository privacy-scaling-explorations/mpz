//! Browser port of `crates/ot/benches/ferret/sender.rs` (ST + MT).
//!
//! The ST/MT bodies are shared verbatim via `#[path]` from
//! `crates/ot/benches/ferret/support/sender_body.rs`.
//!
//! Run with:
//!   CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner \
//!     WASM_BINDGEN_TEST_TIMEOUT=1800 WASM_BINDGEN_TEST_DRIVER_TIMEOUT=1800 \
//!     cargo +nightly bench --no-default-features --target wasm32-unknown-unknown \
//!       --bench ot_ferret_sender -Zbuild-std=panic_abort,std

use core::time::Duration;
use std::sync::Arc;

use wasm_bindgen_test::{Criterion, Instant, wasm_bindgen_bench, wasm_bindgen_test_configure};

#[path = "../../../../ot/benches/ferret/support/sender_body.rs"]
mod body;
#[path = "../../support/pools.rs"]
mod pools;

wasm_bindgen_test_configure!(run_in_browser);

// The replay context (and its recorded-data copy) is built in the untimed part
// of `iter_custom_future`, so only `run_*_with_replay` is measured.

/// Single-threaded Ferret sender.
#[wasm_bindgen_bench]
async fn ferret_sender_st(_c: &mut Criterion) {
    // Native benches use sample_size(10); the macro's criterion can't be reconfigured.
    let mut c = Criterion::default()
        .with_location(file!(), module_path!())
        .sample_size(10);

    let recorded = Arc::new(body::record_for_sender_st(0, body::OT_COUNT).await);

    c.bench_async_function("ferret_sender_st", move |b| {
        let recorded = recorded.clone();
        Box::pin(b.iter_custom_future(move |iters| {
            let recorded = recorded.clone();
            async move {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    // Setup (untimed).
                    let mut ctx = body::replay_sender_st(recorded.bytes.clone());
                    // Timed.
                    let start = Instant::now();
                    body::run_sender_with_replay_st(&mut ctx, &recorded).await;
                    total += start.elapsed();
                }
                total
            }
        }))
    })
    .await;
}

/// Multi-threaded Ferret sender.
#[wasm_bindgen_bench]
async fn ferret_sender_mt(_c: &mut Criterion) {
    // Native benches use sample_size(10); the macro's criterion can't be reconfigured.
    let mut c = Criterion::default()
        .with_location(file!(), module_path!())
        .sample_size(10);

    pools::init_mpz().await;

    let recorded = Arc::new(body::record_for_sender_mt(0, body::OT_COUNT).await);

    c.bench_async_function("ferret_sender_mt", move |b| {
        let recorded = recorded.clone();
        Box::pin(b.iter_custom_future(move |iters| {
            let recorded = recorded.clone();
            async move {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    // Setup (untimed).
                    let mut exec = body::replay_sender_mt(recorded.data.clone());
                    // Timed.
                    let start = Instant::now();
                    body::run_sender_with_replay_mt(&mut exec, &recorded).await;
                    total += start.elapsed();
                }
                total
            }
        }))
    })
    .await;
}
