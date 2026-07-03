use std::collections::BTreeMap;

use mpz_vm_core::{Param, Reg, ValType, value::Value};
use rangeset::set::RangeSet;

use crate::{
    capture::Role,
    error::{Result, ZkVmError},
    segment::{BoundaryDelta, MemItem, ValItem},
};

#[derive(Debug, Default)]
pub(crate) struct PendingIo {
    write_private: RangeSet<u32>,
    bytes: BTreeMap<u32, u8>,
}

impl PendingIo {
    pub(crate) fn cost_bits(&self) -> usize {
        self.write_private.len() * 8
    }

    pub(crate) fn write_private(&mut self, addr: u32, len: usize) {
        self.write_private.union_mut(addr..addr + len as u32);
    }

    pub(crate) fn stage_private(&mut self, addr: u32, data: &[u8]) {
        self.write_private(addr, data.len());
        for (i, &b) in data.iter().enumerate() {
            self.bytes.insert(addr + i as u32, b);
        }
    }

    pub(crate) fn addrs(&self) -> impl Iterator<Item = u32> + '_ {
        self.write_private.iter().flat_map(|r| r.start..r.end)
    }

    fn staged_byte(&self, addr: u32) -> Option<u8> {
        self.bytes.get(&addr).copied()
    }

    pub(crate) fn clear(&mut self) {
        self.write_private = RangeSet::default();
        self.bytes.clear();
    }
}

pub(crate) fn ty_width(ty: mpz_vm_ir::ValType) -> usize {
    match ty {
        mpz_vm_ir::ValType::I32 | mpz_vm_ir::ValType::F32 => 32,
        mpz_vm_ir::ValType::I64 | mpz_vm_ir::ValType::F64 => 64,
    }
}

pub(crate) fn prepare_params(params: &[Param]) -> Result<usize> {
    let mut bits = 0;
    for (i, p) in params.iter().enumerate() {
        let ty = match p {
            Param::Private(v) => v.ty(),
            Param::Blind(ty) => *ty,
            Param::Public(_) => continue,
        };
        if matches!(ty, mpz_vm_ir::ValType::F32 | mpz_vm_ir::ValType::F64) {
            return Err(ZkVmError::Unsupported(format!(
                "param {i}: float ({ty:?}) not supported by zk-vm"
            )));
        }
        bits += ty_width(ty);
    }
    Ok(bits)
}

pub(crate) fn prologue_delta(
    role: Role,
    root_reg_base: Reg,
    params: &[Param],
    pending: &PendingIo,
) -> Result<Option<BoundaryDelta>> {
    let prover = role == Role::Prover;
    let mut regs = Vec::new();
    for (i, p) in params.iter().enumerate() {
        let (ty, value) = match (prover, p) {
            (_, Param::Public(_)) => continue,
            (true, Param::Private(v)) => (v.ty(), Some(*v)),
            (true, Param::Blind(_)) => {
                return Err(ZkVmError::Unsupported(
                    "prover cannot receive blind params".into(),
                ));
            }
            (false, Param::Private(v)) => (v.ty(), None),
            (false, Param::Blind(ty)) => (*ty, None),
        };
        if matches!(ty, ValType::F32 | ValType::F64) {
            return Err(ZkVmError::Unsupported(format!(
                "param {i}: float ({ty:?}) not supported by zk-vm"
            )));
        }
        regs.push(ValItem::Sym {
            key: (root_reg_base + i as u32).0,
            ty,
            value,
        });
    }

    let mut mem = Vec::new();
    for addr in pending.addrs() {
        let value = if prover {
            Some(pending.staged_byte(addr).ok_or_else(|| {
                ZkVmError::Internal(format!("no staged byte for committed input addr {addr:#x}"))
            })?)
        } else {
            None
        };
        mem.push(MemItem::Sym { addr, value });
    }

    if regs.is_empty() && mem.is_empty() {
        return Ok(None);
    }
    Ok(Some(BoundaryDelta {
        regs,
        dropped: Vec::new(),
        globals: Vec::new(),
        mem,
    }))
}

pub(crate) fn value_le_bits(v: Value, width: usize) -> Vec<bool> {
    let bytes = v.to_le_bytes();
    (0..width)
        .map(|i| (bytes[i / 8] >> (i % 8)) & 1 != 0)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prover_rejects_blind_param() {
        let pending = PendingIo::default();
        let err = prologue_delta(
            Role::Prover,
            Reg(0),
            &[Param::Blind(ValType::I32)],
            &pending,
        )
        .unwrap_err();
        assert!(
            matches!(err, ZkVmError::Unsupported(_)),
            "prover must reject a blind param, got {err:?}"
        );
    }

    #[test]
    fn verifier_accepts_blind_param() {
        let pending = PendingIo::default();
        let delta = prologue_delta(
            Role::Verifier,
            Reg(0),
            &[Param::Blind(ValType::I32)],
            &pending,
        )
        .unwrap()
        .expect("a committed blind param yields a delta");
        assert_eq!(
            delta.regs.len(),
            1,
            "the blind param is committed as one reg"
        );
    }
}
