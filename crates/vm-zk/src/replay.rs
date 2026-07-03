use itybity::{GetBit, Lsb0};
use mpz_circuits::Context;
use mpz_fields::{gf2::Gf2, gf2_128::Gf2_128};
use mpz_vm_core::{Directive, Op, Operand, Reg, Trap, ValType, value::Value};
use mpz_vm_ir::{BinaryOp, LoadKind, MemArg, Module, StoreKind, UnaryOp};
use rand_chacha::ChaCha12Rng;

use mpz_vm_memory::{AuthState, AuthValue, Bit, Byte, I32, I64};

use mpz_vm_circuits as circ;

use crate::{
    capture::is_import,
    error::{Result, ZkVmError, unsupported_binary, unsupported_op, unsupported_unary},
    finalize,
    host::HostCallEvent,
};

pub(crate) type ProverCtx<'a> = mpz_zk_core::prover::Accumulate<'a, ChaCha12Rng>;

pub(crate) type VerifierCtx<'a> = mpz_zk_core::verifier::Verifier<'a, ChaCha12Rng>;

pub(crate) trait ZkExec: Context<Field = Gf2, Wire: GetBit<Lsb0>> {
    fn public_bit(&mut self, value: bool) -> Self::Wire;

    fn advise_i32(&mut self, compute: impl FnOnce() -> u32) -> I32<Self::Wire>;

    fn advise_i64(&mut self, compute: impl FnOnce() -> u64) -> I64<Self::Wire>;
}

impl ZkExec for mpz_zk_core::Witness<'_> {
    fn public_bit(&mut self, value: bool) -> Gf2 {
        self.input_public(Gf2(value))
    }

    fn advise_i32(&mut self, compute: impl FnOnce() -> u32) -> I32<Gf2> {
        let v = compute();
        I32::from(core::array::from_fn(|i| self.input(Gf2((v >> i) & 1 != 0))))
    }

    fn advise_i64(&mut self, compute: impl FnOnce() -> u64) -> I64<Gf2> {
        let v = compute();
        I64::from(core::array::from_fn(|i| self.input(Gf2((v >> i) & 1 != 0))))
    }
}

impl ZkExec for ProverCtx<'_> {
    fn public_bit(&mut self, value: bool) -> Gf2_128 {
        self.input_public(value)
    }

    fn advise_i32(&mut self, compute: impl FnOnce() -> u32) -> I32<Gf2_128> {
        let v = compute();
        I32::from(core::array::from_fn(|i| self.input((v >> i) & 1 != 0)))
    }

    fn advise_i64(&mut self, compute: impl FnOnce() -> u64) -> I64<Gf2_128> {
        let v = compute();
        I64::from(core::array::from_fn(|i| self.input((v >> i) & 1 != 0)))
    }
}

impl ZkExec for VerifierCtx<'_> {
    fn public_bit(&mut self, value: bool) -> Gf2_128 {
        self.input_public(value)
    }

    fn advise_i32(&mut self, _compute: impl FnOnce() -> u32) -> I32<Gf2_128> {
        I32::from(core::array::from_fn(|_| self.input()))
    }

    fn advise_i64(&mut self, _compute: impl FnOnce() -> u64) -> I64<Gf2_128> {
        I64::from(core::array::from_fn(|_| self.input()))
    }
}

#[derive(Debug)]
pub(crate) struct ReplayState {
    pub(crate) output_reg: Option<Reg>,
}

impl ReplayState {
    pub(crate) fn root() -> Self {
        Self { output_reg: None }
    }
}

