//! Shared, target-agnostic body for the isolated ZK prover benchmarks.
//!
//! This file lives in the native bench directory but is included verbatim via
//! the `#[path]` attribute by both:
//!
//!   * `crates/zk/benches/prover.rs` — the native criterion harness
//!   * `crates/wasm-bench/benches/zk/prover.rs` — the browser harness
//!
//! Constraints: this file must not reference `crate::`, `super::`, criterion,
//! or wasm types. ST (single-threaded `Context`) and MT (pooled `Session`)
//! variants are suffixed `_st`/`_mt`.

use mpz_circuits::AES128;
use mpz_common::context::{
    RecordedMtData, recording_mt_context_with_limit, recording_st_context_with_limit,
    replay_mt_context_with_limit, replay_st_context,
};
use mpz_common::{Context, Session};
use mpz_ot::ideal::rcot::ideal_rcot;
use mpz_vm_core::{
    Call,
    memory::{Array, binary::U8, correlated::Delta},
    prelude::*,
};
use mpz_zk::{Prover, ProverConfig, Verifier, VerifierConfig};
use rand::{RngExt, SeedableRng, rngs::StdRng};

/// Gate-count thresholds shared by the native criterion bench and the browser
/// harness (`prover/{st,mt}/{100K,1M,10M}`).
pub const THRESHOLDS: &[(u64, &str)] =
    &[(100_000, "100K"), (1_000_000, "1M"), (10_000_000, "10M")];

/// Calculate max frame length based on workload size.
pub fn max_frame_length(circuit: &mpz_circuits::Circuit, circuit_count: usize) -> usize {
    let bytes_per_correlation = 1 + 16; // choice bit + MAC
    let overhead = 1.2;
    let correlations = circuit.and_count() * circuit_count;
    ((correlations * bytes_per_correlation) as f64 * overhead) as usize
}

// ============================================================================
// Single-threaded (ST)
// ============================================================================

/// Runs the full ZK protocol with single-threaded contexts.
/// Records verifier->prover messages.
pub async fn run_protocol_record_verifier_st(
    ctx_p: &mut Context,
    ctx_v: &mut Context,
    circuit_count: usize,
    seed: u64,
) {
    let mut rng = StdRng::seed_from_u64(seed);
    let delta = Delta::random(&mut rng);

    let (ot_send, ot_recv) = ideal_rcot(rng.random(), delta.into_inner());

    let prover_config = ProverConfig::builder().build().unwrap();
    let verifier_config = VerifierConfig::builder().build().unwrap();

    let mut prover = Prover::new(prover_config, ot_recv);
    let mut verifier = Verifier::new(verifier_config, delta, ot_send);

    futures::join!(
        async {
            let key: Array<U8, 16> = prover.alloc().unwrap();
            prover.mark_private(key).unwrap();
            prover.assign(key, [0u8; 16]).unwrap();
            prover.commit(key).unwrap();

            for _ in 0..circuit_count {
                let msg: Array<U8, 16> = prover.alloc().unwrap();
                prover.mark_public(msg).unwrap();
                prover.assign(msg, [42u8; 16]).unwrap();
                prover.commit(msg).unwrap();

                let ciphertext: Array<U8, 16> = prover
                    .call(
                        Call::builder(AES128.clone())
                            .arg(key)
                            .arg(msg)
                            .build()
                            .unwrap(),
                    )
                    .unwrap();

                std::mem::drop(prover.decode(ciphertext).unwrap());
            }

            prover.flush(ctx_p).await.unwrap();
            prover.execute(ctx_p).await.unwrap();
            prover.flush(ctx_p).await.unwrap();
        },
        async {
            let key: Array<U8, 16> = verifier.alloc().unwrap();
            verifier.mark_blind(key).unwrap();
            verifier.commit(key).unwrap();

            for _ in 0..circuit_count {
                let msg: Array<U8, 16> = verifier.alloc().unwrap();
                verifier.mark_public(msg).unwrap();
                verifier.assign(msg, [42u8; 16]).unwrap();
                verifier.commit(msg).unwrap();

                let ciphertext: Array<U8, 16> = verifier
                    .call(
                        Call::builder(AES128.clone())
                            .arg(key)
                            .arg(msg)
                            .build()
                            .unwrap(),
                    )
                    .unwrap();

                std::mem::drop(verifier.decode(ciphertext).unwrap());
            }

            verifier.flush(ctx_v).await.unwrap();
            verifier.execute(ctx_v).await.unwrap();
            verifier.flush(ctx_v).await.unwrap();
        }
    );
}

