use std::{collections::HashMap, ops::Range, sync::Arc};

use mpz_circuits::Context;
use mpz_fields::{gf2::Gf2, gf2_128::Gf2_128};
use mpz_vm_core::{Reg, ValType, value::Value};
use mpz_vm_memory::{
    AuthState, AuthValue, Bit, Byte, RegDelta, Registers, SharedMemory, SharedRegs,
};

use crate::{
    capture::ChunkCapture,
    commit::{ty_width, value_le_bits},
    error::{Result, ZkVmError},
};

#[derive(Debug)]
pub(crate) struct Plan {
    pub(crate) deltas: Vec<Boundary>,
    pub(crate) segments: Vec<Segment>,
    pub(crate) tape_len: usize,
}

#[derive(Debug)]
pub(crate) struct Segment {
    pub(crate) directives: Range<usize>,
    pub(crate) reveals: Range<usize>,
    pub(crate) log: Range<u64>,
    pub(crate) tape: Range<usize>,
    pub(crate) chi_gates: usize,
    pub(crate) layers: usize,
}

#[derive(Debug)]
pub(crate) struct Boundary {
    pub(crate) tape: Range<usize>,
    pub(crate) delta: BoundaryDelta,
}

#[derive(Debug, Clone)]
pub(crate) struct BoundaryDelta {
    pub(crate) regs: Vec<ValItem>,
    pub(crate) dropped: Vec<(Reg, u32)>,
    pub(crate) globals: Vec<ValItem>,
    pub(crate) mem: Vec<MemItem>,
}

impl BoundaryDelta {
    pub(crate) fn tape_len(&self) -> usize {
        self.regs.iter().map(ValItem::tape_bits).sum::<usize>()
            + self.globals.iter().map(ValItem::tape_bits).sum::<usize>()
            + self.mem.iter().map(MemItem::tape_bits).sum::<usize>()
    }
}

#[derive(Debug, Clone)]
pub(crate) enum ValItem {
    Sym {
        key: u32,
        ty: ValType,
        value: Option<Value>,
    },
    Pub {
        key: u32,
        value: Value,
    },
}

