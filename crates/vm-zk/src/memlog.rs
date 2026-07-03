use std::{collections::BTreeMap, ops::Range};

use mpz_vm_core::{AccessAddr, AccessKind, AccessLog, Operand, value::Value};
use mpz_vm_ir::MemArg;

use crate::error::{Result, ZkVmError};

#[derive(Debug, Clone, Copy)]
pub(crate) enum ByteState {
    Symbolic,
    Public(u8),
}

pub(crate) fn written_view(log: &AccessLog, range: Range<u64>) -> BTreeMap<u32, ByteState> {
    let mut written = BTreeMap::new();
    for clock in range {
        let entry = &log.entries()[(clock - log.base()) as usize];
        if entry.kind != AccessKind::Write || !(entry.emitted || entry.host) {
            continue;
        }
        let AccessAddr::Public(a) = entry.addr else {
            continue;
        };
        for b in 0..entry.width as u32 {
            if entry.symbolic_mask & (1 << b) != 0 {
                written.insert(a + b, ByteState::Symbolic);
            } else {
                let value = entry
                    .value
                    .expect("a public written byte should carry its concrete value");
                written.insert(a + b, ByteState::Public(value.to_le_bytes()[b as usize]));
            }
        }
    }
    written
}

pub(crate) fn eff_addr(addr: &Operand, memarg: &MemArg) -> Result<u32> {
    match addr {
        Operand::Concrete(Value::I32(a)) => Ok((*a as u64 + memarg.offset as u64) as u32),
        Operand::Symbol { .. } => Err(ZkVmError::Unsupported(
            "zk-vm: symbolic memory address not supported".into(),
        )),
        other => Err(ZkVmError::Internal(format!(
            "memory address operand is not i32: {other:?}"
        ))),
    }
}