#[tracing::instrument(level = "trace", skip_all, fields(events = trace.len()))]
pub(crate) fn replay<C>(
    trace: &[Directive],
    reveal_actions: &[HostCallEvent],
    module: &Module,
    auth: &mut AuthState<C::Wire>,
    exec: &mut C,
    state: &mut ReplayState,
) -> Result<()>
where
    C: ZkExec,
    C::Error: core::fmt::Debug,
{
    let mut reveal_cursor = 0;
    for directive in trace.iter() {
        match directive {
            Directive::Op(op) => match op {
                Op::Copy { dst, src } => {
                    auth.regs.copy(*dst, *src);
                }
                Op::GlobalGet { dst, global_idx } => {
                    let av = auth
                        .globals
                        .get(Reg(*global_idx))
                        .cloned()
                        .ok_or(ZkVmError::GlobalAuthMissing { idx: *global_idx })?;
                    auth.regs.set(*dst, av);
                }
                Op::GlobalSet { global_idx, src } => {
                    let av = operand_value(src, auth, exec)?;
                    auth.globals.set(Reg(*global_idx), av);
                }
                Op::Binary {
                    dst,
                    op: bop,
                    lhs,
                    rhs,
                } => {
                    let av = binary_eval(exec, auth, *bop, lhs, rhs)?;
                    auth.regs.set(*dst, av);
                }
                Op::Unary { dst, op: uop, src } => {
                    let a = auth
                        .regs
                        .get(*src)
                        .cloned()
                        .ok_or(ZkVmError::RegAuthMissing { reg: *src })?;
                    let av = unary_eval(*uop, exec, a)?;
                    auth.regs.set(*dst, av);
                }
                Op::Load {
                    kind,
                    dst,
                    addr,
                    memarg,
                    concrete,
                    symbolic_mask,
                } => {
                    if kind.is_float() {
                        return Err(unsupported_op(op));
                    }
                    mem_load(auth, *dst, *kind, addr, memarg, *concrete, *symbolic_mask)?;
                }
                Op::Store {
                    kind,
                    addr,
                    val,
                    memarg,
                } => {
                    if kind.is_float() {
                        return Err(unsupported_op(op));
                    }
                    mem_store(exec, auth, *kind, addr, val, memarg)?;
                }
                _ => return Err(unsupported_op(op)),
            },
            Directive::Call {
                func_idx,
                args,
                param_base,
                ..
            } => {
                if is_import(module, *func_idx) {
                    let action = reveal_actions.get(reveal_cursor).ok_or_else(|| {
                        ZkVmError::Internal("reveal action missing for imported call".into())
                    })?;
                    reveal_cursor += 1;
                    match action {
                        HostCallEvent::Sha256Compress {
                            state_ptr,
                            block_ptr,
                            state_pub,
                            state_sym,
                            block_pub,
                            block_sym,
                        } => apply_sha256_compress(
                            exec, auth, *state_ptr, state_pub, *state_sym, *block_ptr, block_pub,
                            *block_sym,
                        )?,
                        HostCallEvent::Sha256CompressPublic { state_ptr, digest } => {
                            write_public_bytes(exec, auth, *state_ptr, digest)
                        }
                        _ => apply_reveal(action, auth, exec)?,
                    }
                } else {
                    propagate_args(auth, *param_base, args);
                }
            }
            Directive::Return { dst, src, reclaim } => {
                handle_return(auth, state, *dst, *src, *reclaim);
            }
            Directive::Branch { .. } => {}
        }
    }
    Ok(())
}

fn operand_value<C>(
    operand: &Operand,
    auth: &AuthState<C::Wire>,
    exec: &mut C,
) -> Result<AuthValue<C::Wire>>
where
    C: ZkExec,
{
    match operand {
        Operand::Symbol { reg, .. } => auth
            .regs
            .get(*reg)
            .cloned()
            .ok_or(ZkVmError::RegAuthMissing { reg: *reg }),
        Operand::Concrete(v) => const_auth(exec, v),
    }
}

fn const_auth<C>(exec: &mut C, v: &Value) -> Result<AuthValue<C::Wire>>
where
    C: ZkExec,
{
    let (ty, width, raw) = decode_concrete(v)?;
    let bits: Vec<Bit<C::Wire>> = (0..width)
        .map(|i| Bit(exec.public_bit((raw >> i) & 1 != 0)))
        .collect();
    Ok(AuthValue::from_bits(ty, &bits)?)
}

fn decode_concrete(v: &Value) -> Result<(ValType, usize, u64)> {
    match v {
        Value::I32(x) => Ok((ValType::I32, 32, *x as u32 as u64)),
        Value::I64(x) => Ok((ValType::I64, 64, *x as u64)),
        _ => Err(ZkVmError::Unsupported(
            "float IT-MAC not supported in zk-vm".into(),
        )),
    }
}

fn const_i32(op: &Operand) -> Result<i32> {
    match op {
        Operand::Concrete(Value::I32(x)) => Ok(*x),
        other => Err(ZkVmError::Internal(format!(
            "expected i32 constant operand, got {other:?}"
        ))),
    }
}

fn const_i64(op: &Operand) -> Result<i64> {
    match op {
        Operand::Concrete(Value::I64(x)) => Ok(*x),
        other => Err(ZkVmError::Internal(format!(
            "expected i64 constant operand, got {other:?}"
        ))),
    }
}

fn wire_value<W: GetBit<Lsb0>>(wires: &[W]) -> u64 {
    let mut out = 0u64;
    for (i, w) in wires.iter().enumerate() {
        if GetBit::<Lsb0>::get_bit(w, 0) {
            out |= 1 << i;
        }
    }
    out
}

