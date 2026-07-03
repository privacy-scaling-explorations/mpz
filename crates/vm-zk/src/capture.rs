use std::collections::{BTreeMap, BTreeSet};

use mpz_vm_core::{
    Directive, Global, Op, Operand, Pending, Reg, StepResult, Thread, Trap, value::Value,
};
use mpz_vm_ir::{Function, Module};

use std::ops::Range;

use crate::{
    cost,
    error::{Result, ZkVmError},
    host::{self, HostCallEvent, RevealPayload, RevealState},
    memlog::{self, ByteState},
    segment::{BoundaryDelta, MemItem, ValItem},
};

pub(crate) fn is_import(module: &Module, func_idx: u32) -> bool {
    matches!(module.function(func_idx), Some(Function::Import(_)))
}

pub(crate) fn is_precompile(module: &Module, func_idx: u32) -> bool {
    matches!(
        module.function(func_idx),
        Some(Function::Import(import)) if import.module() == "crypto"
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Role {
    Prover,
    Verifier,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Limits {
    pub(crate) chunk_cap: Option<usize>,
    pub(crate) segment_cost: Option<usize>,
}

#[derive(Clone, Debug)]
pub(crate) struct TrapPoint {
    pub(crate) index: u64,
    pub(crate) directive: Option<Directive>,
    pub(crate) trap: Trap,
}

#[derive(Clone, Debug)]
pub(crate) struct SegmentInfo {
    pub(crate) directives: Range<usize>,
    pub(crate) reveals: Range<usize>,
    pub(crate) log: Range<u64>,
    pub(crate) bits: usize,
    pub(crate) gates: usize,
    pub(crate) boundary: Option<BoundaryDelta>,
}

pub(crate) struct ChunkCapture {
    pub(crate) trace: Vec<Directive>,
    pub(crate) cost: usize,
    pub(crate) done: bool,
    pub(crate) result: Option<Value>,
    pub(crate) result_symbolic: bool,
    pub(crate) trap: Option<TrapPoint>,
    pub(crate) reveal_actions: Vec<HostCallEvent>,
    pub(crate) reveals: BTreeMap<u32, RevealPayload>,
    pub(crate) segments: Vec<SegmentInfo>,
}

#[allow(clippy::too_many_arguments)]
fn finish_segments(
    mut segments: Vec<SegmentInfo>,
    dir_start: usize,
    rev_start: usize,
    log_start: u64,
    trace_len: usize,
    reveal_len: usize,
    log_end: u64,
    bits: usize,
    gates: usize,
) -> Vec<SegmentInfo> {
    if dir_start == trace_len && !segments.is_empty() {
        let last = segments.last_mut().expect("non-empty");
        last.boundary = None;
        last.log.end = log_end;
    } else {
        segments.push(SegmentInfo {
            directives: dir_start..trace_len,
            reveals: rev_start..reveal_len,
            log: log_start..log_end,
            bits,
            gates,
            boundary: None,
        });
    }
    segments
}

#[tracing::instrument(level = "debug", name = "capture", skip_all, fields(?role))]
pub(crate) fn capture_chunk(
    module: &Module,
    global: &mut Global,
    thread: &mut Thread,
    limits: Limits,
    role: Role,
    announced_trap: Option<(u64, Trap)>,
    reveal_state: &mut RevealState,
) -> Result<ChunkCapture> {
    let mut trace: Vec<Directive> = Vec::new();
    let mut cost: usize = 0;
    let mut reveal_actions: Vec<HostCallEvent> = Vec::new();
    let mut reveals: BTreeMap<u32, RevealPayload> = BTreeMap::new();
    let mut segments: Vec<SegmentInfo> = Vec::new();
    let mut seg_dir_start = 0usize;
    let mut seg_rev_start = 0usize;
    let mut seg_log_start = global.access_log().expect("access log enabled").clock();
    let mut seg_bits = 0usize;
    let mut seg_gates = 0usize;
    let mut seg_regs: BTreeSet<u32> = BTreeSet::new();
    let mut seg_globals: BTreeSet<u32> = BTreeSet::new();
    let mut seg_dropped: Vec<(Reg, u32)> = Vec::new();
    let mut next_mark = limits.segment_cost.unwrap_or(usize::MAX);

    loop {
        let directive = match thread.step(module, global)? {
            StepResult::Continue => continue,
            StepResult::Directive(Directive::Call {
                dst,
                func_idx,
                args,
                ..
            }) if is_import(module, func_idx) => {
                if is_precompile(module, func_idx) {
                    let (action, value, visibility) =
                        host::service_sha256_compress(role, global, &args)?;
                    reveal_actions.push(action);
                    thread.resolve_host_call(value, visibility)?;
                    Directive::Call {
                        dst,
                        func_idx,
                        args,
                        param_base: Reg(0),
                    }
                } else {
                    let (action, value, visibility) = host::service_reveal(
                        role,
                        reveal_state,
                        &mut reveals,
                        module,
                        global,
                        func_idx,
                        dst,
                        &args,
                    )?;
                    reveal_actions.push(action);
                    thread.resolve_host_call(value, visibility)?;
                    if let Some(r) = reveal_written_reg(reveal_actions.last()) {
                        seg_regs.insert(r);
                    }
                    trace.push(Directive::Call {
                        dst,
                        func_idx,
                        args,
                        param_base: Reg(0),
                    });
                    continue;
                }
            }
            StepResult::Directive(d)
                if let Some((i, reason)) = &announced_trap
                    && thread.op_counter() == *i + 1 =>
            {
                validate_trap_directive(&d, reason)?;
                let log_end = global.access_log().expect("access log enabled").clock();
                let segments = finish_segments(
                    segments,
                    seg_dir_start,
                    seg_rev_start,
                    seg_log_start,
                    trace.len(),
                    reveal_actions.len(),
                    log_end,
                    seg_bits,
                    seg_gates,
                );
                return Ok(ChunkCapture {
                    trace,
                    cost,
                    done: true,
                    result: None,
                    result_symbolic: false,
                    trap: Some(TrapPoint {
                        index: *i,
                        directive: Some(d),
                        trap: reason.clone(),
                    }),
                    reveal_actions,
                    reveals,
                    segments,
                });
            }
            StepResult::Directive(d) => d,
            StepResult::Trapped {
                index,
                directive,
                trap,
            } => {
                if let Some((announced, _)) = &announced_trap
                    && *announced != index
                {
                    return Err(ZkVmError::Internal(format!(
                        "local trap at index {index} but prover announced {announced}"
                    )));
                }
                let log_end = global.access_log().expect("access log enabled").clock();
                let segments = finish_segments(
                    segments,
                    seg_dir_start,
                    seg_rev_start,
                    seg_log_start,
                    trace.len(),
                    reveal_actions.len(),
                    log_end,
                    seg_bits,
                    seg_gates,
                );
                return Ok(ChunkCapture {
                    trace,
                    cost,
                    done: true,
                    result: None,
                    result_symbolic: false,
                    trap: Some(TrapPoint {
                        index,
                        directive,
                        trap,
                    }),
                    reveal_actions,
                    reveals,
                    segments,
                });
            }
            StepResult::Blocked(pending) => match pending {
                Pending::Branch => {
                    return Err(ZkVmError::Unsupported(
                        "private branching not supported in zk-vm".into(),
                    ));
                }
                Pending::HostCall { .. } => {
                    return Err(ZkVmError::Internal(
                        "host call surfaced as blocked but should be serviced at its directive"
                            .into(),
                    ));
                }
                Pending::CallIndirect { .. } => {
                    return Err(ZkVmError::Unsupported(
                        "private indirect-call dispatch not supported in zk-vm".into(),
                    ));
                }
                Pending::MemoryGrow { .. } => {
                    return Err(ZkVmError::Unsupported(
                        "private memory.grow not supported in zk-vm".into(),
                    ));
                }
            },
            StepResult::Done { result, symbolic } => {
                let log_end = global.access_log().expect("access log enabled").clock();
                let segments = finish_segments(
                    segments,
                    seg_dir_start,
                    seg_rev_start,
                    seg_log_start,
                    trace.len(),
                    reveal_actions.len(),
                    log_end,
                    seg_bits,
                    seg_gates,
                );
                return Ok(ChunkCapture {
                    trace,
                    cost,
                    done: true,
                    result,
                    result_symbolic: symbolic,
                    trap: None,
                    reveal_actions,
                    reveals,
                    segments,
                });
            }
        };

        match &directive {
            Directive::Op(op) => {
                let c = cost::op_cost(op)?;
                cost += c;
                seg_bits += c;
                seg_gates += c - cost::op_advice_bits(op);
                match op {
                    Op::Copy { dst, .. }
                    | Op::GlobalGet { dst, .. }
                    | Op::Binary { dst, .. }
                    | Op::Unary { dst, .. }
                    | Op::Load { dst, .. } => {
                        seg_regs.insert(dst.0);
                    }
                    Op::GlobalSet { global_idx, .. } => {
                        seg_globals.insert(*global_idx);
                    }
                    _ => {}
                }
            }
            Directive::Call {
                func_idx,
                args,
                param_base,
                ..
            } => {
                if is_precompile(module, *func_idx) {
                    if let Some(action) = reveal_actions.last() {
                        if matches!(action, HostCallEvent::Sha256Compress { .. }) {
                            cost += cost::SHA256_COMPRESS_COST;
                            seg_bits += cost::SHA256_COMPRESS_COST;
                            seg_gates += cost::SHA256_COMPRESS_COST;
                        }
                    }
                } else if !is_import(module, *func_idx) {
                    for (k, arg) in args.iter().enumerate() {
                        if matches!(arg, Operand::Symbol { .. }) {
                            seg_regs.insert(param_base.0 + k as u32);
                        }
                    }
                }
            }
            Directive::Return { dst, src, reclaim } => {
                if let (Some(d), Some(_)) = (dst, src) {
                    seg_regs.insert(d.0);
                }
                if let Some((base, count)) = reclaim {
                    for r in base.0..base.0 + *count {
                        seg_regs.remove(&r);
                    }
                    seg_dropped.push((*base, *count));
                }
            }
            Directive::Branch {
                cond: Some(Operand::Symbol { .. }),
                ..
            } => {
                return Err(ZkVmError::Unsupported(
                    "private branching not supported in zk-vm".into(),
                ));
            }
            Directive::Branch { .. } => {}
        }

        trace.push(directive);

        if cost >= next_mark {
            let log_end = global.access_log().expect("access log enabled").clock();
            let written = memlog::written_view(
                global.access_log().expect("access log enabled"),
                seg_log_start..log_end,
            );
            let boundary = build_boundary(
                role,
                thread,
                global,
                std::mem::take(&mut seg_regs),
                std::mem::take(&mut seg_globals),
                std::mem::take(&mut seg_dropped),
                written,
            )?;
            segments.push(SegmentInfo {
                directives: seg_dir_start..trace.len(),
                reveals: seg_rev_start..reveal_actions.len(),
                log: seg_log_start..log_end,
                bits: seg_bits,
                gates: seg_gates,
                boundary: Some(boundary),
            });
            seg_dir_start = trace.len();
            seg_rev_start = reveal_actions.len();
            seg_log_start = log_end;
            seg_bits = 0;
            seg_gates = 0;
            next_mark = cost
                + limits
                    .segment_cost
                    .expect("next_mark finite only with segment cost");
        }

        if let Some(c) = limits.chunk_cap
            && cost >= c
        {
            let log_end = global.access_log().expect("access log enabled").clock();
            let segments = finish_segments(
                segments,
                seg_dir_start,
                seg_rev_start,
                seg_log_start,
                trace.len(),
                reveal_actions.len(),
                log_end,
                seg_bits,
                seg_gates,
            );
            return Ok(ChunkCapture {
                trace,
                cost,
                done: false,
                result: None,
                result_symbolic: false,
                trap: None,
                reveal_actions,
                reveals,
                segments,
            });
        }
    }
}

fn reveal_written_reg(action: Option<&HostCallEvent>) -> Option<u32> {
    match action? {
        HostCallEvent::OpenScalar { handle_dst, .. }
        | HostCallEvent::OpenBytes { handle_dst, .. } => Some(handle_dst.0),
        HostCallEvent::WaitScalar { dst, .. } => Some(dst.0),
        _ => None,
    }
}

fn build_boundary(
    role: Role,
    thread: &Thread,
    global: &Global,
    seg_regs: BTreeSet<u32>,
    seg_globals: BTreeSet<u32>,
    dropped: Vec<(Reg, u32)>,
    written: BTreeMap<u32, ByteState>,
) -> Result<BoundaryDelta> {
    let prover = role == Role::Prover;

    let vals =
        |keys: BTreeSet<u32>, symbolic: &dyn Fn(u32) -> bool, read: &dyn Fn(u32) -> Value| {
            keys.into_iter()
                .map(|key| {
                    let v = read(key);
                    if symbolic(key) {
                        ValItem::Sym {
                            key,
                            ty: v.ty(),
                            value: prover.then_some(v),
                        }
                    } else {
                        ValItem::Pub { key, value: v }
                    }
                })
                .collect()
        };

    let regs = vals(seg_regs, &|r| thread.is_register_symbolic(r), &|r| {
        thread.registers()[r as usize]
    });
    let globals = vals(seg_globals, &|g| global.is_global_symbolic(g), &|g| {
        global.globals()[g as usize]
    });

    let mut mem = Vec::new();
    for (a, state) in written {
        if global.memory_tainted(a, 1) {
            let value = if prover {
                let byte = global
                    .memory()
                    .ok_or(ZkVmError::Core(mpz_vm_core::Error::MemoryNotDefined))?
                    .read_bytes(a, 1)
                    .map_err(ZkVmError::Trap)?[0];
                Some(byte)
            } else {
                None
            };
            mem.push(MemItem::Sym { addr: a, value });
        } else if let ByteState::Public(value) = state {
            mem.push(MemItem::Pub { addr: a, value });
        }
    }

    Ok(BoundaryDelta {
        regs,
        dropped,
        globals,
        mem,
    })
}

pub(crate) fn run_local(
    module: &Module,
    global: &mut Global,
    thread: &mut Thread,
) -> Result<Option<Value>> {
    loop {
        match thread.step(module, global)? {
            StepResult::Continue => {}
            StepResult::Directive(Directive::Op(_)) => {
                return Err(ZkVmError::RequiresCommunication(
                    "symbolic operation requires a proving round".into(),
                ));
            }
            StepResult::Directive(Directive::Branch {
                cond: Some(Operand::Symbol { .. }),
                ..
            }) => {
                return Err(ZkVmError::RequiresCommunication(
                    "private branch requires a proving round".into(),
                ));
            }
            StepResult::Directive(_) => {}
            StepResult::Blocked(_) => {
                return Err(ZkVmError::RequiresCommunication(
                    "execution requires communication".into(),
                ));
            }
            StepResult::Trapped { trap, .. } => return Err(ZkVmError::Trap(trap)),
            StepResult::Done { result, .. } => return Ok(result),
        }
    }
}

fn validate_trap_directive(directive: &Directive, reason: &Trap) -> Result<()> {
    use mpz_vm_core::Op;
    use mpz_vm_ir::BinaryOp::*;
    let op = match directive {
        Directive::Op(Op::Binary { op, .. }) => *op,
        other => {
            return Err(ZkVmError::Internal(format!(
                "announced trap directive is not a binary op: {other:?}"
            )));
        }
    };
    let ok = match reason {
        Trap::DivideByZero => matches!(
            op,
            I32DivU | I32RemU | I32DivS | I32RemS | I64DivU | I64RemU | I64DivS | I64RemS
        ),
        Trap::IntegerOverflow => matches!(op, I32DivS | I64DivS),
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err(ZkVmError::Internal(format!(
            "announced trap reason {reason:?} not provable for op {op:?}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpz_vm_core::{Access, AccessAddr, AccessKind, Call, Op, Param, Visibility};
    use mpz_vm_ir::{ExportKind, Module, ValType};

    fn capture_full(
        module: &Module,
        func_idx: u32,
        role: Role,
        params: Vec<Param>,
        limits: Limits,
    ) -> (ChunkCapture, Global) {
        let mut global = Global::new(module).unwrap();
        global.enable_access_log();
        let mut thread = Thread::new();
        thread
            .call(module, &mut global, Call { func_idx, params })
            .unwrap();
        let mut reveal_state = RevealState::default();
        let capture = capture_chunk(
            module,
            &mut global,
            &mut thread,
            limits,
            role,
            None,
            &mut reveal_state,
        )
        .unwrap();
        (capture, global)
    }

    fn capture_trace(
        module: &Module,
        func_idx: u32,
        role: Role,
        params: Vec<Param>,
    ) -> Vec<Directive> {
        capture_full(module, func_idx, role, params, Limits::default())
            .0
            .trace
    }

    const SEGMENTED_WAT: &str = r#"(module
        (memory 1)
        (func (export "main") (param i32) (result i32)
          (local $acc i32) (local $tmp i32)
          local.get 0 local.get 0 i32.add local.set $acc
          i32.const 0 local.get $acc i32.store
          i32.const 64 i32.const 7 i32.store
          local.get $acc local.get 0 i32.add local.set $acc
          i32.const 4 local.get $acc i32.store
          i32.const 0 i32.load local.set $tmp
          local.get $acc local.get $tmp i32.add local.set $acc
          i32.const 8 local.get $acc i32.store
          local.get $acc local.get 0 i32.add local.set $acc
          i32.const 4 i32.load local.get $acc i32.add))"#;

    fn segmented_module() -> Module {
        Module::parse(&wat::parse_str(SEGMENTED_WAT).unwrap()).unwrap()
    }

    fn main_idx(module: &Module) -> u32 {
        module
            .exports()
            .iter()
            .find_map(|e| match e.kind {
                ExportKind::Func(i) if e.name == "main" => Some(i),
                _ => None,
            })
            .unwrap()
    }

    fn segmented_limits() -> Limits {
        Limits {
            chunk_cap: None,
            segment_cost: Some(1),
        }
    }

    fn skeleton(directive: &Directive) -> String {
        match directive {
            Directive::Op(Op::Copy { dst, src }) => format!("copy {dst} {src}"),
            Directive::Op(Op::Binary { dst, op, .. }) => format!("binary {dst} {op:?}"),
            Directive::Op(Op::GlobalGet { dst, global_idx }) => format!("gget {dst} {global_idx}"),
            Directive::Op(Op::GlobalSet { global_idx, .. }) => format!("gset {global_idx}"),
            Directive::Call {
                dst,
                func_idx,
                param_base,
                ..
            } => format!("call {dst:?} {func_idx} pb{param_base}"),
            Directive::Return { dst, src, reclaim } => format!("ret {dst:?} {src:?} {reclaim:?}"),
            other => format!("{other:?}"),
        }
    }

    #[test]
    fn prover_and_verifier_capture_identical_skeletons() {
        let wat = r#"(module
            (func $helper (param i32) (result i32)
                local.get 0 local.get 0 i32.add)
            (func $main (export "main") (param i32) (result i32)
                local.get 0 call $helper))"#;
        let module = Module::parse(&wat::parse_str(wat).unwrap()).unwrap();
        let idx = module
            .exports()
            .iter()
            .find_map(|e| match e.kind {
                ExportKind::Func(i) if e.name == "main" => Some(i),
                _ => None,
            })
            .unwrap();

        let prover = capture_trace(
            &module,
            idx,
            Role::Prover,
            vec![Param::Private(Value::I32(7))],
        );
        let verifier = capture_trace(
            &module,
            idx,
            Role::Verifier,
            vec![Param::Blind(ValType::I32)],
        );

        let ps: Vec<_> = prover.iter().map(skeleton).collect();
        let vs: Vec<_> = verifier.iter().map(skeleton).collect();
        assert_eq!(
            ps, vs,
            "prover and verifier must capture identical directive skeletons"
        );
        assert!(
            ps.iter().any(|k| k.starts_with("call")),
            "the test program must exercise a Call"
        );
    }

    #[test]
    fn segment_log_ranges_are_contiguous() {
        let module = segmented_module();
        let idx = main_idx(&module);
        let (chunk, global) = capture_full(
            &module,
            idx,
            Role::Prover,
            vec![Param::Private(Value::I32(7))],
            segmented_limits(),
        );

        assert!(
            chunk.segments.len() >= 2,
            "the program must split into at least two segments (got {})",
            chunk.segments.len()
        );

        let final_clock = global.access_log().expect("log enabled").clock();
        assert_eq!(
            chunk
                .segments
                .first()
                .expect("at least one segment")
                .log
                .start,
            0,
            "the first segment starts at the chunk-start clock"
        );
        for seg in &chunk.segments {
            assert!(
                seg.log.start <= seg.log.end,
                "segment log range must be well-formed: {:?}",
                seg.log
            );
        }
        for pair in chunk.segments.windows(2) {
            assert_eq!(
                pair[0].log.end, pair[1].log.start,
                "each segment's log range must start where the previous ended"
            );
        }
        assert_eq!(
            chunk.segments.last().expect("at least one segment").log.end,
            final_clock,
            "the last segment must end at the final clock"
        );
    }

    #[test]
    fn emitted_log_entries_zip_segment_memory_directives() {
        let module = segmented_module();
        let idx = main_idx(&module);
        let (chunk, global) = capture_full(
            &module,
            idx,
            Role::Prover,
            vec![Param::Private(Value::I32(7))],
            segmented_limits(),
        );
        let entries = global.access_log().expect("log enabled").entries();

        for seg in &chunk.segments {
            let emitted: Vec<AccessKind> = entries[seg.log.start as usize..seg.log.end as usize]
                .iter()
                .filter(|a| a.emitted)
                .map(|a| a.kind)
                .collect();
            let mem_dirs: Vec<&Directive> = chunk.trace[seg.directives.clone()]
                .iter()
                .filter(|d| {
                    matches!(
                        d,
                        Directive::Op(Op::Load { .. }) | Directive::Op(Op::Store { .. })
                    )
                })
                .collect();
            assert_eq!(
                emitted.len(),
                mem_dirs.len(),
                "segment (directives {:?}): emitted entries vs memory directives",
                seg.directives
            );
            for (kind, directive) in emitted.iter().zip(&mem_dirs) {
                match (kind, directive) {
                    (AccessKind::Read, Directive::Op(Op::Load { .. })) => {}
                    (AccessKind::Write, Directive::Op(Op::Store { .. })) => {}
                    _ => panic!("emitted kind {kind:?} does not match directive {directive:?}"),
                }
            }
        }
    }

    #[test]
    fn prover_and_verifier_segment_log_ranges_match() {
        let module = segmented_module();
        let idx = main_idx(&module);
        let (prover, _) = capture_full(
            &module,
            idx,
            Role::Prover,
            vec![Param::Private(Value::I32(7))],
            segmented_limits(),
        );
        let (verifier, _) = capture_full(
            &module,
            idx,
            Role::Verifier,
            vec![Param::Blind(ValType::I32)],
            segmented_limits(),
        );

        let ps: Vec<_> = prover.segments.iter().map(|s| s.log.clone()).collect();
        let vs: Vec<_> = verifier.segments.iter().map(|s| s.log.clone()).collect();
        assert_eq!(
            ps, vs,
            "prover and verifier must derive identical per-segment log ranges"
        );
    }

    const PRECOMPILE_WAT: &str = r#"(module
        (import "crypto" "sha256_compress" (func $compress (param i32 i32)))
        (memory 2)
        (func (export "main") (param $state i32) (param $block i32)
            (call $compress (local.get $state) (local.get $block))))"#;

    const STATE_PTR: u32 = 256;
    const BLOCK_PTR: u32 = 512;

    fn capture_precompile(role: Role, state_symbolic: bool) -> (ChunkCapture, Global) {
        let module = Module::parse(&wat::parse_str(PRECOMPILE_WAT).unwrap()).unwrap();
        let idx = main_idx(&module);
        let mut global = Global::new(&module).unwrap();
        global.enable_access_log();
        if state_symbolic {
            let vis = match role {
                Role::Prover => Visibility::Private,
                Role::Verifier => Visibility::Blind,
            };
            global.set_memory_visibility(STATE_PTR, 32, vis);
        }
        let mut thread = Thread::new();
        thread
            .call(
                &module,
                &mut global,
                Call {
                    func_idx: idx,
                    params: vec![
                        Param::Public(Value::I32(STATE_PTR as i32)),
                        Param::Public(Value::I32(BLOCK_PTR as i32)),
                    ],
                },
            )
            .unwrap();
        let mut reveal_state = RevealState::default();
        let capture = capture_chunk(
            &module,
            &mut global,
            &mut thread,
            Limits::default(),
            role,
            None,
            &mut reveal_state,
        )
        .unwrap();
        (capture, global)
    }

    fn assert_digest_entries(entries: &[Access], expected: [Option<u64>; 4], mask: u8) {
        assert_eq!(
            entries.len(),
            4,
            "one compress logs exactly four host writes"
        );
        for (i, (entry, value)) in entries.iter().zip(expected).enumerate() {
            assert_eq!(
                entry,
                &Access {
                    kind: AccessKind::Write,
                    addr: AccessAddr::Public(STATE_PTR + 8 * i as u32),
                    width: 8,
                    symbolic_mask: mask,
                    value,
                    emitted: false,
                    host: true,
                },
                "host-write entry {i} must cover its 8-byte digest chunk"
            );
        }
    }

    #[test]
    fn precompile_public_output_logs_host_writes() {
        let (chunk, global) = capture_precompile(Role::Prover, false);
        let log = global.access_log().expect("log enabled");
        let entries = log.entries();

        let digest = global
            .memory()
            .unwrap()
            .read_bytes(STATE_PTR, 32)
            .unwrap()
            .to_vec();
        let expected = core::array::from_fn(|i| {
            Some(u64::from_le_bytes(
                digest[8 * i..8 * i + 8].try_into().unwrap(),
            ))
        });
        assert_digest_entries(entries, expected, 0);

        let seg = chunk
            .segments
            .iter()
            .find(|s| s.log.start == 0)
            .expect("a segment starts at the chunk-start clock");
        assert!(
            seg.log.start == 0 && seg.log.end >= 4,
            "the covering segment must contain the four host writes: {:?}",
            seg.log
        );

        let (_, verifier) = capture_precompile(Role::Verifier, false);
        assert_eq!(
            entries,
            verifier.access_log().expect("log enabled").entries(),
            "prover and verifier must log identical host writes"
        );
    }

    #[test]
    fn precompile_authenticated_output_logs_host_writes() {
        let (_, prover) = capture_precompile(Role::Prover, true);
        let (_, verifier) = capture_precompile(Role::Verifier, true);

        let prover_entries = prover.access_log().expect("log enabled").entries();
        let verifier_entries = verifier.access_log().expect("log enabled").entries();

        assert_digest_entries(prover_entries, [None; 4], 0xff);
        assert_eq!(
            prover_entries, verifier_entries,
            "prover and verifier must log identical host writes"
        );
    }

    const SILENT_VS_SYMBOLIC_WAT: &str = r#"(module
        (memory 1)
        (func (export "main") (param i32) (result i32)
          ;; silent concrete store at address 0 (public value, no directive)
          i32.const 0 i32.const 42 i32.store
          ;; symbolic-value store at address 8 (private input, emitted)
          i32.const 8 local.get 0 i32.store
          ;; trailing symbolic arithmetic so the store's segment is not the last
          local.get 0 local.get 0 i32.add
          local.get 0 i32.add))"#;

    fn boundary_skeleton(delta: &BoundaryDelta) -> Vec<String> {
        let vals = |items: &[ValItem]| -> Vec<String> {
            items
                .iter()
                .map(|item| match item {
                    ValItem::Sym { key, ty, .. } => format!("sym {key} {ty:?}"),
                    ValItem::Pub { key, value } => format!("pub {key} {value:?}"),
                })
                .collect()
        };
        let mut out = vals(&delta.regs);
        out.extend(vals(&delta.globals));
        out.extend(delta.mem.iter().map(|item| match item {
            MemItem::Sym { addr, .. } => format!("msym {addr}"),
            MemItem::Pub { addr, value } => format!("mpub {addr} {value}"),
        }));
        out
    }

    #[test]
    fn silent_store_excluded_from_written_view() {
        let module = Module::parse(&wat::parse_str(SILENT_VS_SYMBOLIC_WAT).unwrap()).unwrap();
        let idx = main_idx(&module);
        let (prover, _) = capture_full(
            &module,
            idx,
            Role::Prover,
            vec![Param::Private(Value::I32(0x1122_3344))],
            segmented_limits(),
        );
        let (verifier, _) = capture_full(
            &module,
            idx,
            Role::Verifier,
            vec![Param::Blind(ValType::I32)],
            segmented_limits(),
        );

        let mem_boundaries: Vec<&BoundaryDelta> = prover
            .segments
            .iter()
            .filter_map(|s| s.boundary.as_ref())
            .filter(|b| !b.mem.is_empty())
            .collect();
        assert_eq!(
            mem_boundaries.len(),
            1,
            "only the symbolic store contributes memory to a boundary"
        );

        let mem = &mem_boundaries[0].mem;
        let addrs: Vec<u32> = mem
            .iter()
            .map(|item| match item {
                MemItem::Sym { addr, .. } | MemItem::Pub { addr, .. } => *addr,
            })
            .collect();
        assert_eq!(addrs, vec![8, 9, 10, 11], "the symbolic store covers 8..12");
        assert!(
            mem.iter().all(|item| matches!(item, MemItem::Sym { .. })),
            "the symbolic store's bytes are committed"
        );
        assert!(
            !addrs.iter().any(|&a| a < 8),
            "the silent public store's bytes must not enter the written view"
        );

        let ps: Vec<_> = prover
            .segments
            .iter()
            .filter_map(|s| s.boundary.as_ref())
            .map(boundary_skeleton)
            .collect();
        let vs: Vec<_> = verifier
            .segments
            .iter()
            .filter_map(|s| s.boundary.as_ref())
            .map(boundary_skeleton)
            .collect();
        assert_eq!(
            ps, vs,
            "prover and verifier must commit identical boundary skeletons"
        );
    }
}