/// Records verifier->prover messages for ST prover replay.
pub async fn record_for_prover_st(circuit_count: usize, seed: u64) -> Vec<u8> {
    let (mut ctx_p, mut ctx_v, recorded) =
        recording_st_context_with_limit(1024 * 1024, max_frame_length(&AES128, circuit_count));
    run_protocol_record_verifier_st(&mut ctx_p, &mut ctx_v, circuit_count, seed).await;
    recorded.lock().unwrap().clone()
}

/// Builds an ST replay context for the prover.
pub fn replay_prover_st(recorded: Vec<u8>, circuit_count: usize) -> Context {
    replay_st_context(recorded, max_frame_length(&AES128, circuit_count))
}

/// Runs ST prover only with replay context.
pub async fn run_prover_with_replay_st(ctx: &mut Context, circuit_count: usize) {
    let (_, ot_recv) = ideal_rcot([0u8; 16].into(), [0u8; 16].into());
    let prover_config = ProverConfig::builder().build().unwrap();
    let mut prover = Prover::new(prover_config, ot_recv);

    let key: Array<U8, 16> = prover.alloc().unwrap();
    prover.mark_private(key).unwrap();
    prover.assign(key, [0u8; 16]).unwrap();
    prover.commit(key).unwrap();

    for _ in 0..circuit_count {
        let msg: Array<U8, 16> = prover.alloc().unwrap();
        prover.mark_public(msg).unwrap();
        prover.assign(msg, [42u8; 16]).unwrap();
        prover.commit(msg).unwrap();

        let ciphertext: Array<U8, 16> = prover
            .call(
                Call::builder(AES128.clone())
                    .arg(key)
                    .arg(msg)
                    .build()
                    .unwrap(),
            )
            .unwrap();

        std::mem::drop(prover.decode(ciphertext).unwrap());
    }

    prover.flush(ctx).await.unwrap();
    prover.execute(ctx).await.unwrap();
    prover.flush(ctx).await.unwrap();
}

// ============================================================================
// Multi-threaded (MT)
// ============================================================================

