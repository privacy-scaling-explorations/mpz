use std::collections::BTreeMap;

use mpz_vm_core::{Error as CoreError, Global, Operand, Reg, Visibility, value::Value};
use mpz_vm_ir::{Function, Module};
use serde::{Deserialize, Serialize};

use crate::{
    capture::Role,
    error::{Result, ZkVmError},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) enum RevealPayload {
    Scalar(Value),
    Bytes { ptr: u32, len: u32, bytes: Vec<u8> },
}

#[derive(Clone, Debug)]
pub(crate) enum HostCallEvent {
    OpenScalar {
        src: Option<Reg>,
        value: Value,
        handle_dst: Reg,
        id: u32,
    },
    WaitScalar {
        dst: Reg,
        value: Value,
    },
    OpenBytes {
        ptr: u32,
        bytes: Vec<u8>,
        handle_dst: Reg,
        id: u32,
    },
    WaitBytes,
    Sha256Compress {
        state_ptr: u32,
        block_ptr: u32,
        state_pub: [u8; 32],
        state_sym: u64,
        block_pub: [u8; 64],
        block_sym: u64,
    },
    Sha256CompressPublic {
        state_ptr: u32,
        digest: [u8; 32],
    },
}

#[derive(Debug, Default)]
pub(crate) struct RevealState {
    next_id: u32,
    payloads: BTreeMap<u32, RevealPayload>,
}

impl RevealState {
    pub(crate) fn merge(&mut self, announced: BTreeMap<u32, RevealPayload>) {
        self.payloads.extend(announced);
    }

