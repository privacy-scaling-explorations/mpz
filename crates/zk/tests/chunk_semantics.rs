//! Deterministic chunking semantics: exact chunk/batch/residual boundaries and
//! readiness-driven (non-contiguous) chunking.

mod common;

use common::{
    CHECK_OTS, CountedReceiver, CountedSender, Counts, and_chain, calibrate, reference_checks,
    run_case,
};
use mpz_common::context::test_st_context;
use mpz_ot::ideal::rcot::ideal_rcot;
use mpz_vm_core::{
    Call,
    memory::{Array, binary::U8, correlated::Delta},
    prelude::*,
};
use mpz_zk::{Prover, ProverConfig, Verifier, VerifierConfig};
use rand::{RngExt, SeedableRng, rngs::StdRng};

/// Asserts a fully-ready case against the independent model.
async fn assert_case(sizes: &[usize], batch: usize, chunk_cap: usize, base: (usize, usize)) {
    let counts = run_case(sizes, batch, chunk_cap, 0).await;
    assert_eq!(
        counts.prover, counts.verifier,
        "asymmetry: sizes={sizes:?} batch={batch} chunk_cap={chunk_cap}",
    );

    let gates: usize = sizes.iter().sum();
    let checks = reference_checks(sizes, batch, chunk_cap);

    assert_eq!(
        counts.prover.1,
        base.1 + gates + CHECK_OTS * checks,
        "consumed: sizes={sizes:?} batch={batch} chunk_cap={chunk_cap} checks={checks}",
    );
    assert_eq!(
        counts.prover.0,
        base.0 + gates + CHECK_OTS * gates.div_ceil(batch).max(checks),
        "allocated: sizes={sizes:?} batch={batch} chunk_cap={chunk_cap} checks={checks}",
    );
}

/// Exact chunk/batch/residual boundaries, pinned deterministically.
#[tokio::test]
async fn chunk_boundaries() {
    let base = calibrate().await;

    // Empty callstack.
    assert_case(&[], 4, 4, base).await;
    // Zero-gate calls only.
    assert_case(&[0, 0, 0], 4, 4, base).await;
    // A call exactly `batch_size`.
    assert_case(&[5], 5, 5, base).await;
    // Chunk ends exactly on a batch boundary: no residual check.
    assert_case(&[5, 5], 5, 10, base).await;
    // Residual of exactly one gate: an extra check.
    assert_case(&[5, 1], 5, 10, base).await;
    // Tightest config: chunk_cap == batch_size.
    assert_case(&[2, 2, 2, 2], 4, 4, base).await;
    // A single call larger than the chunk window: its own chunk, never split.
    assert_case(&[10], 4, 4, base).await;
    // A single call larger than a batch but within the chunk window.
    assert_case(&[10], 4, 20, base).await;
    // batch_size == 1.
    assert_case(&[3, 3], 1, 8, base).await;
    // Exact chunk multiples: one check per chunk.
    assert_case(&[4, 4, 4], 2, 4, base).await;
}

/// Model of checks across staggered commit rounds.
fn reference_checks_rounds(
    sizes: &[usize],
    commit_rounds: &[usize],
    batch: usize,
    chunk_cap: usize,
) -> usize {
    let mut executed = vec![false; sizes.len()];
    let max_round = commit_rounds.iter().copied().max().unwrap_or(0);
    let mut checks = 0;

    for round in 0..=max_round {
        let ready: Vec<usize> = (0..sizes.len())
            .filter(|&i| !executed[i] && commit_rounds[i] <= round)
            .collect();

        let mut idx = 0;
        while idx < ready.len() {
            let start = idx;
            let mut gates = 0;
            while idx < ready.len() {
                gates += sizes[ready[idx]];
                idx += 1;
                if gates >= chunk_cap {
                    break;
                }
            }

            let mut pending = 0;
            for &i in &ready[start..idx] {
                pending += sizes[i];
                if pending >= batch {
                    checks += 1;
                    pending = 0;
                }
            }
            if pending > 0 {
                checks += 1;
            }

            for &i in &ready[start..idx] {
                executed[i] = true;
            }
        }
    }

    checks
}