/// Runs the full ZK protocol with MT contexts.
/// Records verifier->prover messages.
pub async fn run_protocol_record_verifier_mt(
    exec_p: &mut Session,
    exec_v: &mut Session,
    circuit_count: usize,
    seed: u64,
) {
    let mut rng = StdRng::seed_from_u64(seed);
    let delta = Delta::random(&mut rng);

    let (ot_send, ot_recv) = ideal_rcot(rng.random(), delta.into_inner());

    let prover_config = ProverConfig::builder().build().unwrap();
    let verifier_config = VerifierConfig::builder().build().unwrap();

    let mut prover = Prover::new(prover_config, ot_recv);
    let mut verifier = Verifier::new(verifier_config, delta, ot_send);

    let mut ctx_p = exec_p.new_context().unwrap();
    let mut ctx_v = exec_v.new_context().unwrap();

    futures::join!(
        async {
            let key: Array<U8, 16> = prover.alloc().unwrap();
            prover.mark_private(key).unwrap();
            prover.assign(key, [0u8; 16]).unwrap();
            prover.commit(key).unwrap();

            for _ in 0..circuit_count {
                let msg: Array<U8, 16> = prover.alloc().unwrap();
                prover.mark_public(msg).unwrap();
                prover.assign(msg, [42u8; 16]).unwrap();
                prover.commit(msg).unwrap();

                let ciphertext: Array<U8, 16> = prover
                    .call(
                        Call::builder(AES128.clone())
                            .arg(key)
                            .arg(msg)
                            .build()
                            .unwrap(),
                    )
                    .unwrap();

                std::mem::drop(prover.decode(ciphertext).unwrap());
            }

            prover.flush(&mut ctx_p).await.unwrap();
            prover.execute(&mut ctx_p).await.unwrap();
            prover.flush(&mut ctx_p).await.unwrap();
        },
        async {
            let key: Array<U8, 16> = verifier.alloc().unwrap();
            verifier.mark_blind(key).unwrap();
            verifier.commit(key).unwrap();

            for _ in 0..circuit_count {
                let msg: Array<U8, 16> = verifier.alloc().unwrap();
                verifier.mark_public(msg).unwrap();
                verifier.assign(msg, [42u8; 16]).unwrap();
                verifier.commit(msg).unwrap();

                let ciphertext: Array<U8, 16> = verifier
                    .call(
                        Call::builder(AES128.clone())
                            .arg(key)
                            .arg(msg)
                            .build()
                            .unwrap(),
                    )
                    .unwrap();

                std::mem::drop(verifier.decode(ciphertext).unwrap());
            }

            verifier.flush(&mut ctx_v).await.unwrap();
            verifier.execute(&mut ctx_v).await.unwrap();
            verifier.flush(&mut ctx_v).await.unwrap();
        }
    );
}

/// Records verifier->prover messages for MT prover replay.
///
/// Runs on the process-wide `ThreadPool`, so callers must install a global pool
/// (with a `web_spawn` spawn callback on wasm) before calling this.
pub async fn record_for_prover_mt(circuit_count: usize, seed: u64) -> RecordedMtData {
    let (mut exec_p, mut exec_v, recorded) = recording_mt_context_with_limit(
        1024 * 1024,
        max_frame_length(&AES128, circuit_count),
    );
    run_protocol_record_verifier_mt(&mut exec_p, &mut exec_v, circuit_count, seed).await;
    recorded.lock().unwrap().clone()
}

/// Builds an MT replay session for the prover on the process-wide thread pool.
pub fn replay_prover_mt(recorded: RecordedMtData, circuit_count: usize) -> Session {
    replay_mt_context_with_limit(recorded, max_frame_length(&AES128, circuit_count))
}

/// Runs MT prover only with replay context.
pub async fn run_prover_with_replay_mt(exec: &mut Session, circuit_count: usize) {
    let (_, ot_recv) = ideal_rcot([0u8; 16].into(), [0u8; 16].into());
    let prover_config = ProverConfig::builder().build().unwrap();
    let mut prover = Prover::new(prover_config, ot_recv);

    let mut ctx = exec.new_context().unwrap();

    let key: Array<U8, 16> = prover.alloc().unwrap();
    prover.mark_private(key).unwrap();
    prover.assign(key, [0u8; 16]).unwrap();
    prover.commit(key).unwrap();

    for _ in 0..circuit_count {
        let msg: Array<U8, 16> = prover.alloc().unwrap();
        prover.mark_public(msg).unwrap();
        prover.assign(msg, [42u8; 16]).unwrap();
        prover.commit(msg).unwrap();

        let ciphertext: Array<U8, 16> = prover
            .call(
                Call::builder(AES128.clone())
                    .arg(key)
                    .arg(msg)
                    .build()
                    .unwrap(),
            )
            .unwrap();

        std::mem::drop(prover.decode(ciphertext).unwrap());
    }

    prover.flush(&mut ctx).await.unwrap();
    prover.execute(&mut ctx).await.unwrap();
    prover.flush(&mut ctx).await.unwrap();
}