fn binary_eval<C>(
    exec: &mut C,
    auth: &AuthState<C::Wire>,
    op: BinaryOp,
    lhs: &Operand,
    rhs: &Operand,
) -> Result<AuthValue<C::Wire>>
where
    C: ZkExec,
    C::Error: core::fmt::Debug,
{
    use BinaryOp::*;
    let a = operand_value(lhs, auth, exec)?;
    let b = operand_value(rhs, auth, exec)?;
    Ok(match op {
        I32Eq => circ::I32Eq::eval(exec, a.try_into_i32()?, b.try_into_i32()?).into(),
        I32Ne => circ::I32Ne::eval(exec, a.try_into_i32()?, b.try_into_i32()?).into(),
        I32LtS => circ::I32LtS::eval(exec, a.try_into_i32()?, b.try_into_i32()?).into(),
        I32LtU => circ::I32LtU::eval(exec, a.try_into_i32()?, b.try_into_i32()?).into(),
        I32GtS => circ::I32GtS::eval(exec, a.try_into_i32()?, b.try_into_i32()?).into(),
        I32GtU => circ::I32GtU::eval(exec, a.try_into_i32()?, b.try_into_i32()?).into(),
        I32LeS => circ::I32LeS::eval(exec, a.try_into_i32()?, b.try_into_i32()?).into(),
        I32LeU => circ::I32LeU::eval(exec, a.try_into_i32()?, b.try_into_i32()?).into(),
        I32GeS => circ::I32GeS::eval(exec, a.try_into_i32()?, b.try_into_i32()?).into(),
        I32GeU => circ::I32GeU::eval(exec, a.try_into_i32()?, b.try_into_i32()?).into(),
        I64Eq => circ::I64Eq::eval(exec, a.try_into_i64()?, b.try_into_i64()?).into(),
        I64Ne => circ::I64Ne::eval(exec, a.try_into_i64()?, b.try_into_i64()?).into(),
        I64LtS => circ::I64LtS::eval(exec, a.try_into_i64()?, b.try_into_i64()?).into(),
        I64LtU => circ::I64LtU::eval(exec, a.try_into_i64()?, b.try_into_i64()?).into(),
        I64GtS => circ::I64GtS::eval(exec, a.try_into_i64()?, b.try_into_i64()?).into(),
        I64GtU => circ::I64GtU::eval(exec, a.try_into_i64()?, b.try_into_i64()?).into(),
        I64LeS => circ::I64LeS::eval(exec, a.try_into_i64()?, b.try_into_i64()?).into(),
        I64LeU => circ::I64LeU::eval(exec, a.try_into_i64()?, b.try_into_i64()?).into(),
        I64GeS => circ::I64GeS::eval(exec, a.try_into_i64()?, b.try_into_i64()?).into(),
        I64GeU => circ::I64GeU::eval(exec, a.try_into_i64()?, b.try_into_i64()?).into(),
        I32Add => circ::I32Add::eval(exec, a.try_into_i32()?, b.try_into_i32()?).into(),
        I32Sub => circ::I32Sub::eval(exec, a.try_into_i32()?, b.try_into_i32()?).into(),
        I32Mul if rhs.is_concrete() => {
            circ::I32Mul::eval_const(exec, a.try_into_i32()?, const_i32(rhs)?).into()
        }
        I32Mul => circ::I32Mul::eval(exec, a.try_into_i32()?, b.try_into_i32()?).into(),
        I32And if rhs.is_concrete() => {
            circ::I32And::eval_const(exec, a.try_into_i32()?, const_i32(rhs)?).into()
        }
        I32And => circ::I32And::eval(exec, a.try_into_i32()?, b.try_into_i32()?).into(),
        I32Or if rhs.is_concrete() => {
            circ::I32Or::eval_const(exec, a.try_into_i32()?, const_i32(rhs)?).into()
        }
        I32Or => circ::I32Or::eval(exec, a.try_into_i32()?, b.try_into_i32()?).into(),
        I32Xor => circ::I32Xor::eval(exec, a.try_into_i32()?, b.try_into_i32()?).into(),
        I32Shl if rhs.is_concrete() => {
            circ::I32Shl::eval_const_amount(exec, a.try_into_i32()?, const_i32(rhs)?).into()
        }
        I32Shl => circ::I32Shl::eval(exec, a.try_into_i32()?, b.try_into_i32()?).into(),
        I32ShrS if rhs.is_concrete() => {
            circ::I32ShrS::eval_const_amount(a.try_into_i32()?, const_i32(rhs)?).into()
        }
        I32ShrS => circ::I32ShrS::eval(exec, a.try_into_i32()?, b.try_into_i32()?).into(),
        I32ShrU if rhs.is_concrete() => {
            circ::I32ShrU::eval_const_amount(exec, a.try_into_i32()?, const_i32(rhs)?).into()
        }
        I32ShrU => circ::I32ShrU::eval(exec, a.try_into_i32()?, b.try_into_i32()?).into(),
        I32Rotl if rhs.is_concrete() => {
            circ::I32Rotl::eval_const_amount(a.try_into_i32()?, const_i32(rhs)?).into()
        }
        I32Rotl => circ::I32Rotl::eval(exec, a.try_into_i32()?, b.try_into_i32()?).into(),
        I32Rotr if rhs.is_concrete() => {
            circ::I32Rotr::eval_const_amount(a.try_into_i32()?, const_i32(rhs)?).into()
        }
        I32Rotr => circ::I32Rotr::eval(exec, a.try_into_i32()?, b.try_into_i32()?).into(),
        I32DivU if rhs.is_concrete() => {
            circ::I32DivU::eval_const_divisor(exec, a.try_into_i32()?, const_i32(rhs)?).into()
        }
        I32DivU => div_rem_i32(exec, a.try_into_i32()?, b.try_into_i32()?, I32DivU)?,
        I32RemU if rhs.is_concrete() => {
            circ::I32RemU::eval_const_divisor(exec, a.try_into_i32()?, const_i32(rhs)?).into()
        }
        I32RemU => div_rem_i32(exec, a.try_into_i32()?, b.try_into_i32()?, I32RemU)?,
        I32DivS if rhs.is_concrete() => {
            circ::I32DivS::eval_const_divisor(exec, a.try_into_i32()?, const_i32(rhs)?).into()
        }
        I32DivS => div_rem_i32(exec, a.try_into_i32()?, b.try_into_i32()?, I32DivS)?,
        I32RemS if rhs.is_concrete() => {
            circ::I32RemS::eval_const_divisor(exec, a.try_into_i32()?, const_i32(rhs)?).into()
        }
        I32RemS => div_rem_i32(exec, a.try_into_i32()?, b.try_into_i32()?, I32RemS)?,
        I64Add => circ::I64Add::eval(exec, a.try_into_i64()?, b.try_into_i64()?).into(),
        I64Sub => circ::I64Sub::eval(exec, a.try_into_i64()?, b.try_into_i64()?).into(),
        I64Mul if rhs.is_concrete() => {
            circ::I64Mul::eval_const(exec, a.try_into_i64()?, const_i64(rhs)?).into()
        }
        I64Mul => circ::I64Mul::eval(exec, a.try_into_i64()?, b.try_into_i64()?).into(),
        I64And if rhs.is_concrete() => {
            circ::I64And::eval_const(exec, a.try_into_i64()?, const_i64(rhs)?).into()
        }
        I64And => circ::I64And::eval(exec, a.try_into_i64()?, b.try_into_i64()?).into(),
        I64Or if rhs.is_concrete() => {
            circ::I64Or::eval_const(exec, a.try_into_i64()?, const_i64(rhs)?).into()
        }
        I64Or => circ::I64Or::eval(exec, a.try_into_i64()?, b.try_into_i64()?).into(),
        I64Xor => circ::I64Xor::eval(exec, a.try_into_i64()?, b.try_into_i64()?).into(),
        I64Shl if rhs.is_concrete() => {
            circ::I64Shl::eval_const_amount(exec, a.try_into_i64()?, const_i64(rhs)?).into()
        }
        I64Shl => circ::I64Shl::eval(exec, a.try_into_i64()?, b.try_into_i64()?).into(),
        I64ShrS if rhs.is_concrete() => {
            circ::I64ShrS::eval_const_amount(a.try_into_i64()?, const_i64(rhs)?).into()
        }
        I64ShrS => circ::I64ShrS::eval(exec, a.try_into_i64()?, b.try_into_i64()?).into(),
        I64ShrU if rhs.is_concrete() => {
            circ::I64ShrU::eval_const_amount(exec, a.try_into_i64()?, const_i64(rhs)?).into()
        }
        I64ShrU => circ::I64ShrU::eval(exec, a.try_into_i64()?, b.try_into_i64()?).into(),
        I64Rotl if rhs.is_concrete() => {
            circ::I64Rotl::eval_const_amount(a.try_into_i64()?, const_i64(rhs)?).into()
        }
        I64Rotl => circ::I64Rotl::eval(exec, a.try_into_i64()?, b.try_into_i64()?).into(),
        I64Rotr if rhs.is_concrete() => {
            circ::I64Rotr::eval_const_amount(a.try_into_i64()?, const_i64(rhs)?).into()
        }
        I64Rotr => circ::I64Rotr::eval(exec, a.try_into_i64()?, b.try_into_i64()?).into(),
        I64DivU if rhs.is_concrete() => {
            circ::I64DivU::eval_const_divisor(exec, a.try_into_i64()?, const_i64(rhs)?).into()
        }
        I64DivU => div_rem_i64(exec, a.try_into_i64()?, b.try_into_i64()?, I64DivU)?,
        I64RemU if rhs.is_concrete() => {
            circ::I64RemU::eval_const_divisor(exec, a.try_into_i64()?, const_i64(rhs)?).into()
        }
        I64RemU => div_rem_i64(exec, a.try_into_i64()?, b.try_into_i64()?, I64RemU)?,
        I64DivS if rhs.is_concrete() => {
            circ::I64DivS::eval_const_divisor(exec, a.try_into_i64()?, const_i64(rhs)?).into()
        }
        I64DivS => div_rem_i64(exec, a.try_into_i64()?, b.try_into_i64()?, I64DivS)?,
        I64RemS if rhs.is_concrete() => {
            circ::I64RemS::eval_const_divisor(exec, a.try_into_i64()?, const_i64(rhs)?).into()
        }
        I64RemS => div_rem_i64(exec, a.try_into_i64()?, b.try_into_i64()?, I64RemS)?,
        _ => return unsupported_binary(op),
    })
}

