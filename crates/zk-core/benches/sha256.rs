//! SHA-256 prover/verifier benchmarks over message sizes from one
//! block (64 B) up to 128 KiB.
//!
//! Run with: `cargo bench -p mpz-zk-core --bench sha256`

use std::{hint::black_box, time::Duration};

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use itybity::ToBits;
use mpz_circuits::{
    Context,
    sha256::{AND_PER_BLOCK, H0, compress as sha256_compress},
};
use mpz_core::Block;
use mpz_fields::{gf2::Gf2, gf2_128::Gf2_128};
use mpz_ot_core::ideal::rcot::IdealRCOT;
use mpz_zk_core::{
    Accumulate, Proof, ProverOutput, Verifier, VerifierOutput, Witness, vope_receiver, vope_sender,
};
use rand::{Rng, SeedableRng, rngs::StdRng};
use rand_chacha::ChaCha12Rng;

const VOPE_COST: usize = 128;

/// Samples `total` RCOT correlations as `(delta, keys, choices, macs)`
/// with `delta.lsb = 1`.
fn sample_rcot<R: Rng>(
    rng: &mut R,
    total: usize,
) -> (Gf2_128, Vec<Gf2_128>, Vec<bool>, Vec<Gf2_128>) {
    let mut delta_block: Block = rng.random();
    delta_block.set_lsb(true);
    let seed: Block = rng.random();

    let mut rcot = IdealRCOT::new(seed, delta_block);
    rcot.alloc(total);
    rcot.flush().expect("ideal rcot flush");
    let (sender_out, receiver_out) = rcot.transfer(total).expect("ideal rcot transfer");

    (
        delta_block.into(),
        sender_out.keys.into_iter().map(Into::into).collect(),
        receiver_out.choices,
        receiver_out.msgs.into_iter().map(Into::into).collect(),
    )
}

/// Sets the LSB of `g` to `bit`.
fn set_lsb(g: Gf2_128, bit: bool) -> Gf2_128 {
    Gf2_128::new((g.to_inner() & !1) | u128::from(bit))
}

/// Iterated SHA-256 compression over a sequence of message blocks.
fn sha256_chain<C: Context<Field = Gf2>>(
    ctx: &mut C,
    initial_state: [C::Wire; 256],
    msg_blocks: &[[C::Wire; 512]],
) -> [C::Wire; 256] {
    let mut state = initial_state;
    for block in msg_blocks {
        state = sha256_compress(ctx, *block, state);
    }
    state
}

const SIZES: &[(usize, &str)] = &[(16 * 1024, "16KiB")];

struct BenchInputs {
    delta: Gf2_128,
    /// Prover witness pass: initial state + message blocks as cleartext bit
    /// wires, laid out once so the benched pass measures only evaluation.
    state_p_bits: [Gf2; 256],
    msg_p_bits: Vec<[Gf2; 512]>,
    /// Prover accumulate pass: initial state + message blocks (MAC wires).
    state_p: [Gf2_128; 256],
    msg_p: Vec<[Gf2_128; 512]>,
    /// Verifier initial state + message blocks (key wires).
    state_v: [Gf2_128; 256],
    msg_v: Vec<[Gf2_128; 512]>,
    /// Gate masks as committed (cloned per prover iteration, since
    /// the witness pass overwrites them with the adjust bits in place).
    gate_masks: Vec<bool>,
    gate_macs: Vec<Gf2_128>,
    gate_keys: Vec<Gf2_128>,
    /// Gate adjust bits the prover produced, fed to the verifier.
    gate_adjust: Vec<bool>,
    vope_keys: [Gf2_128; VOPE_COST],
    /// Seed of the consistency-check challenge stream.
    chi: [u8; 32],
    /// Proof produced by the prover, consumed in the verifier benchmark.
    proof: Proof,
}