/// Runs a prover/verifier pair, committing each call's input in the round given
/// by `commit_rounds` and executing everything ready after each round.
async fn run_rounds(
    sizes: &[usize],
    commit_rounds: &[usize],
    batch: usize,
    chunk_cap: usize,
    seed: u64,
) -> Counts {
    let mut rng = StdRng::seed_from_u64(seed);
    let delta = Delta::random(&mut rng);
    let (ot_send, ot_recv) = ideal_rcot(rng.random(), delta.into_inner());
    let ot_recv = CountedReceiver::new(ot_recv);
    let ot_send = CountedSender::new(ot_send);

    let prover_counters = ot_recv.counters();
    let verifier_counters = ot_send.counters();

    let mut prover = Prover::new(
        ProverConfig::builder()
            .batch_size(batch)
            .chunk_cap(chunk_cap)
            .build()
            .unwrap(),
        ot_recv,
    );
    let mut verifier = Verifier::new(
        VerifierConfig::builder()
            .batch_size(batch)
            .chunk_cap(chunk_cap)
            .build()
            .unwrap(),
        delta,
        ot_send,
    );

    let (mut ctx_p, mut ctx_v) = test_st_context(1 << 20);

    let p_key: Array<U8, 16> = prover.alloc().unwrap();
    let v_key: Array<U8, 16> = verifier.alloc().unwrap();
    prover.mark_private(p_key).unwrap();
    verifier.mark_blind(v_key).unwrap();
    prover.assign(p_key, [7u8; 16]).unwrap();
    prover.commit(p_key).unwrap();
    verifier.commit(v_key).unwrap();

    let mut p_msgs = Vec::with_capacity(sizes.len());
    let mut v_msgs = Vec::with_capacity(sizes.len());
    let mut p_outputs = Vec::with_capacity(sizes.len());
    let mut v_outputs = Vec::with_capacity(sizes.len());

    for &and_count in sizes {
        let circuit = and_chain(and_count);

        let p_msg: Array<U8, 16> = prover.alloc().unwrap();
        let v_msg: Array<U8, 16> = verifier.alloc().unwrap();
        prover.mark_public(p_msg).unwrap();
        verifier.mark_public(v_msg).unwrap();
        prover.assign(p_msg, [42u8; 16]).unwrap();
        verifier.assign(v_msg, [42u8; 16]).unwrap();

        let p_out: Array<U8, 16> = prover
            .call(
                Call::builder(circuit.clone())
                    .arg(p_key)
                    .arg(p_msg)
                    .build()
                    .unwrap(),
            )
            .unwrap();
        let v_out: Array<U8, 16> = verifier
            .call(
                Call::builder(circuit)
                    .arg(v_key)
                    .arg(v_msg)
                    .build()
                    .unwrap(),
            )
            .unwrap();

        p_msgs.push(p_msg);
        v_msgs.push(v_msg);
        p_outputs.push(prover.decode(p_out).unwrap());
        v_outputs.push(verifier.decode(v_out).unwrap());
    }

    let max_round = commit_rounds.iter().copied().max().unwrap_or(0);
    for round in 0..=max_round {
        for i in 0..sizes.len() {
            if commit_rounds[i] == round {
                prover.commit(p_msgs[i]).unwrap();
                verifier.commit(v_msgs[i]).unwrap();
            }
        }

        let (prover_result, verifier_result) = futures::join!(
            prover.execute_all(&mut ctx_p),
            verifier.execute_all(&mut ctx_v),
        );
        prover_result.unwrap();
        verifier_result.unwrap();
    }

    for mut output in p_outputs {
        output.try_recv().unwrap().unwrap();
    }
    for mut output in v_outputs {
        output.try_recv().unwrap().unwrap();
    }

    Counts {
        prover: (prover_counters.allocated(), prover_counters.consumed()),
        verifier: (verifier_counters.allocated(), verifier_counters.consumed()),
    }
}

/// Readiness-driven chunking: a call committed in a later round forces a
/// non-contiguous chunk (ready calls in stack order, skipping the unready one).
#[tokio::test]
async fn chunk_readiness() {
    let base = calibrate().await;

    let cases: &[(&[usize], &[usize])] = &[
        (&[3, 3, 3], &[0, 1, 0]),
        (&[3, 3, 3], &[0, 0, 1]),
        (&[5, 5, 5, 5], &[0, 1, 1, 0]),
    ];

    for &(sizes, commit_rounds) in cases {
        let batch = 2;
        let chunk_cap = 8;

        let counts = run_rounds(sizes, commit_rounds, batch, chunk_cap, 0).await;
        assert_eq!(
            counts.prover, counts.verifier,
            "readiness asymmetry: sizes={sizes:?} rounds={commit_rounds:?}",
        );

        let gates: usize = sizes.iter().sum();
        let checks = reference_checks_rounds(sizes, commit_rounds, batch, chunk_cap);

        assert_eq!(
            counts.prover.1,
            base.1 + gates + CHECK_OTS * checks,
            "readiness consumed: sizes={sizes:?} rounds={commit_rounds:?} checks={checks}",
        );
        assert_eq!(
            counts.prover.0,
            base.0 + gates + CHECK_OTS * gates.div_ceil(batch).max(checks),
            "readiness allocated: sizes={sizes:?} rounds={commit_rounds:?} checks={checks}",
        );
    }
}