fn div_rem_i32<C>(
    exec: &mut C,
    a: I32<C::Wire>,
    b: I32<C::Wire>,
    op: BinaryOp,
) -> Result<AuthValue<C::Wire>>
where
    C: ZkExec,
    C::Error: core::fmt::Debug,
{
    use BinaryOp::*;
    let dividend = wire_value(&a.to_wires()) as u32;
    let divisor = wire_value(&b.to_wires()) as u32;
    let signed = matches!(op, I32DivS | I32RemS);
    let advice = || {
        if signed {
            let (q, r) = circ::I32DivS::advice_values(dividend as i32, divisor as i32);
            (q as u32, r as u32)
        } else {
            circ::I32DivU::advice_values(dividend, divisor)
        }
    };
    let q = exec.advise_i32(|| advice().0);
    let r = exec.advise_i32(|| advice().1);
    let out = match op {
        I32DivU => circ::I32DivU::eval_with_advice(exec, a, b, q, r),
        I32RemU => circ::I32RemU::eval_with_advice(exec, a, b, q, r),
        I32DivS => circ::I32DivS::eval_with_advice(exec, a, b, q, r),
        I32RemS => circ::I32RemS::eval_with_advice(exec, a, b, q, r),
        other => return Err(ZkVmError::Internal(format!("div_rem_i32 on {other:?}"))),
    }
    .map_err(|e| ZkVmError::Internal(format!("div/rem assert: {e:?}")))?;
    Ok(out.into())
}

