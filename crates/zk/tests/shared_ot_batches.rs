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
use std::time::Duration;

fn enqueue(
    vm: &mut dyn Vm<Binary>,
    data: &[u8],
    is_prover: bool,
) -> mpz_vm_core::memory::DecodeFutureTyped<mpz_core::bitvec::BitVec, [u8; 32]> {
    let private = vm.alloc_vec::<U8>(data.len()).unwrap();
    if is_prover {
        vm.mark_private(private).unwrap();
        vm.assign(private, data.to_vec()).unwrap();
    } else {
        vm.mark_blind(private).unwrap();
    }
    vm.commit(private).unwrap();
    let mut hasher = Sha256::new();
    hasher.update(&private);
    let digest = hasher.finalize(vm).unwrap();
    vm.decode(digest).unwrap()
}

#[tokio::test]
async fn shared_real_ot_completes_unequal_hash_batches() {
    let mut rng = StdRng::seed_from_u64(43);
    let delta = Delta::random(&mut rng);
    let config = ferret::FerretConfig::builder()
        .build()
        .unwrap();
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
    let send = SharedRCOTSender::new(send);
    let recv = SharedRCOTReceiver::new(recv);
    let mut prover_a = Prover::new(ProverConfig::default(), recv.clone());
    let mut verifier_a = Verifier::new(VerifierConfig::default(), delta, send.clone());
    let mut prover_b = Prover::new(ProverConfig::default(), recv);
    let mut verifier_b = Verifier::new(VerifierConfig::default(), delta, send);
    let (mut ctx_pa, mut ctx_va) = test_st_context(8);
    let (mut ctx_pb, mut ctx_vb) = test_st_context(8);
    let short = vec![7; 65];
    let long = vec![8; 129];
    let mut pa = enqueue(&mut prover_a, &short, true);
    let mut va = enqueue(&mut verifier_a, &short, false);
    let mut pb = enqueue(&mut prover_b, &long, true);
    let mut vb = enqueue(&mut verifier_b, &long, false);

    // WHY: a reusable VM may finish its current work while another VM that
    // shares preprocessing still has dependent calls. Finishing the short
    // proof must not strand the long proof inside a collective OT barrier.
    tokio::time::timeout(Duration::from_secs(10), async {
        let (pa, va, pb, vb) = futures::join!(
            prover_a.execute_all(&mut ctx_pa),
            verifier_a.execute_all(&mut ctx_va),
            prover_b.execute_all(&mut ctx_pb),
            verifier_b.execute_all(&mut ctx_vb),
        );
        pa.unwrap();
        va.unwrap();
        pb.unwrap();
        vb.unwrap();
    })
    .await
    .expect("shared OT proof stalled after the shorter VM finished");
    let short_hash: [u8; 32] = sha2::Sha256::digest(&short).into();
    let long_hash: [u8; 32] = sha2::Sha256::digest(&long).into();
    assert_eq!(pa.try_recv().unwrap().unwrap(), short_hash);
    assert_eq!(va.try_recv().unwrap().unwrap(), short_hash);
    assert_eq!(pb.try_recv().unwrap().unwrap(), long_hash);
    assert_eq!(vb.try_recv().unwrap().unwrap(), long_hash);
}