impl ValItem {
    fn tape_bits(&self) -> usize {
        match self {
            ValItem::Sym { ty, .. } => ty_width(*ty),
            ValItem::Pub { .. } => 0,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum MemItem {
    Sym { addr: u32, value: Option<u8> },
    Pub { addr: u32, value: u8 },
}

impl MemItem {
    fn tape_bits(&self) -> usize {
        match self {
            MemItem::Sym { .. } => 8,
            MemItem::Pub { .. } => 0,
        }
    }
}

#[tracing::instrument(
    level = "debug",
    skip_all,
    fields(segments = tracing::field::Empty, tape_len = tracing::field::Empty)
)]
pub(crate) fn plan(chunk: &ChunkCapture, prologue: Option<BoundaryDelta>) -> Plan {
    let mut segments = Vec::with_capacity(chunk.segments.len());
    let mut deltas = Vec::new();
    let mut offset = 0usize;
    if let Some(delta) = prologue {
        let len = delta.tape_len();
        let tape = offset..offset + len;
        offset += len;
        deltas.push(Boundary { tape, delta });
    }
    let mut chi = 0usize;
    for info in &chunk.segments {
        let layers = deltas.len();
        let tape = offset..offset + info.bits;
        offset += info.bits;
        let chi_gates = chi;
        chi += info.gates;
        if let Some(delta) = info.boundary.as_ref() {
            let len = delta.tape_len();
            let tape = offset..offset + len;
            offset += len;
            deltas.push(Boundary {
                tape,
                delta: delta.clone(),
            });
        }
        segments.push(Segment {
            directives: info.directives.clone(),
            reveals: info.reveals.clone(),
            log: info.log.clone(),
            tape,
            chi_gates,
            layers,
        });
    }
    let span = tracing::Span::current();
    span.record("segments", segments.len());
    span.record("tape_len", offset);
    Plan {
        deltas,
        segments,
        tape_len: offset,
    }
}

pub(crate) struct Shared<W> {
    regs: Arc<SharedRegs<AuthValue<W>>>,
    globals: Arc<SharedRegs<AuthValue<W>>>,
    memory: Arc<SharedMemory<W>>,
}

type Deltas<W> = (
    RegDelta<AuthValue<W>>,
    RegDelta<AuthValue<W>>,
    HashMap<u32, Byte<W>>,
);

fn materialize_boundary<W: Copy>(
    delta: &BoundaryDelta,
    wires: &[W],
    pub_bit: &dyn Fn(bool) -> W,
) -> Result<Deltas<W>> {
    let mut k = 0usize;

    let mut reg_sets = HashMap::new();
    let mut glob_sets = HashMap::new();
    for (items, sets) in [
        (&delta.regs, &mut reg_sets),
        (&delta.globals, &mut glob_sets),
    ] {
        for item in items {
            match item {
                ValItem::Sym { key, ty, .. } => {
                    let bits: Vec<Bit<W>> = wires[k..k + ty_width(*ty)]
                        .iter()
                        .map(|w| Bit(*w))
                        .collect();
                    k += ty_width(*ty);
                    sets.insert(Reg(*key), AuthValue::from_bits(*ty, &bits)?);
                }
                ValItem::Pub { key, value } => {
                    sets.insert(Reg(*key), pub_value(*value, pub_bit)?);
                }
            }
        }
    }

    let mut mem = HashMap::new();
    for item in &delta.mem {
        match item {
            MemItem::Sym { addr, .. } => {
                let bits: Vec<Bit<W>> = wires[k..k + 8].iter().map(|w| Bit(*w)).collect();
                k += 8;
                mem.insert(*addr, Byte::new(core::array::from_fn(|i| bits[i])));
            }
            MemItem::Pub { addr, value } => {
                mem.insert(
                    *addr,
                    Byte::new(core::array::from_fn(|i| {
                        Bit(pub_bit((value >> i) & 1 != 0))
                    })),
                );
            }
        }
    }
    debug_assert_eq!(k, wires.len());
    Ok((
        RegDelta {
            sets: reg_sets,
            dropped: delta.dropped.clone(),
        },
        RegDelta {
            sets: glob_sets,
            dropped: Vec::new(),
        },
        mem,
    ))
}

pub(crate) fn plaintext_bits(delta: &BoundaryDelta) -> Result<Vec<bool>> {
    let mut bits = Vec::with_capacity(delta.tape_len());
    for item in delta.regs.iter().chain(&delta.globals) {
        if let ValItem::Sym { ty, value, .. } = item {
            let v = value.ok_or_else(|| {
                ZkVmError::Internal("boundary plaintext missing on prover".into())
            })?;
            bits.extend(value_le_bits(v, ty_width(*ty)));
        }
    }
    for item in &delta.mem {
        if let MemItem::Sym { value, .. } = item {
            let b = value.ok_or_else(|| {
                ZkVmError::Internal("boundary plaintext missing on prover".into())
            })?;
            bits.extend((0..8).map(|i| (b >> i) & 1 != 0));
        }
    }
    Ok(bits)
}

pub(crate) fn build_shared<W: Copy>(
    base: &AuthState<W>,
    plan: &Plan,
    delta_wires: &[Vec<W>],
    pub_bit: &dyn Fn(bool) -> W,
) -> Result<Shared<W>> {
    let mut regs = SharedRegs::new(&base.regs);
    let mut globals = SharedRegs::new(&base.globals);
    let mut memory = SharedMemory::new(&base.memory);
    for (b, wires) in plan.deltas.iter().zip(delta_wires) {
        let (rd, gd, md) = materialize_boundary(&b.delta, wires, pub_bit)?;
        regs.push(rd);
        globals.push(gd);
        memory.push(md);
    }
    Ok(Shared {
        regs: Arc::new(regs),
        globals: Arc::new(globals),
        memory: Arc::new(memory),
    })
}

pub(crate) fn layered_auth<W: Copy>(
    base: &AuthState<W>,
    shared: &Shared<W>,
    layers: usize,
) -> AuthState<W> {
    AuthState {
        regs: Registers::layered(shared.regs.clone(), layers),
        globals: Registers::layered(shared.globals.clone(), layers),
        memory: base.memory.layered(shared.memory.clone(), layers),
    }
}

pub(crate) fn apply_delta<W: Copy>(
    base: &mut AuthState<W>,
    delta: &BoundaryDelta,
    wires: &[W],
    pub_bit: &dyn Fn(bool) -> W,
) -> Result<()> {
    let (rd, gd, md) = materialize_boundary(delta, wires, pub_bit)?;
    for (reg, value) in rd.sets {
        base.regs.set(reg, value);
    }
    for (reg, value) in gd.sets {
        base.globals.set(reg, value);
    }
    for (addr, byte) in md {
        base.memory.set_byte(addr, byte);
    }
    Ok(())
}

pub(crate) fn assert_boundary<C>(
    auth: &AuthState,
    delta: &BoundaryDelta,
    wires: &[Gf2_128],
    ctx: &mut C,
) -> Result<()>
where
    C: Context<Wire = Gf2_128, Field = Gf2>,
    C::Error: std::fmt::Debug,
{
    let mut k = 0usize;
    let mut assert_wires = |ctx: &mut C, state: &[Bit], n: usize, label: &str| -> Result<()> {
        for i in 0..n {
            ctx.assert_eq(state[i].0, wires[k + i]).map_err(|e| {
                ZkVmError::Internal(format!(
                    "boundary assert at {label} bit {i}: wire lsb {} vs committed lsb {} ({e:?})",
                    state[i].0.to_inner() & 1,
                    wires[k + i].to_inner() & 1,
                ))
            })?;
        }
        k += n;
        Ok(())
    };
    for item in &delta.regs {
        if let ValItem::Sym { key, ty, .. } = item {
            let av = auth
                .regs
                .get(Reg(*key))
                .ok_or(ZkVmError::RegAuthMissing { reg: Reg(*key) })?;
            assert_wires(ctx, av.bits(), ty_width(*ty), &format!("reg {key}"))?;
        }
    }
    for item in &delta.globals {
        if let ValItem::Sym { key, ty, .. } = item {
            let av = auth
                .globals
                .get(Reg(*key))
                .ok_or(ZkVmError::GlobalAuthMissing { idx: *key })?;
            assert_wires(ctx, av.bits(), ty_width(*ty), &format!("global {key}"))?;
        }
    }
    for item in &delta.mem {
        if let MemItem::Sym { addr, .. } = item {
            let byte = auth
                .memory
                .get_byte(*addr)
                .ok_or(ZkVmError::MemAuthMissing { addr: *addr })?;
            assert_wires(ctx, byte.bits(), 8, &format!("mem {addr:#x}"))?;
        }
    }
    debug_assert_eq!(k, wires.len());
    Ok(())
}

fn pub_value<W: Copy>(value: Value, pub_bit: &dyn Fn(bool) -> W) -> Result<AuthValue<W>> {
    let ty = value.ty();
    let bits: Vec<Bit<W>> = value_le_bits(value, ty_width(ty))
        .into_iter()
        .map(|b| Bit(pub_bit(b)))
        .collect();
    Ok(AuthValue::from_bits(ty, &bits)?)
}