fn setup_inputs(num_blocks: usize) -> BenchInputs {
    let mut rng = StdRng::seed_from_u64(0);

    // Initial state + N message blocks, each 512 bits.
    let mut input_bits: Vec<bool> = H0.iter_lsb0().collect();
    for _ in 0..num_blocks {
        let block: [u32; 16] = core::array::from_fn(|_| rng.random());
        input_bits.extend(block.iter_lsb0());
    }
    let input_count = input_bits.len();
    let gate_count = num_blocks * AND_PER_BLOCK;
    let total = input_count + gate_count + VOPE_COST;

    let (delta, raw_keys, choices, macs) = sample_rcot(&mut rng, total);

    let input_adjust: Vec<bool> = (0..input_count)
        .map(|i| input_bits[i] ^ choices[i])
        .collect();
    let input_mac_wires: Vec<Gf2_128> = (0..input_count)
        .map(|i| set_lsb(macs[i], input_bits[i]))
        .collect();
    let input_key_wires: Vec<Gf2_128> = (0..input_count)
        .map(|i| {
            let k = raw_keys[i];
            let key = if input_adjust[i] { k + delta } else { k };
            set_lsb(key, false)
        })
        .collect();

    let main_cost = input_count + gate_count;
    let gate_masks: Vec<bool> = choices[input_count..main_cost].to_vec();
    let gate_macs: Vec<Gf2_128> = macs[input_count..main_cost].to_vec();
    let gate_keys: Vec<Gf2_128> = raw_keys[input_count..main_cost].to_vec();

    let vope_choices: [bool; VOPE_COST] = core::array::from_fn(|i| choices[main_cost + i]);
    let vope_ev: [Gf2_128; VOPE_COST] = core::array::from_fn(|i| macs[main_cost + i]);
    let vope_keys: [Gf2_128; VOPE_COST] = core::array::from_fn(|i| raw_keys[main_cost + i]);
    let chi: [u8; 32] = rng.random();

    // Lay out the prover (MAC) and verifier (key) wires as initial state +
    // message blocks once, so the benched passes never rebuild them.
    let state_p: [Gf2_128; 256] = core::array::from_fn(|i| input_mac_wires[i]);
    let msg_p: Vec<[Gf2_128; 512]> = (0..num_blocks)
        .map(|b| core::array::from_fn(|i| input_mac_wires[256 + b * 512 + i]))
        .collect();
    let state_v: [Gf2_128; 256] = core::array::from_fn(|i| input_key_wires[i]);
    let msg_v: Vec<[Gf2_128; 512]> = (0..num_blocks)
        .map(|b| core::array::from_fn(|i| input_key_wires[256 + b * 512 + i]))
        .collect();
    let state_p_bits: [Gf2; 256] = core::array::from_fn(|i| Gf2(input_bits[i]));
    let msg_p_bits: Vec<[Gf2; 512]> = (0..num_blocks)
        .map(|b| core::array::from_fn(|i| Gf2(input_bits[256 + b * 512 + i])))
        .collect();

    // Run the prover once to produce a valid proof and the gate adjust
    // bits for the verifier benchmark.
    let mut gate_adjust = gate_masks.clone();
    let mut commit = Witness::new(&mut gate_adjust);
    let _ = sha256_chain(&mut commit, state_p_bits, &msg_p_bits);
    commit.finish().expect("commit finish");
    let mut prover = Accumulate::new(&gate_macs, ChaCha12Rng::from_seed(chi));
    let _ = sha256_chain(&mut prover, state_p, &msg_p);
    let ProverOutput {
        u, v, assertions, ..
    } = prover.finish().expect("accumulate finish");

    let (a_0, a_1) = vope_receiver(&vope_choices, &vope_ev);
    let proof = Proof {
        assertions,
        u: u + a_0,
        v: v + a_1,
        coefficients: Vec::new(),
    };

    BenchInputs {
        delta,
        state_p_bits,
        msg_p_bits,
        state_p,
        msg_p,
        state_v,
        msg_v,
        gate_masks,
        gate_macs,
        gate_keys,
        gate_adjust,
        vope_keys,
        chi,
        proof,
    }
}

fn run_witness(inputs: &BenchInputs) {
    let mut masks = inputs.gate_masks.clone();
    let mut commit = Witness::new(&mut masks);
    let _ = sha256_chain(&mut commit, inputs.state_p_bits, &inputs.msg_p_bits);
    commit.finish().expect("witness finish");
    black_box(&masks);
}

fn run_accumulate(inputs: &BenchInputs) {
    let mut prover = Accumulate::new(&inputs.gate_macs, ChaCha12Rng::from_seed(inputs.chi));
    let _ = sha256_chain(&mut prover, inputs.state_p, &inputs.msg_p);
    black_box(prover.finish().expect("accumulate finish"));
}

fn run_verifier(inputs: &BenchInputs) {
    let mut verifier = Verifier::new(
        inputs.delta,
        &inputs.gate_keys,
        &inputs.gate_adjust,
        ChaCha12Rng::from_seed(inputs.chi),
    )
    .expect("new");
    let _ = sha256_chain(&mut verifier, inputs.state_v, &inputs.msg_v);
    let VerifierOutput { w, assertions, .. } = verifier.finish().expect("finish");

    let b = vope_sender(&inputs.vope_keys);
    assert_eq!(assertions, inputs.proof.assertions);
    assert_eq!(
        w + b,
        inputs.proof.u + inputs.delta * inputs.proof.v,
        "consistency check failed"
    );
}

fn bench_sha256(c: &mut Criterion) {
    // Build, bench, and drop one `BenchInputs` per size before
    // constructing the next: the gate tapes for the largest size are
    // hundreds of MiB, so keeping every size resident at once strains
    // 32-bit targets.
    for &(bytes, name) in SIZES {
        let num_blocks = bytes / 64;
        let inputs = setup_inputs(num_blocks);

        let mut witness_group = c.benchmark_group("sha256_witness");
        witness_group.sample_size(10);
        witness_group.measurement_time(Duration::from_secs(10));
        witness_group.throughput(Throughput::Bytes(bytes as u64));
        witness_group.bench_function(BenchmarkId::new("message", name), |b| {
            b.iter(|| run_witness(&inputs));
        });
        witness_group.finish();

        let mut accumulate_group = c.benchmark_group("sha256_accumulate");
        accumulate_group.sample_size(10);
        accumulate_group.measurement_time(Duration::from_secs(10));
        accumulate_group.throughput(Throughput::Bytes(bytes as u64));
        accumulate_group.bench_function(BenchmarkId::new("message", name), |b| {
            b.iter(|| run_accumulate(&inputs));
        });
        accumulate_group.finish();

        let mut verifier_group = c.benchmark_group("sha256_verifier");
        verifier_group.sample_size(10);
        verifier_group.measurement_time(Duration::from_secs(10));
        verifier_group.throughput(Throughput::Bytes(bytes as u64));
        verifier_group.bench_function(BenchmarkId::new("message", name), |b| {
            b.iter(|| run_verifier(&inputs));
        });
        verifier_group.finish();
    }
}

criterion_group!(benches, bench_sha256);
criterion_main!(benches);
