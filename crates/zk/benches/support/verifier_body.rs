//! Shared, target-agnostic body for the isolated ZK verifier benchmarks.
//!
//! This file lives in the native bench directory but is included verbatim via
//! the `#[path]` attribute by both:
//!
//!   * `crates/zk/benches/verifier.rs` — the native criterion harness
//!   * `crates/wasm-bench/benches/zk/verifier.rs` — the browser harness
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
use mpz_core::Block;
use mpz_ot::ideal::rcot::ideal_rcot;
use mpz_vm_core::{
    Call,
    memory::{Array, binary::U8, correlated::Delta},
    prelude::*,
};
use mpz_zk::{Prover, ProverConfig, Verifier, VerifierConfig};
use rand::{RngExt, SeedableRng, rngs::StdRng};

/// Gate-count thresholds shared by the native criterion bench and the browser
/// harness (`verifier/{st,mt}/{100K,1M,10M}`).
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
/// Records prover->verifier messages.
pub async fn run_protocol_record_prover_st(
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

/// Records prover->verifier messages for ST verifier replay. Also returns the OT
/// seed and delta so the verifier's OT stream can be rebuilt on replay.
pub async fn record_for_verifier_st(
    circuit_count: usize,
    seed: u64,
) -> (Vec<u8>, Block, Delta) {
    // exec_v's writes are recorded, so prover uses exec_v.
    let (mut ctx_v, mut ctx_p, recorded) =
        recording_st_context_with_limit(1024 * 1024, max_frame_length(&AES128, circuit_count));

    let mut rng = StdRng::seed_from_u64(seed);
    let delta = Delta::random(&mut rng);
    let ot_seed: Block = rng.random();

    run_protocol_record_prover_st(&mut ctx_p, &mut ctx_v, circuit_count, seed).await;

    (recorded.lock().unwrap().clone(), ot_seed, delta)
}

/// Builds an ST replay context for the verifier.
pub fn replay_verifier_st(recorded: Vec<u8>, circuit_count: usize) -> Context {
    replay_st_context(recorded, max_frame_length(&AES128, circuit_count))
}

/// Runs ST verifier only with replay context.
pub async fn run_verifier_with_replay_st(
    ctx: &mut Context,
    circuit_count: usize,
    delta: Delta,
    ot_seed: Block,
) {
    let (ot_send, _) = ideal_rcot(ot_seed, delta.into_inner());
    let verifier_config = VerifierConfig::builder().build().unwrap();
    let mut verifier = Verifier::new(verifier_config, delta, ot_send);

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

    verifier.flush(ctx).await.unwrap();
    verifier.execute(ctx).await.unwrap();
    verifier.flush(ctx).await.unwrap();
}

// ============================================================================
// Multi-threaded (MT)
// ============================================================================

/// Runs the full ZK protocol with MT contexts.
/// Records prover->verifier messages.
pub async fn run_protocol_record_prover_mt(
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

/// Records prover->verifier messages for MT verifier replay. Also returns the OT
/// seed and delta so the verifier's OT stream can be rebuilt on replay.
///
/// Runs on the process-wide `ThreadPool`, so callers must install a global pool
/// (with a `web_spawn` spawn callback on wasm) before calling this.
pub async fn record_for_verifier_mt(
    circuit_count: usize,
    seed: u64,
) -> (RecordedMtData, Block, Delta) {
    // exec_v's writes are recorded, so prover uses exec_v.
    let (mut exec_v, mut exec_p, recorded) = recording_mt_context_with_limit(
        1024 * 1024,
        max_frame_length(&AES128, circuit_count),
    );

    let mut rng = StdRng::seed_from_u64(seed);
    let delta = Delta::random(&mut rng);
    let ot_seed: Block = rng.random();

    run_protocol_record_prover_mt(&mut exec_p, &mut exec_v, circuit_count, seed).await;

    (recorded.lock().unwrap().clone(), ot_seed, delta)
}

/// Builds an MT replay session for the verifier on the process-wide thread pool.
pub fn replay_verifier_mt(recorded: RecordedMtData, circuit_count: usize) -> Session {
    replay_mt_context_with_limit(recorded, max_frame_length(&AES128, circuit_count))
}

/// Runs MT verifier only with replay context.
pub async fn run_verifier_with_replay_mt(
    exec: &mut Session,
    circuit_count: usize,
    delta: Delta,
    ot_seed: Block,
) {
    let (ot_send, _) = ideal_rcot(ot_seed, delta.into_inner());
    let verifier_config = VerifierConfig::builder().build().unwrap();
    let mut verifier = Verifier::new(verifier_config, delta, ot_send);

    let mut ctx = exec.new_context().unwrap();

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

    verifier.flush(&mut ctx).await.unwrap();
    verifier.execute(&mut ctx).await.unwrap();
    verifier.flush(&mut ctx).await.unwrap();
}
