use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use mpz_core::Block;
use mpz_ot_core::chou_orlandi;
use rand::{RngCore, SeedableRng};
use rand_chacha::ChaCha12Rng;

fn chou_orlandi(c: &mut Criterion) {
    let mut group = c.benchmark_group("chou_orlandi");
    for n in [128, 256, 1024] {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            let msgs = vec![[Block::ONES; 2]; n];
            let mut rng = ChaCha12Rng::from_entropy();
            let mut choices = vec![0u8; n / 8];
            rng.fill_bytes(&mut choices);
            b.iter(|| {
                let sender = chou_orlandi::Sender::default();
                let receiver = chou_orlandi::Receiver::default();

                let (sender_setup, sender) = sender.setup();
                let (receiver_setup, mut receiver) = receiver.setup(sender_setup);
                let mut sender = sender.receive_setup(receiver_setup).unwrap();

                let receiver_payload = receiver.receive_random(choices.as_slice());
                let sender_payload = sender.send(&msgs, receiver_payload).unwrap();
                black_box(receiver.receive(sender_payload).unwrap())
            })
        });
    }
}

criterion_group! {
    name = chou_orlandi_benches;
    config = Criterion::default().sample_size(50);
    targets = chou_orlandi
}

criterion_main!(chou_orlandi_benches);
