//! End-to-end private hashing with the production oblivious-transfer protocols.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicUsize, Ordering},
};

use mpz_common::context::test_st_context;
use mpz_core::Block;
use mpz_hash::sha256::Sha256;
use mpz_ot::{
    chou_orlandi as co, ferret, kos,
    rcot::shared::{SharedRCOTReceiver, SharedRCOTSender},
};
use mpz_vm_core::{
    Vm,
    memory::{
        binary::{Binary, U8},
        correlated::Delta,
    },
    prelude::*,
};
use mpz_zk::{Prover, ProverConfig, Verifier, VerifierConfig};
use rand::{SeedableRng, rngs::StdRng};
use sha2::Digest;

struct MeasuredAllocator;
static LIVE_BYTES: AtomicUsize = AtomicUsize::new(0);
static PEAK_BYTES: AtomicUsize = AtomicUsize::new(0);
static TOTAL_ALLOCATION_BYTES: AtomicUsize = AtomicUsize::new(0);

fn allocated(size: usize) {
    TOTAL_ALLOCATION_BYTES.fetch_add(size, Ordering::Relaxed);
    let live = LIVE_BYTES.fetch_add(size, Ordering::Relaxed) + size;
    PEAK_BYTES.fetch_max(live, Ordering::Relaxed);
}

// WHY: measure the real protocol's heap, including OT buffers, instead of
// replacing its allocator or OT implementation with a capacity-limited fake.
unsafe impl GlobalAlloc for MeasuredAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            allocated(layout.size());
        }
        ptr
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            allocated(layout.size());
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE_BYTES.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe {
            System.dealloc(ptr, layout);
        }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let new_ptr = unsafe { System.realloc(ptr, layout, size) };
        if !new_ptr.is_null() {
            LIVE_BYTES.fetch_sub(layout.size(), Ordering::Relaxed);
            allocated(size);
        }
        new_ptr
    }
}

#[global_allocator]
static ALLOCATOR: MeasuredAllocator = MeasuredAllocator;

fn enqueue_hash(
    vm: &mut dyn Vm<Binary>,
    data: &[u8],
    is_prover: bool,
    public_prefix: u8,
) -> mpz_vm_core::memory::DecodeFutureTyped<mpz_core::bitvec::BitVec, [u8; 32]> {
    let private = if data.is_empty() {
        None
    } else {
        let private = vm.alloc_vec::<U8>(data.len()).unwrap();
        if is_prover {
            vm.mark_private(private).unwrap();
            vm.assign(private, data.to_vec()).unwrap();
        } else {
            vm.mark_blind(private).unwrap();
        }
        vm.commit(private).unwrap();
        Some(private)
    };
    let prefix = vm.alloc_vec::<U8>(1).unwrap();
    vm.mark_public(prefix).unwrap();
    vm.assign(prefix, vec![public_prefix]).unwrap();
    vm.commit(prefix).unwrap();
    let mut hasher = Sha256::new();
    hasher.update(&prefix);
    if let Some(private) = private {
        hasher.update(&private);
    }
    let digest = hasher.finalize(vm).unwrap();
    vm.decode(digest).unwrap()
}

#[tokio::test]
async fn private_sha256_uses_bounded_real_ot_and_rejects_inconsistent_public_input() {
    let mut rng = StdRng::seed_from_u64(42);
    let delta = Delta::random(&mut rng);
    let config = ferret::FerretConfig::builder().build().unwrap();
    let send = ferret::Sender::new(
        config.clone(),
        Block::random(&mut rng),
        kos::Sender::new(
            kos::SenderConfig::default(),
            delta.into_inner(),
            co::Receiver::new(),
        ),
    );
    let recv = ferret::Receiver::new(
        config,
        Block::random(&mut rng),
        kos::Receiver::new(kos::ReceiverConfig::default(), co::Sender::new()),
    );
    let mut prover = Prover::new(
        ProverConfig::builder()
            .batch_gate_ots(true)
            .build()
            .unwrap(),
        SharedRCOTReceiver::new(recv),
    );
    let mut verifier = Verifier::new(
        VerifierConfig::builder()
            .batch_gate_ots(true)
            .build()
            .unwrap(),
        delta,
        SharedRCOTSender::new(send),
    );
    let (mut ctx_p, mut ctx_v) = test_st_context(8);

    // WHY: enqueue a long dependent hash before executing it, just as a private
    // TLS response commitment does. Eager gate OTs used to retain the entire
    // callstack's preprocessing even though only one compression is ready.
    for size in [0, 65, 32 * 1024, 257] {
        let data: Vec<u8> = (0..size).map(|i| (i * 31 + 7) as u8).collect();
        let mut digest_p = enqueue_hash(&mut prover, &data, true, 42);
        let mut digest_v = enqueue_hash(&mut verifier, &data, false, 42);
        let (p, v) = futures::join!(
            prover.execute_all(&mut ctx_p),
            verifier.execute_all(&mut ctx_v)
        );
        p.unwrap();
        v.unwrap();
        let mut expected = sha2::Sha256::new();
        expected.update([42]);
        expected.update(&data);
        let expected: [u8; 32] = expected.finalize().into();
        assert_eq!(digest_p.try_recv().unwrap().unwrap(), expected);
        assert_eq!(digest_v.try_recv().unwrap().unwrap(), expected);
    }
    let peak = PEAK_BYTES.load(Ordering::Relaxed);
    eprintln!("peak Rust allocation: {peak} bytes");
    eprintln!(
        "total successful allocation requests: {} bytes",
        TOTAL_ALLOCATION_BYTES.load(Ordering::Relaxed)
    );
    assert!(
        peak < 256 * 1024 * 1024,
        "private SHA-256 peaked at {peak} heap bytes"
    );

    // WHY: batching must preserve the consistency proof; equal lengths and
    // honest OT alone must not make mismatched circuit inputs acceptable.
    drop(enqueue_hash(&mut prover, b"private response", true, 42));
    drop(enqueue_hash(&mut verifier, b"private response", false, 43));
    let (p, v) = futures::join!(
        prover.execute_all(&mut ctx_p),
        verifier.execute_all(&mut ctx_v)
    );
    p.unwrap();
    let error = v.expect_err("inconsistent public input was accepted");
    assert_eq!(
        format!("{error:?}"),
        "VmError(Execute(VerifierError(Check(Invalid))))",
        "the consistency proof must reject the input, not an unrelated failure"
    );
}