fn div_rem_i64<C>(
    exec: &mut C,
    a: I64<C::Wire>,
    b: I64<C::Wire>,
    op: BinaryOp,
) -> Result<AuthValue<C::Wire>>
where
    C: ZkExec,
    C::Error: core::fmt::Debug,
{
    use BinaryOp::*;
    let dividend = wire_value(&a.to_wires());
    let divisor = wire_value(&b.to_wires());
    let signed = matches!(op, I64DivS | I64RemS);
    let advice = || {
        if signed {
            let (q, r) = circ::I64DivS::advice_values(dividend as i64, divisor as i64);
            (q as u64, r as u64)
        } else {
            circ::I64DivU::advice_values(dividend, divisor)
        }
    };
    let q = exec.advise_i64(|| advice().0);
    let r = exec.advise_i64(|| advice().1);
    let out = match op {
        I64DivU => circ::I64DivU::eval_with_advice(exec, a, b, q, r),
        I64RemU => circ::I64RemU::eval_with_advice(exec, a, b, q, r),
        I64DivS => circ::I64DivS::eval_with_advice(exec, a, b, q, r),
        I64RemS => circ::I64RemS::eval_with_advice(exec, a, b, q, r),
        other => return Err(ZkVmError::Internal(format!("div_rem_i64 on {other:?}"))),
    }
    .map_err(|e| ZkVmError::Internal(format!("div/rem assert: {e:?}")))?;
    Ok(out.into())
}

fn count_i32<C>(exec: &mut C, a: I32<C::Wire>, clz: bool) -> Result<AuthValue<C::Wire>>
where
    C: ZkExec,
    C::Error: core::fmt::Debug,
{
    let val = wire_value(&a.to_wires()) as u32;
    let advice = exec.advise_i32(|| {
        if clz {
            circ::I32Clz::advice_values(val)
        } else {
            circ::I32Ctz::advice_values(val)
        }
    });
    let out = if clz {
        circ::I32Clz::eval_with_advice(exec, a, advice)
    } else {
        circ::I32Ctz::eval_with_advice(exec, a, advice)
    }
    .map_err(|e| ZkVmError::Internal(format!("count assert: {e:?}")))?;
    Ok(out.into())
}

