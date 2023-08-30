use criterion::{black_box, criterion_group, criterion_main, Criterion};
use mpz_core::{block::Block, ggm_tree::GgmTree};

#[allow(clippy::all)]
fn criterion_benchmark(c: &mut Criterion) {
    c.bench_function("ggm::gen::1K", move |bench| {
        let depth = 11;
        let ggm = GgmTree::new(depth);
        let mut tree = vec![Block::ZERO; 1 << (depth - 1)];
        let mut k0 = vec![Block::ZERO; depth - 1];
        let mut k1 = vec![Block::ZERO; depth - 1];
        let seed = rand::random::<Block>();
        bench.iter(|| {
            black_box(ggm.gen(
                black_box(seed),
                black_box(&mut tree),
                black_box(&mut k0),
                black_box(&mut k1),
            ));
        });
    });
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
