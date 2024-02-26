use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use mpz_core::Block;
use mpz_ot::{
    chou_orlandi::{Receiver, ReceiverConfig, Sender, SenderConfig},
    OTReceiver, OTSender, OTSetup,
};
use utils_aio::duplex::MemoryDuplex;

fn chou_orlandi(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("chou_orlandi");
    for n in [128, 256, 1024] {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            let msgs = vec![[Block::ONES; 2]; n];
            let choices = vec![false; n];
            b.to_async(&rt).iter(|| async {
                let (sender_channel, receiver_channel) = MemoryDuplex::new();

                let mut sender = Sender::new(SenderConfig::default(), sender_channel);
                let mut receiver = Receiver::new(ReceiverConfig::default(), receiver_channel);

                let (sender_res, receiver_res) = futures::join!(sender.setup(), receiver.setup());

                sender_res.unwrap();
                receiver_res.unwrap();

                let (sender_res, receiver_res) =
                    futures::join!(sender.send(&msgs), receiver.receive(&choices));

                sender_res.unwrap();
                let received = receiver_res.unwrap();

                black_box(received)
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