fn count_i64<C>(exec: &mut C, a: I64<C::Wire>, clz: bool) -> Result<AuthValue<C::Wire>>
where
    C: ZkExec,
    C::Error: core::fmt::Debug,
{
    let val = wire_value(&a.to_wires());
    let advice = exec.advise_i64(|| {
        if clz {
            circ::I64Clz::advice_values(val)
        } else {
            circ::I64Ctz::advice_values(val)
        }
    });
    let out = if clz {
        circ::I64Clz::eval_with_advice(exec, a, advice)
    } else {
        circ::I64Ctz::eval_with_advice(exec, a, advice)
    }
    .map_err(|e| ZkVmError::Internal(format!("count assert: {e:?}")))?;
    Ok(out.into())
}

fn unary_eval<C>(op: UnaryOp, exec: &mut C, a: AuthValue<C::Wire>) -> Result<AuthValue<C::Wire>>
where
    C: ZkExec,
    C::Error: core::fmt::Debug,
{
    use UnaryOp::*;
    Ok(match op {
        I32Eqz => circ::I32Eqz::eval(exec, a.try_into_i32()?).into(),
        I64Eqz => circ::I64Eqz::eval(exec, a.try_into_i64()?).into(),
        I32Clz => count_i32(exec, a.try_into_i32()?, true)?,
        I32Ctz => count_i32(exec, a.try_into_i32()?, false)?,
        I32Popcnt => circ::I32Popcnt::eval(exec, a.try_into_i32()?).into(),
        I64Clz => count_i64(exec, a.try_into_i64()?, true)?,
        I64Ctz => count_i64(exec, a.try_into_i64()?, false)?,
        I64Popcnt => circ::I64Popcnt::eval(exec, a.try_into_i64()?).into(),
        I32WrapI64 => circ::I32WrapI64::eval(exec, a.try_into_i64()?).into(),
        I64ExtendI32S => circ::I64ExtendI32S::eval(exec, a.try_into_i32()?).into(),
        I64ExtendI32U => circ::I64ExtendI32U::eval(exec, a.try_into_i32()?).into(),
        I32Extend8S => circ::I32Extend8S::eval(exec, a.try_into_i32()?).into(),
        I32Extend16S => circ::I32Extend16S::eval(exec, a.try_into_i32()?).into(),
        I64Extend8S => circ::I64Extend8S::eval(exec, a.try_into_i64()?).into(),
        I64Extend16S => circ::I64Extend16S::eval(exec, a.try_into_i64()?).into(),
        I64Extend32S => circ::I64Extend32S::eval(exec, a.try_into_i64()?).into(),
        _ => return unsupported_unary(op),
    })
}

fn mem_load<W: Copy>(
    auth: &mut AuthState<W>,
    dst: Reg,
    kind: LoadKind,
    addr: &Operand,
    memarg: &MemArg,
    concrete: u64,
    symbolic_mask: u8,
) -> Result<()> {
    use LoadKind::*;
    let eff = crate::memlog::eff_addr(addr, memarg)?;
    let m = &auth.memory;
    let av: AuthValue<W> = match kind {
        I32 => m
            .load_i32_mixed(eff, concrete, symbolic_mask)
            .map(Into::into),
        I64 => m
            .load_i64_mixed(eff, concrete, symbolic_mask)
            .map(Into::into),
        I32Load8U => m
            .load_i32_8u_mixed(eff, concrete, symbolic_mask)
            .map(Into::into),
        I32Load8S => m
            .load_i32_8s_mixed(eff, concrete, symbolic_mask)
            .map(Into::into),
        I32Load16U => m
            .load_i32_16u_mixed(eff, concrete, symbolic_mask)
            .map(Into::into),
        I32Load16S => m
            .load_i32_16s_mixed(eff, concrete, symbolic_mask)
            .map(Into::into),
        I64Load8U => m
            .load_i64_8u_mixed(eff, concrete, symbolic_mask)
            .map(Into::into),
        I64Load8S => m
            .load_i64_8s_mixed(eff, concrete, symbolic_mask)
            .map(Into::into),
        I64Load16U => m
            .load_i64_16u_mixed(eff, concrete, symbolic_mask)
            .map(Into::into),
        I64Load16S => m
            .load_i64_16s_mixed(eff, concrete, symbolic_mask)
            .map(Into::into),
        I64Load32U => m
            .load_i64_32u_mixed(eff, concrete, symbolic_mask)
            .map(Into::into),
        I64Load32S => m
            .load_i64_32s_mixed(eff, concrete, symbolic_mask)
            .map(Into::into),
        F32 | F64 => {
            return Err(ZkVmError::Internal(
                "zk-vm: float load not supported".into(),
            ));
        }
    }
    .ok_or(ZkVmError::MemAuthMissing { addr: eff })?;
    auth.regs.set(dst, av);
    Ok(())
}

