use futures::{executor::block_on, future::try_join};
use mpz_common::context::test_st_context;
use mpz_core::Block;
use mpz_ot::ideal::rcot::{IdealRCOTReceiver, IdealRCOTSender, ideal_rcot};
use mpz_vm_core::{Param, Vm, value::Value};
use mpz_vm_ir::Module;
use mpz_vm_test_harness::{SpecConfig, SpecVm, run_suite, suites};
use mpz_vm_zk::{Config, Prover, Verifier, ZkVmError};
use rand::{Rng, SeedableRng, rngs::StdRng};

struct ZkPair {
    prover: Prover<IdealRCOTReceiver>,
    verifier: Verifier<IdealRCOTSender>,
}

impl SpecVm for ZkPair {
    type Error = ZkVmError;

    fn variants() -> Vec<String> {
        vec![
            String::new(),
            "chunk64".to_string(),
            "seg16".to_string(),
            "chunk64seg16".to_string(),
        ]
    }

    fn instantiate(module: &Module, variant: &str) -> Result<Self, String> {
        let (cap, segment_cost) = match variant {
            "" => (None, None),
            "chunk64" => (Some(64), None),
            "seg16" => (None, Some(16)),
            "chunk64seg16" => (Some(64), Some(16)),
            other => return Err(format!("unknown variant: {other}")),
        };
        let mut rng = StdRng::seed_from_u64(0);
        let mut delta: Block = rng.random();
        delta.set_lsb(true);
        let (svole_sender, svole_receiver) = ideal_rcot(rng.random(), delta);
        let config = Config::builder()
            .chunk_cap(cap)
            .segment_cost(segment_cost)
            .build();
        let prover = Prover::new_with_config(module.clone(), svole_receiver, config.clone())
            .map_err(|e| format!("{:?}", e))?;
        let verifier = Verifier::new_with_config(module.clone(), svole_sender, config)
            .map_err(|e| format!("{:?}", e))?;
        Ok(Self { prover, verifier })
    }

    fn run(
        &mut self,
        func_idx: u32,
        params_a: Vec<Param>,
        params_b: Vec<Param>,
    ) -> Result<(Option<Value>, Option<Value>), ZkVmError> {
        let (mut ctx_p, mut ctx_v) = test_st_context(1024 * 1024);
        block_on(try_join(
            self.prover.call(&mut ctx_p, func_idx, params_a),
            self.verifier.call(&mut ctx_v, func_idx, params_b),
        ))
    }

    fn is_expected_unsupported(err: &ZkVmError) -> bool {
        err.is_expected_unsupported()
    }
}

fn zk_config() -> SpecConfig {
    SpecConfig {
        run_private_passes: true,
    }
}

#[test]
fn spec_all() {
    let (mut passed, mut failed, mut skipped) = (0usize, 0usize, 0usize);
    let mut failures: Vec<String> = Vec::new();
    for &(name, wast) in suites::ALL {
        let stats = run_suite::<ZkPair>(wast, &zk_config());
        println!(
            "{name}: {} passed, {} failed, {} skipped",
            stats.passed, stats.failed, stats.skipped
        );
        for (category, count) in stats.skip_summary() {
            println!("  skipped {count:>5}  {category}");
        }
        for msg in &stats.failure_messages {
            failures.push(format!("[{name}] {msg}"));
        }
        passed += stats.passed;
        failed += stats.failed;
        skipped += stats.skipped;
    }
    println!("TOTAL: {passed} passed, {failed} failed, {skipped} skipped");
    for (i, msg) in failures.iter().enumerate() {
        println!("  failure {}. {}", i + 1, msg);
    }
    assert_eq!(failed, 0, "zkVM spec suites had {failed} failure(s)");
}