    fn alloc(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn disclose(
        &mut self,
        new: &mut BTreeMap<u32, RevealPayload>,
        id: u32,
        payload: RevealPayload,
    ) {
        self.payloads.insert(id, payload.clone());
        new.insert(id, payload);
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn service_reveal(
    role: Role,
    state: &mut RevealState,
    new: &mut BTreeMap<u32, RevealPayload>,
    module: &Module,
    global: &mut Global,
    func_idx: u32,
    dst: Option<Reg>,
    args: &[Operand],
) -> Result<(HostCallEvent, Option<Value>, Visibility)> {
    let name = match module.function(func_idx) {
        Some(Function::Import(import)) if import.module() == "vc" => import.name(),
        _ => {
            return Err(ZkVmError::Unsupported(
                "zk-vm services only `vc` reveal imports".into(),
            ));
        }
    };

    match name {
        "reveal_i32" | "reveal_i64" => {
            let id = state.alloc();
            let (src, value) = match args.first() {
                Some(Operand::Concrete(v)) => (None, *v),
                Some(Operand::Symbol { reg, value }) => {
                    let v = match role {
                        Role::Prover => (*value).ok_or_else(|| {
                            ZkVmError::Internal("prover does not hold revealed value".into())
                        })?,
                        Role::Verifier => scalar(state.payloads.get(&id), id)?,
                    };
                    (Some(*reg), v)
                }
                None => return Err(ZkVmError::Internal("reveal call missing argument".into())),
            };
            if role == Role::Prover {
                state.disclose(new, id, RevealPayload::Scalar(value));
            }
            let handle_dst = handle_dst(dst)?;
            let event = HostCallEvent::OpenScalar {
                src,
                value,
                handle_dst,
                id,
            };
            Ok((event, Some(Value::I32(id as i32)), Visibility::Public))
        }
        "reveal_i64_wait" | "reveal_i32_wait" => {
            let handle = handle_arg(args)?;
            let value = scalar(state.payloads.get(&handle), handle)?;
            let dst =
                dst.ok_or_else(|| ZkVmError::Internal("reveal wait has no destination".into()))?;
            Ok((
                HostCallEvent::WaitScalar { dst, value },
                Some(value),
                Visibility::Public,
            ))
        }
        "reveal_bytes" => {
            let id = state.alloc();
            let ptr = arg_u32(args, 0)?;
            let len = arg_u32(args, 1)?;
            let bytes = match role {
                Role::Prover => global
                    .memory()
                    .ok_or(ZkVmError::Core(CoreError::MemoryNotDefined))?
                    .read_bytes(ptr, len as usize)
                    .map_err(ZkVmError::Trap)?
                    .to_vec(),
                Role::Verifier => bytes_payload(state.payloads.get(&id), id)?.2,
            };
            if role == Role::Prover {
                state.disclose(
                    new,
                    id,
                    RevealPayload::Bytes {
                        ptr,
                        len,
                        bytes: bytes.clone(),
                    },
                );
            }
            let handle_dst = handle_dst(dst)?;
            let event = HostCallEvent::OpenBytes {
                ptr,
                bytes,
                handle_dst,
                id,
            };
            Ok((event, Some(Value::I32(id as i32)), Visibility::Public))
        }
        "reveal_bytes_wait" => {
            let handle = handle_arg(args)?;
            let (ptr, len, bytes) = bytes_payload(state.payloads.get(&handle), handle)?;
            if role == Role::Verifier {
                global
                    .memory_mut()
                    .ok_or(ZkVmError::Core(CoreError::MemoryNotDefined))?
                    .write_bytes(ptr, &bytes)
                    .map_err(ZkVmError::Trap)?;
            }
            global.set_memory_visibility(ptr, len as usize, Visibility::Public);
            Ok((HostCallEvent::WaitBytes, None, Visibility::Public))
        }
        other => Err(ZkVmError::Unsupported(format!(
            "zk-vm does not service reveal import `vc::{other}`"
        ))),
    }
}

pub(crate) fn service_sha256_compress(
    role: Role,
    global: &mut Global,
    args: &[Operand],
) -> Result<(HostCallEvent, Option<Value>, Visibility)> {
    let state_ptr = arg_u32(args, 0)?;
    let block_ptr = arg_u32(args, 1)?;

    let (state_pub, state_sym) = masked_input::<32>(global, state_ptr)?;
    let (block_pub, block_sym) = masked_input::<64>(global, block_ptr)?;

    if state_sym == 0 && block_sym == 0 {
        let digest = compress_block(&state_pub, &block_pub);
        write_state(global, state_ptr, &digest)?;
        global.set_memory_visibility(state_ptr, 32, Visibility::Public);
        log_digest_write(global, state_ptr, Some(&digest));
        return Ok((
            HostCallEvent::Sha256CompressPublic { state_ptr, digest },
            None,
            Visibility::Public,
        ));
    }

    match role {
        Role::Prover => {
            let (state, block) = read_inputs(global, state_ptr, block_ptr)?;
            let digest = compress_block(&state, &block);
            write_state(global, state_ptr, &digest)?;
            global.set_memory_visibility(state_ptr, 32, Visibility::Private);
        }
        Role::Verifier => global.set_memory_visibility(state_ptr, 32, Visibility::Blind),
    }
    log_digest_write(global, state_ptr, None);
    Ok((
        HostCallEvent::Sha256Compress {
            state_ptr,
            block_ptr,
            state_pub,
            state_sym,
            block_pub,
            block_sym,
        },
        None,
        Visibility::Public,
    ))
}

fn masked_input<const N: usize>(global: &Global, ptr: u32) -> Result<([u8; N], u64)> {
    debug_assert!(N <= 64, "symbolic mask is a u64");
    let mem = global
        .memory()
        .ok_or(ZkVmError::Core(CoreError::MemoryNotDefined))?;
    let raw = mem.read_bytes(ptr, N).map_err(ZkVmError::Trap)?;
    let mut public = [0u8; N];
    let mut sym = 0u64;
    for (i, slot) in public.iter_mut().enumerate() {
        if global.memory_tainted(ptr + i as u32, 1) {
            sym |= 1u64 << i;
        } else {
            *slot = raw[i];
        }
    }
    Ok((public, sym))
}

fn read_inputs(global: &Global, state_ptr: u32, block_ptr: u32) -> Result<([u8; 32], [u8; 64])> {
    let mem = global
        .memory()
        .ok_or(ZkVmError::Core(CoreError::MemoryNotDefined))?;
    let mut state = [0u8; 32];
    state.copy_from_slice(mem.read_bytes(state_ptr, 32).map_err(ZkVmError::Trap)?);
    let mut block = [0u8; 64];
    block.copy_from_slice(mem.read_bytes(block_ptr, 64).map_err(ZkVmError::Trap)?);
    Ok((state, block))
}

fn log_digest_write(global: &mut Global, state_ptr: u32, digest: Option<&[u8; 32]>) {
    for i in 0..4u32 {
        let (symbolic_mask, value) = match digest {
            Some(d) => {
                let off = (i * 8) as usize;
                let chunk = d[off..off + 8].try_into().expect("8-byte digest chunk");
                (0, Some(u64::from_le_bytes(chunk)))
            }
            None => (0xff, None),
        };
        global.log_host_write(state_ptr + i * 8, 8, symbolic_mask, value);
    }
}

fn write_state(global: &mut Global, state_ptr: u32, digest: &[u8; 32]) -> Result<()> {
    global
        .memory_mut()
        .ok_or(ZkVmError::Core(CoreError::MemoryNotDefined))?
        .write_bytes(state_ptr, digest)
        .map_err(ZkVmError::Trap)?;
    Ok(())
}

fn compress_block(state: &[u8; 32], block: &[u8; 64]) -> [u8; 32] {
    let mut h: [u32; 8] = core::array::from_fn(|i| {
        u32::from_le_bytes([
            state[4 * i],
            state[4 * i + 1],
            state[4 * i + 2],
            state[4 * i + 3],
        ])
    });
    sha2::compress256(&mut h, &[(*block).into()]);
    let mut digest = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        digest[4 * i..4 * i + 4].copy_from_slice(&word.to_le_bytes());
    }
    digest
}

fn scalar(payload: Option<&RevealPayload>, id: u32) -> Result<Value> {
    match payload {
        Some(RevealPayload::Scalar(v)) => Ok(*v),
        _ => Err(ZkVmError::Internal(format!(
            "no scalar reveal payload for id {id}"
        ))),
    }
}

fn bytes_payload(payload: Option<&RevealPayload>, id: u32) -> Result<(u32, u32, Vec<u8>)> {
    match payload {
        Some(RevealPayload::Bytes { ptr, len, bytes }) => Ok((*ptr, *len, bytes.clone())),
        _ => Err(ZkVmError::Internal(format!(
            "no byte reveal payload for id {id}"
        ))),
    }
}

fn handle_dst(dst: Option<Reg>) -> Result<Reg> {
    dst.ok_or_else(|| ZkVmError::Internal("reveal call has no handle destination".into()))
}

fn handle_arg(args: &[Operand]) -> Result<u32> {
    let v = match args.first() {
        Some(Operand::Concrete(v)) | Some(Operand::Symbol { value: Some(v), .. }) => *v,
        _ => {
            return Err(ZkVmError::Internal(
                "reveal wait handle is not available".into(),
            ));
        }
    };
    v.as_i32()
        .map(|h| h as u32)
        .map_err(|_| ZkVmError::Internal("reveal wait handle is not an i32".into()))
}

fn arg_u32(args: &[Operand], i: usize) -> Result<u32> {
    let v = match args.get(i) {
        Some(Operand::Concrete(v)) | Some(Operand::Symbol { value: Some(v), .. }) => *v,
        _ => {
            return Err(ZkVmError::Unsupported(
                "zk-vm: reveal_bytes requires concrete ptr and len".into(),
            ));
        }
    };
    v.as_i32()
        .map(|x| x as u32)
        .map_err(|_| ZkVmError::Internal("reveal_bytes ptr/len is not an i32".into()))
}