fn mem_store<C>(
    exec: &mut C,
    auth: &mut AuthState<C::Wire>,
    kind: StoreKind,
    addr: &Operand,
    val: &Operand,
    memarg: &MemArg,
) -> Result<()>
where
    C: ZkExec,
{
    use StoreKind::*;
    let eff = crate::memlog::eff_addr(addr, memarg)?;
    let av = operand_value(val, auth, exec)?;
    match kind {
        I32 => auth.memory.store_i32(eff, av.try_as_i32()?),
        I64 => auth.memory.store_i64(eff, av.try_as_i64()?),
        I32Store8 => auth.memory.store_i32_8(eff, av.try_as_i32()?),
        I32Store16 => auth.memory.store_i32_16(eff, av.try_as_i32()?),
        I64Store8 => auth.memory.store_i64_8(eff, av.try_as_i64()?),
        I64Store16 => auth.memory.store_i64_16(eff, av.try_as_i64()?),
        I64Store32 => auth.memory.store_i64_32(eff, av.try_as_i64()?),
        F32 | F64 => {
            return Err(ZkVmError::Internal(
                "zk-vm: float store not supported".into(),
            ));
        }
    }
    Ok(())
}

fn apply_reveal<C>(event: &HostCallEvent, auth: &mut AuthState<C::Wire>, exec: &mut C) -> Result<()>
where
    C: ZkExec,
    C::Error: core::fmt::Debug,
{
    match event {
        HostCallEvent::OpenScalar {
            src,
            value,
            handle_dst,
            id,
        } => {
            if let Some(src) = src {
                finalize::assert_output(exec, auth, *src, *value)?;
            }
            set_public_reg(auth, exec, *handle_dst, &Value::I32(*id as i32))?;
        }
        HostCallEvent::WaitScalar { dst, value } => {
            set_public_reg(auth, exec, *dst, value)?;
        }
        HostCallEvent::OpenBytes {
            ptr,
            bytes,
            handle_dst,
            id,
        } => {
            crate::reveal::assert_bytes(exec, auth, *ptr, bytes)?;
            set_public_reg(auth, exec, *handle_dst, &Value::I32(*id as i32))?;
        }
        HostCallEvent::WaitBytes => {}
        HostCallEvent::Sha256Compress { .. } | HostCallEvent::Sha256CompressPublic { .. } => {
            return Err(ZkVmError::Internal(
                "precompile action routed to apply_reveal".into(),
            ));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn apply_sha256_compress<C>(
    exec: &mut C,
    auth: &mut AuthState<C::Wire>,
    state_ptr: u32,
    state_pub: &[u8; 32],
    state_sym: u64,
    block_ptr: u32,
    block_pub: &[u8; 64],
    block_sym: u64,
) -> Result<()>
where
    C: ZkExec,
    C::Error: core::fmt::Debug,
{
    let state = read_mixed::<256, _>(exec, auth, state_ptr, state_pub, state_sym)?;
    let msg = read_mixed::<512, _>(exec, auth, block_ptr, block_pub, block_sym)?;
    let out = mpz_circuits::sha256::compress(exec, msg, state);
    write_words(auth, state_ptr, &out);
    Ok(())
}

fn write_public_bytes<C>(exec: &mut C, auth: &mut AuthState<C::Wire>, base: u32, digest: &[u8; 32])
where
    C: ZkExec,
{
    for (i, b) in digest.iter().enumerate() {
        let byte = Byte::new(core::array::from_fn(|k| {
            Bit(exec.public_bit((b >> k) & 1 != 0))
        }));
        auth.memory.set_byte(base + i as u32, byte);
    }
}

fn read_mixed<const N: usize, C>(
    exec: &mut C,
    auth: &AuthState<C::Wire>,
    base: u32,
    public: &[u8],
    sym: u64,
) -> Result<[C::Wire; N]>
where
    C: ZkExec,
{
    let mut out = [exec.public_bit(false); N];
    for bo in 0..N / 8 {
        let bits: [C::Wire; 8] = if (sym >> bo) & 1 != 0 {
            let off = base + bo as u32;
            let byte = auth
                .memory
                .get_byte(off)
                .ok_or(ZkVmError::MemAuthMissing { addr: off })?;
            core::array::from_fn(|k| byte.bits()[k].0)
        } else {
            core::array::from_fn(|k| exec.public_bit((public[bo] >> k) & 1 != 0))
        };
        out[bo * 8..bo * 8 + 8].copy_from_slice(&bits);
    }
    Ok(out)
}

fn write_words<W: Copy>(auth: &mut AuthState<W>, base: u32, wires: &[W]) {
    for bo in 0..wires.len() / 8 {
        let byte = Byte::new(core::array::from_fn(|k| Bit(wires[bo * 8 + k])));
        auth.memory.set_byte(base + bo as u32, byte);
    }
}

fn set_public_reg<C>(
    auth: &mut AuthState<C::Wire>,
    exec: &mut C,
    reg: Reg,
    value: &Value,
) -> Result<()>
where
    C: ZkExec,
{
    auth.regs.set(reg, const_auth(exec, value)?);
    Ok(())
}

fn handle_return<W: Copy>(
    auth: &mut AuthState<W>,
    state: &mut ReplayState,
    dst: Option<Reg>,
    src: Option<Reg>,
    reclaim: Option<(Reg, u32)>,
) {
    match (dst, src) {
        (Some(d), Some(s)) => auth.regs.copy(d, s),
        (None, Some(s)) if reclaim.is_none() => state.output_reg = Some(s),
        _ => {}
    }
    if let Some((base, count)) = reclaim {
        auth.regs.drop_range(base, count);
    }
}

pub(crate) fn replay_trap<C>(
    directive: &Directive,
    reason: &Trap,
    auth: &AuthState<C::Wire>,
    exec: &mut C,
) -> Result<()>
where
    C: ZkExec,
    C::Error: core::fmt::Debug,
{
    let (lhs, rhs, width) = trap_operands(directive)?;
    match reason {
        Trap::DivideByZero => {
            let b = operand_value(rhs, auth, exec)?;
            assert_divisor_zero(exec, &b, width)
        }
        Trap::IntegerOverflow => {
            let a = operand_value(lhs, auth, exec)?;
            let b = operand_value(rhs, auth, exec)?;
            assert_overflow(exec, &a, &b, width)
        }
        other => Err(ZkVmError::Internal(format!(
            "replay: unproven committed trap reason {other:?} for {directive:?}"
        ))),
    }
}

fn trap_operands(directive: &Directive) -> Result<(&Operand, &Operand, usize)> {
    use BinaryOp::*;
    match directive {
        Directive::Op(Op::Binary { op, lhs, rhs, .. }) => match op {
            I32DivU | I32RemU | I32DivS | I32RemS => Ok((lhs, rhs, 32)),
            I64DivU | I64RemU | I64DivS | I64RemS => Ok((lhs, rhs, 64)),
            other => Err(ZkVmError::Internal(format!(
                "replay: trap directive op is not a div/rem: {other:?}"
            ))),
        },
        other => Err(ZkVmError::Internal(format!(
            "replay: trap directive is not a binary op: {other:?}"
        ))),
    }
}

fn assert_const_bits<C>(
    ctx: &mut C,
    value: &AuthValue<C::Wire>,
    width: usize,
    bits: u64,
) -> Result<()>
where
    C: Context<Field = Gf2>,
    C::Error: core::fmt::Debug,
{
    let wires = match width {
        32 => value.try_as_i32()?.to_wires().to_vec(),
        64 => value.try_as_i64()?.to_wires().to_vec(),
        other => {
            return Err(ZkVmError::Internal(format!(
                "replay: unexpected trap operand width {other}"
            )));
        }
    };
    for (i, w) in wires.into_iter().enumerate() {
        ctx.assert_const(w, Gf2((bits >> i) & 1 != 0))
            .map_err(|e| ZkVmError::Internal(format!("assert_const: {e:?}")))?;
    }
    Ok(())
}

fn assert_divisor_zero<C>(ctx: &mut C, divisor: &AuthValue<C::Wire>, width: usize) -> Result<()>
where
    C: Context<Field = Gf2>,
    C::Error: core::fmt::Debug,
{
    assert_const_bits(ctx, divisor, width, 0)
}

fn assert_overflow<C>(
    ctx: &mut C,
    lhs: &AuthValue<C::Wire>,
    rhs: &AuthValue<C::Wire>,
    width: usize,
) -> Result<()>
where
    C: Context<Field = Gf2>,
    C::Error: core::fmt::Debug,
{
    let all_ones = if width == 64 {
        u64::MAX
    } else {
        (1u64 << width) - 1
    };
    let int_min = 1u64 << (width - 1);
    assert_const_bits(ctx, rhs, width, all_ones)?;
    assert_const_bits(ctx, lhs, width, int_min)
}

fn propagate_args<W: Copy>(auth: &mut AuthState<W>, param_base: Reg, args: &[Operand]) {
    for (i, arg) in args.iter().enumerate() {
        if let Operand::Symbol { reg, .. } = arg {
            auth.regs.copy(param_base + i as u32, *reg);
        }
    }
}
