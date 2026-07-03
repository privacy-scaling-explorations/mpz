mod common;

use futures::{executor::block_on, future::join};
use mpz_common::context::test_st_context;
use mpz_ot::ideal::rcot::ideal_rcot;
use mpz_vm_core::{Param, Vm, value::Value};
use mpz_vm_ir::{ExportKind, Module, ValType};
use mpz_vm_zk::{Config, Prover, Verifier};
use rand::{SeedableRng, rngs::StdRng};

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

fn blind(v: &Value) -> Param {
    match v {
        Value::I32(_) => Param::Blind(ValType::I32),
        Value::I64(_) => Param::Blind(ValType::I64),
        Value::F32(_) => Param::Blind(ValType::F32),
        Value::F64(_) => Param::Blind(ValType::F64),
    }
}

fn run(wat: &str, func: &str, inputs: &[Value], expected: Value, chunk_cap: Option<usize>) {
    common::init_tracing();
    let module = Module::parse(&wat::parse_str(wat).expect("valid WAT")).expect("valid module");
    let idx = func_idx(&module, func);

    let mut rng = StdRng::seed_from_u64(0);
    let mut delta: mpz_core::Block = rand::Rng::random(&mut rng);
    delta.set_lsb(true);
    let (svole_sender, svole_receiver) = ideal_rcot(rand::Rng::random(&mut rng), delta);

    let config = Config::builder().chunk_cap(chunk_cap).build();
    let mut prover =
        Prover::new_with_config(module.clone(), svole_receiver, config.clone()).unwrap();
    let mut verifier = Verifier::new_with_config(module, svole_sender, config).unwrap();

    let (mut ctx_p, mut ctx_v) = test_st_context(1024 * 1024);

    let p_params: Vec<Param> = inputs.iter().cloned().map(Param::Private).collect();
    let v_params: Vec<Param> = inputs.iter().map(blind).collect();

    let (result_p, result_v) = block_on(join(
        async { prover.call(&mut ctx_p, idx, p_params).await.unwrap() },
        async { verifier.call(&mut ctx_v, idx, v_params).await.unwrap() },
    ));

    assert_eq!(result_p, Some(expected), "prover result for `{func}`");
    assert_eq!(
        result_v,
        Some(expected),
        "verifier must learn the reveal for `{func}`"
    );
}

#[test]
fn scalar_reveal_discloses_private_input() {
    let wat = r#"
        (module
          (import "vc" "reveal_i32" (func $reveal (param i32) (result i32)))
          (import "vc" "reveal_i32_wait" (func $wait (param i32) (result i32)))
          (func (export "disclose") (param i32) (result i32)
            (call $wait (call $reveal (local.get 0)))))
    "#;
    run(wat, "disclose", &[Value::I32(42)], Value::I32(42), None);
}

#[test]
fn revealed_value_drives_a_branch() {
    let wat = r#"
        (module
          (import "vc" "reveal_i32" (func $reveal (param i32) (result i32)))
          (import "vc" "reveal_i32_wait" (func $wait (param i32) (result i32)))
          (func (export "branch") (param i32) (result i32)
            (local $x i32)
            (local.set $x (call $wait (call $reveal (local.get 0))))
            (if (result i32) (i32.gt_s (local.get $x) (i32.const 5))
              (then (i32.const 100))
              (else (i32.const 200)))))
    "#;
    run(wat, "branch", &[Value::I32(7)], Value::I32(100), None);
    run(wat, "branch", &[Value::I32(3)], Value::I32(200), None);
}

#[test]
fn reveal_and_wait_span_chunks() {
    let wat = r#"
        (module
          (import "vc" "reveal_i32" (func $reveal (param i32) (result i32)))
          (import "vc" "reveal_i32_wait" (func $wait (param i32) (result i32)))
          (func (export "spanned") (param i32) (result i32)
            (local $h i32)
            (local.set $h (call $reveal (local.get 0)))
            (drop (i32.add (i32.const 1) (i32.const 2)))
            (drop (i32.add (i32.const 3) (i32.const 4)))
            (drop (i32.add (i32.const 5) (i32.const 6)))
            (call $wait (local.get $h))))
    "#;
    run(wat, "spanned", &[Value::I32(99)], Value::I32(99), Some(1));
}

#[test]
fn two_staged_reveals_resolve_by_handle() {
    let wat = r#"
        (module
          (import "vc" "reveal_i32" (func $reveal (param i32) (result i32)))
          (import "vc" "reveal_i32_wait" (func $wait (param i32) (result i32)))
          (func (export "sum") (param i32 i32) (result i32)
            (local $ha i32) (local $hb i32)
            (local.set $ha (call $reveal (local.get 0)))
            (local.set $hb (call $reveal (local.get 1)))
            (i32.add (call $wait (local.get $ha)) (call $wait (local.get $hb)))))
    "#;
    run(
        wat,
        "sum",
        &[Value::I32(10), Value::I32(20)],
        Value::I32(30),
        None,
    );
}

#[test]
fn byte_reveal_discloses_private_memory() {
    common::init_tracing();
    let wat = r#"
        (module
          (import "vc" "reveal_bytes" (func $reveal (param i32 i32) (result i32)))
          (import "vc" "reveal_bytes_wait" (func $wait (param i32)))
          (memory 1)
          (func (export "disclose_bytes") (param i32) (result i32)
            (i32.store (i32.const 0) (local.get 0))
            (call $wait (call $reveal (i32.const 0) (i32.const 4)))
            (i32.load (i32.const 0))))
    "#;
    let secret = 0x1234_5678u32 as i32;
    run(
        wat,
        "disclose_bytes",
        &[Value::I32(secret)],
        Value::I32(secret),
        None,
    );
}
