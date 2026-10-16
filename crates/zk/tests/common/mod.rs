//! Shared harness for the zk chunked-execution tests.
#![allow(dead_code)]

mod counted;

pub use counted::{CountedReceiver, CountedSender};

use std::sync::Arc;

use mpz_circuits::{Circuit, CircuitBuilder};
use mpz_common::context::test_st_context;
use mpz_ot::ideal::rcot::ideal_rcot;
use mpz_vm_core::{
    Call,
    memory::{Array, binary::U8, correlated::Delta},
    prelude::*,
};
use mpz_zk::{Prover, ProverConfig, Verifier, VerifierConfig};
use rand::{RngExt, SeedableRng, rngs::StdRng};

/// Number of OTs a single consistency check consumes.
pub const CHECK_OTS: usize = 128;

/// Number of bits in the reused private key / public message (`Array<U8, 16>`).
pub const WORD_BITS: usize = 128;

/// Builds a circuit with exactly `and_count` AND gates, taking two `U8` words
/// as input and yielding one.
pub fn and_chain(and_count: usize) -> Arc<Circuit> {
    let mut builder = CircuitBuilder::new();
    let inputs: Vec<_> = (0..2 * WORD_BITS).map(|_| builder.add_input()).collect();

    let mut acc = inputs[0];
    for i in 0..and_count {
        acc = builder.add_and_gate(acc, inputs[1 + (i % (inputs.len() - 1))]);
    }

    // One word of outputs to match `Array<U8, 16>`.
    builder.add_output(acc);
    for node in &inputs[1..WORD_BITS] {
        builder.add_output(*node);
    }

    Arc::new(builder.build().unwrap())
}

/// Independent model of how many consistency checks a call sequence triggers.
///
/// Mirrors chunking (greedy prefix of whole calls up to `chunk_cap`), batch
/// grouping within a chunk (whole calls, fire on crossing), and the per-chunk
/// residual partial batch.
pub fn reference_checks(sizes: &[usize], batch: usize, chunk_cap: usize) -> usize {
    let mut checks = 0;
    let mut idx = 0;

    while idx < sizes.len() {
        let start = idx;
        let mut gates = 0;
        while idx < sizes.len() {
            gates += sizes[idx];
            idx += 1;
            if gates >= chunk_cap {
                break;
            }
        }

        let mut pending = 0;
        for &size in &sizes[start..idx] {
            pending += size;
            if pending >= batch {
                checks += 1;
                pending = 0;
            }
        }
        if pending > 0 {
            checks += 1;
        }
    }

    checks
}

/// Per-side cumulative `(allocated, consumed)` OT counts.
pub struct Counts {
    pub prover: (usize, usize),
    pub verifier: (usize, usize),
}

/// Runs one prover/verifier pair over the given call sizes and config, with all
/// inputs committed up front.
pub async fn run_case(sizes: &[usize], batch: usize, chunk_cap: usize, seed: u64) -> Counts {
    let mut rng = StdRng::seed_from_u64(seed);
    let delta = Delta::random(&mut rng);
    let (ot_send, ot_recv) = ideal_rcot(rng.random(), delta.into_inner());
    let ot_recv = CountedReceiver::new(ot_recv);
    let ot_send = CountedSender::new(ot_send);

    let prover_counters = ot_recv.counters();
    let verifier_counters = ot_send.counters();

    let prover_config = ProverConfig::builder()
        .batch_size(batch)
        .chunk_cap(chunk_cap)
        .build()
        .unwrap();
    let verifier_config = VerifierConfig::builder()
        .batch_size(batch)
        .chunk_cap(chunk_cap)
        .build()
        .unwrap();

    let mut prover = Prover::new(prover_config, ot_recv);
    let mut verifier = Verifier::new(verifier_config, delta, ot_send);

    let (mut ctx_p, mut ctx_v) = test_st_context(1 << 20);

    // A single private key, reused across all calls.
    let p_key: Array<U8, 16> = prover.alloc().unwrap();
    let v_key: Array<U8, 16> = verifier.alloc().unwrap();
    prover.mark_private(p_key).unwrap();
    verifier.mark_blind(v_key).unwrap();
    prover.assign(p_key, [7u8; 16]).unwrap();
    prover.commit(p_key).unwrap();
    verifier.commit(v_key).unwrap();

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
        prover.commit(p_msg).unwrap();
        verifier.commit(v_msg).unwrap();

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

        p_outputs.push(prover.decode(p_out).unwrap());
        v_outputs.push(verifier.decode(v_out).unwrap());
    }

    let (prover_result, verifier_result) = futures::join!(
        prover.execute_all(&mut ctx_p),
        verifier.execute_all(&mut ctx_v),
    );
    prover_result.unwrap();
    verifier_result.unwrap();

    // Drain the outputs so both stores are complete.
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

/// Calibrates the constant per-side overhead (the reused private key's MAC).
pub async fn calibrate() -> (usize, usize) {
    let base = run_case(&[], 4096, 4096, 0).await;
    assert_eq!(base.prover, base.verifier, "baseline asymmetry");
    base.prover
}
