use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use futures::executor::block_on;
use mpz_common::{Context, context::test_mt_context, executor::Executor};
use mpz_core::Block;
use mpz_ot::{chou_orlandi, ferret, softspoken};
use mpz_vm_core::{Param, Vm, Write, value::Value};
use mpz_vm_ir::{ExportKind, Module};
use mpz_vm_zk::{Prover, Verifier};
use rand::{Rng, SeedableRng, rngs::StdRng};
use sha2::{Digest, Sha256};

type ProverSvole = ferret::Receiver<softspoken::Receiver<chou_orlandi::Sender>>;
type VerifierSvole = ferret::Sender<softspoken::Sender<chou_orlandi::Receiver>>;

fn rcot_stack(seed: u64) -> (VerifierSvole, ProverSvole) {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut delta: Block = rng.random();
    delta.set_lsb(true);

    let verifier = ferret::Sender::new(
        ferret::FerretConfig::default(),
        rng.random(),
        softspoken::Sender::new(
            softspoken::SenderConfig::default(),
            delta,
            chou_orlandi::Receiver::new(),
        ),
    );
    let prover = ferret::Receiver::new(
        ferret::FerretConfig::default(),
        rng.random(),
        softspoken::Receiver::new(
            softspoken::ReceiverConfig::default(),
            chou_orlandi::Sender::new(),
        ),
    );
    (verifier, prover)
}

fn func_idx(module: &Module, name: &str) -> u32 {
    module
        .exports()
        .iter()
        .find_map(|e| match e.kind {
            ExportKind::Func(idx) if e.name == name => Some(idx),
            _ => None,
        })
        .expect("function should be exported")
}

struct Session {
    exec_p: Executor,
    exec_v: Executor,
    ctx_p: Context,
    ctx_v: Context,
}

impl Session {
    fn new() -> Self {
        let (exec_p, exec_v) = test_mt_context(32 << 20);
        let ctx_p = exec_p.new_context().unwrap();
        let ctx_v = exec_v.new_context().unwrap();
        Self {
            exec_p,
            exec_v,
            ctx_p,
            ctx_v,
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.exec_p.shutdown();
        self.exec_v.shutdown();
    }
}

fn call_both(
    prover: &mut Prover<ProverSvole>,
    verifier: &mut Verifier<VerifierSvole>,
    ctx_p: &mut Context,
    ctx_v: &mut Context,
    func: u32,
    p_params: Vec<Param>,
    v_params: Vec<Param>,
) -> (Option<Value>, Option<Value>) {
    std::thread::scope(|s| {
        let hp = s.spawn(move || block_on(prover.call(ctx_p, func, p_params)).unwrap());
        let hv = s.spawn(move || block_on(verifier.call(ctx_v, func, v_params)).unwrap());
        (hp.join().unwrap(), hv.join().unwrap())
    })
}

const DIGEST_LEN: usize = 32;

fn prove_sha256(module: &Module, msg: &[u8], session: &mut Session, func: &str) -> Vec<u8> {
    let (v_svole, p_svole) = rcot_stack(0);
    let mut prover = Prover::new(module.clone(), p_svole).unwrap();
    let mut verifier = Verifier::new(module.clone(), v_svole).unwrap();
    let Session { ctx_p, ctx_v, .. } = session;

    let realloc = func_idx(module, "cabi_realloc");
    let alloc_args = || {
        vec![
            Param::Public(Value::I32(0)),
            Param::Public(Value::I32(0)),
            Param::Public(Value::I32(1)),
            Param::Public(Value::I32(msg.len() as i32)),
        ]
    };
    let (rp, rv) = call_both(
        &mut prover,
        &mut verifier,
        ctx_p,
        ctx_v,
        realloc,
        alloc_args(),
        alloc_args(),
    );
    assert_eq!(
        rp, rv,
        "cabi_realloc must return the same pointer on both sides"
    );
    let ptr = match rp {
        Some(Value::I32(p)) => p as u32,
        other => panic!("cabi_realloc returned {other:?}"),
    };
    prover.write(ptr, Write::Private(msg)).unwrap();
    verifier.write(ptr, Write::Blind(msg.len())).unwrap();

    let hash = func_idx(module, func);
    let hash_args = || {
        vec![
            Param::Public(Value::I32(ptr as i32)),
            Param::Public(Value::I32(msg.len() as i32)),
        ]
    };
    let (rp, rv) = call_both(
        &mut prover,
        &mut verifier,
        ctx_p,
        ctx_v,
        hash,
        hash_args(),
        hash_args(),
    );
    assert_eq!(rp, rv, "prover and verifier results must agree");
    let digest_ptr = match rp {
        Some(Value::I32(p)) => p as u32,
        other => panic!("hash returned {other:?}"),
    };
    verifier.read(digest_ptr, DIGEST_LEN).unwrap().to_vec()
}

fn init_tracing() {
    use tracing_subscriber::{EnvFilter, fmt::format::FmtSpan};

    let Ok(filter) = EnvFilter::try_from_default_env() else {
        return;
    };
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_span_events(FmtSpan::CLOSE)
        .with_target(true)
        .with_writer(std::io::stderr)
        .try_init();
}

const SHA256_SIZES: &[usize] = &[4096, 16384];

const SHA256_VARIANTS: &[(&str, &str)] = &[("wasm", "hash"), ("precompile", "hash_precompile")];

fn bench_sha256(c: &mut Criterion) {
    init_tracing();
    let wasm = include_bytes!("guests/sha256.wasm");
    let module = Module::parse(wasm).unwrap();

    let mut session = Session::new();
    let mut group = c.benchmark_group("zk-vm/sha256");
    group.sample_size(10);
    for &len in SHA256_SIZES {
        let msg: Vec<u8> = (0..len).map(|i| i as u8).collect();
        for &(label, func) in SHA256_VARIANTS {
            let digest = prove_sha256(&module, &msg, &mut session, func);
            assert_eq!(
                digest,
                Sha256::digest(&msg).as_slice(),
                "{label} variant digest must match reference SHA-256 for {len} bytes"
            );

            group.throughput(Throughput::Bytes(len as u64));
            group.bench_with_input(BenchmarkId::new(label, len), &msg, |b, msg| {
                b.iter(|| prove_sha256(&module, msg, &mut session, func))
            });
        }
    }
    group.finish();
}

criterion_group!(benches, bench_sha256);
criterion_main!(benches);
