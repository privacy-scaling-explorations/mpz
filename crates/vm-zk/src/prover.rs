use mpz_common::{Context, Flush};
use mpz_core::Block;
use mpz_fields::{gf2::Gf2, gf2_128::Gf2_128};
use mpz_ot_core::rcot::{RCOTReceiver, RCOTReceiverOutput};
use mpz_vm_core::{
    Call, Error as CoreError, Global, Param, Reg, Thread, Visibility, Vm, Write, value::Value,
};
use mpz_vm_ir::{Function, Module};

use mpz_vm_memory::{AuthState, Bit, Registers};
use mpz_zk_core::{Commitment, MAC_ONE, MAC_ZERO, Proof, ProverOutput, prover_wire, vope_receiver};
use rand_chacha::{ChaCha12Rng, rand_core::SeedableRng};
use rangeset::set::RangeSet;
use rayon::prelude::*;
use serio::{SinkExt, stream::IoStreamExt};
use std::ops::Range;
use tracing::Instrument;

use crate::{
    ChunkOutcome, Config, ProofMessage, VOPE_BITS,
    capture::{self, ChunkCapture, Role},
    commit::{self, PendingIo, prepare_params},
    error::ZkVmError,
    finalize, host,
    replay::{self, ReplayState},
    reveal, segment,
};

#[derive(Debug)]
pub struct Prover<T> {
    module: Module,
    global: Global,
    svole: T,
    pending_io: PendingIo,
    pending_reveal: RangeSet<u32>,
    reveal_state: host::RevealState,
    auth: AuthState,
    auth_clear: AuthState<Gf2>,
    config: Config,
}

impl<T> Prover<T> {
    pub fn new(module: Module, svole: T) -> Result<Self, ZkVmError> {
        Self::new_with_config(module, svole, Config::default())
    }

    pub fn new_with_config(module: Module, svole: T, config: Config) -> Result<Self, ZkVmError> {
        let mut global = Global::new(&module)?;
        global.enable_access_log();
        let auth = AuthState::new(Bit(MAC_ZERO), Bit(MAC_ONE));
        let auth_clear = AuthState::new(Bit(Gf2(false)), Bit(Gf2(true)));
        Ok(Self {
            module,
            global,
            svole,
            pending_io: PendingIo::default(),
            pending_reveal: RangeSet::default(),
            reveal_state: host::RevealState::default(),
            auth,
            auth_clear,
            config,
        })
    }

    fn publish_revealed(&mut self, ranges: &[Range<u32>]) {
        for r in ranges {
            self.global.set_memory_visibility(
                r.start,
                (r.end - r.start) as usize,
                Visibility::Public,
            );
        }
    }
}

impl<T> Prover<T>
where
    T: RCOTReceiver<bool, Block> + Flush,
{
    #[tracing::instrument(level = "debug", skip(self, io))]
    async fn allocate(
        &mut self,
        io: &mut Context,
        total: usize,
    ) -> Result<(Vec<bool>, Vec<Gf2_128>), ZkVmError> {
        self.svole
            .alloc(total)
            .map_err(|e| ZkVmError::SvoleAlloc(e.to_string()))?;
        self.svole
            .flush(io)
            .await
            .map_err(|e| ZkVmError::SvoleFlush(e.to_string()))?;
        let RCOTReceiverOutput {
            choices: masks,
            msgs,
            ..
        } = self
            .svole
            .try_recv_rcot(total)
            .map_err(|e| ZkVmError::SvoleIo(e.to_string()))?;
        let macs = msgs.into_iter().map(|m| zerocopy::transmute!(m)).collect();
        Ok((masks, macs))
    }

    #[tracing::instrument(level = "debug", name = "commit", skip_all)]
    fn commit_pass(
        &mut self,
        chunk: &ChunkCapture,
        plan: &segment::Plan,
    ) -> Result<Vec<bool>, ZkVmError> {
        let mut witness = vec![false; plan.tape_len];

        let mut delta_wires: Vec<Vec<Gf2>> = Vec::with_capacity(plan.deltas.len());
        for d in &plan.deltas {
            let bits = segment::plaintext_bits(&d.delta)?;
            witness[d.tape.clone()].copy_from_slice(&bits);
            delta_wires.push(bits.iter().map(|&bit| Gf2(bit)).collect());
        }

        let shared = segment::build_shared(&self.auth_clear, plan, &delta_wires, &pub_bit_witness)?;

        let gate_slices = gate_mask_slices(&mut witness, &plan.segments);
        let module = &self.module;
        let auth_base = &self.auth_clear;
        let last = plan.segments.len() - 1;
        let finals: Vec<Option<AuthState<Gf2>>> = plan
            .segments
            .par_iter()
            .zip(gate_slices)
            .enumerate()
            .map(
                |(j, (seg, gslice))| -> Result<Option<AuthState<Gf2>>, ZkVmError> {
                    let mut auth = segment::layered_auth(auth_base, &shared, seg.layers);
                    let mut ctx = mpz_zk_core::Witness::new(gslice);
                    let mut state = ReplayState::root();
                    replay::replay(
                        &chunk.trace[seg.directives.clone()],
                        &chunk.reveal_actions[seg.reveals.clone()],
                        module,
                        &mut auth,
                        &mut ctx,
                        &mut state,
                    )?;
                    ctx.finish()
                        .map_err(|e| ZkVmError::Internal(e.to_string()))?;
                    Ok((j == last).then(|| auth.flatten()))
                },
            )
            .collect::<Result<_, _>>()?;

        self.auth_clear = finals
            .into_iter()
            .flatten()
            .next_back()
            .expect("last segment produces final cleartext state");

        Ok(witness)
    }

    #[tracing::instrument(level = "debug", name = "accumulate", skip_all)]
    #[allow(clippy::too_many_arguments)]
    fn accumulate_pass(
        &self,
        chunk: &ChunkCapture,
        plan: &segment::Plan,
        exec_macs: &[Gf2_128],
        chi: [u8; 32],
        reveal_ranges: &[Range<u32>],
        reveal_pending: bool,
    ) -> Result<AccOut, ZkVmError> {
        let mut delta_wires: Vec<Vec<Gf2_128>> = Vec::with_capacity(plan.deltas.len());
        for d in &plan.deltas {
            let bits = segment::plaintext_bits(&d.delta)?;
            delta_wires.push(
                bits.iter()
                    .enumerate()
                    .map(|(i, &bit)| prover_wire(exec_macs[d.tape.start + i], bit))
                    .collect(),
            );
        }

        let module = &self.module;
        let auth_base = &self.auth;
        let memory = self.global.memory();
        let last = plan.segments.len() - 1;
        let shared = segment::build_shared(auth_base, plan, &delta_wires, &pub_bit_prover)?;

        let results: Vec<(Gf2_128, Gf2_128, [u8; 32], Option<LastOut>)> = plan
            .segments
            .par_iter()
            .enumerate()
            .map(|(j, seg)| -> Result<_, ZkVmError> {
                let mut auth = segment::layered_auth(auth_base, &shared, seg.layers);

                let mut rng = ChaCha12Rng::from_seed(chi);
                rng.set_word_pos((seg.chi_gates as u128) * 4);
                let mut ctx = mpz_zk_core::Accumulate::new(&exec_macs[seg.tape.clone()], rng);
                let mut state = ReplayState::root();
                replay::replay(
                    &chunk.trace[seg.directives.clone()],
                    &chunk.reveal_actions[seg.reveals.clone()],
                    module,
                    &mut auth,
                    &mut ctx,
                    &mut state,
                )?;

                if let Some(b) = plan.deltas.get(seg.layers) {
                    let wires = &delta_wires[seg.layers];
                    segment::assert_boundary(&auth, &b.delta, wires, &mut ctx).map_err(|e| {
                        ZkVmError::Internal(format!(
                            "segment {j} (directives {:?}): {e}",
                            seg.directives
                        ))
                    })?;
                }

                let mut last_out = None;
                if j == last {
                    let mut revealed = Vec::new();
                    match &chunk.trap {
                        Some(t) => {
                            if let Some(directive) = &t.directive {
                                replay::replay_trap(directive, &t.trap, &auth, &mut ctx)?;
                            }
                        }
                        None => {
                            if chunk.result_symbolic {
                                finalize::bind_output(&state, &mut ctx, &auth, chunk.result)?;
                            }
                        }
                    }
                    if reveal_pending {
                        let memory = memory.ok_or(ZkVmError::Core(CoreError::MemoryNotDefined))?;
                        revealed = reveal::reveal_prover(&mut ctx, &auth, memory, reveal_ranges)?;
                    }
                    last_out = Some(LastOut {
                        auth: auth.flatten(),
                        revealed,
                    });
                }

                let ProverOutput {
                    u, v, assertions, ..
                } = ctx
                    .finish()
                    .map_err(|e| ZkVmError::Internal(e.to_string()))?;
                Ok((u, v, assertions, last_out))
            })
            .collect::<Result<_, _>>()?;

        let mut u = Gf2_128::new(0);
        let mut v = Gf2_128::new(0);
        let mut hasher = blake3::Hasher::new();
        let mut final_out = None;
        for (u_j, v_j, h_j, last_out) in results {
            u = u + u_j;
            v = v + v_j;
            hasher.update(&h_j);
            if let Some(out) = last_out {
                final_out = Some(out);
            }
        }
        let LastOut { auth, revealed } = final_out.expect("last segment produces final state");

        Ok(AccOut {
            u,
            v,
            assertions: *hasher.finalize().as_bytes(),
            revealed,
            auth,
        })
    }

    async fn prove_chunk(
        &mut self,
        io: &mut Context,
        chunk: &ChunkCapture,
        plan: &segment::Plan,
        reveal_ranges: &[Range<u32>],
        reveal_pending: bool,
    ) -> Result<(), ZkVmError> {
        let execute_bits = plan.tape_len;
        let total = execute_bits + VOPE_BITS;
        tracing::info!(
            cost = chunk.cost,
            total,
            segments = plan.segments.len(),
            "cost plan"
        );

        let witness = self.commit_pass(chunk, plan)?;

        let (mut masks, macs) = self.allocate(io, total).await?;
        let (exec_masks, vope_masks, exec_macs, vope_macs) =
            split_vope(&mut masks, &macs, execute_bits);

        for (m, w) in exec_masks.iter_mut().zip(&witness) {
            *m ^= *w;
        }

        let commitment = Commitment {
            adjust: exec_masks.iter().copied().collect(),
        };
        let chi: [u8; 32] = async {
            io.io_mut()
                .send(commitment)
                .await
                .map_err(|e| ZkVmError::IoSend(e.to_string()))?;
            io.io_mut()
                .expect_next()
                .await
                .map_err(|e| ZkVmError::IoRecv(e.to_string()))
        }
        .instrument(tracing::debug_span!("exchange"))
        .await?;

        let out =
            self.accumulate_pass(chunk, plan, exec_macs, chi, reveal_ranges, reveal_pending)?;
        self.auth = out.auth;

        let out_val = if chunk.result_symbolic {
            chunk.result
        } else {
            None
        };
        let (a_0, a_1) = vope_receiver(vope_masks, vope_macs);
        let proof = Proof {
            assertions: out.assertions,
            u: out.u + a_0,
            v: out.v + a_1,
            coefficients: Vec::new(),
        };
        io.io_mut()
            .send(ProofMessage {
                output: out_val,
                revealed: out.revealed,
                proof,
            })
            .await
            .map_err(|e| ZkVmError::IoSend(e.to_string()))?;

        Ok(())
    }
}

struct AccOut {
    u: Gf2_128,
    v: Gf2_128,
    assertions: [u8; 32],
    revealed: Vec<u8>,
    auth: AuthState,
}

struct LastOut {
    auth: AuthState,
    revealed: Vec<u8>,
}

fn pub_bit_prover(bit: bool) -> Gf2_128 {
    if bit { MAC_ONE } else { MAC_ZERO }
}

fn pub_bit_witness(bit: bool) -> Gf2 {
    Gf2(bit)
}

fn gate_mask_slices<'m>(
    witness: &'m mut [bool],
    segments: &[segment::Segment],
) -> Vec<&'m mut [bool]> {
    let mut out = Vec::with_capacity(segments.len());
    let mut region = witness;
    let mut cursor = 0usize;
    for seg in segments {
        region = region.split_at_mut(seg.tape.start - cursor).1;
        let (gates, rest) = region.split_at_mut(seg.tape.len());
        out.push(gates);
        region = rest;
        cursor = seg.tape.end;
    }
    out
}

fn split_vope<'a>(
    masks: &'a mut [bool],
    macs: &'a [Gf2_128],
    execute_bits: usize,
) -> (
    &'a mut [bool],
    &'a [bool; VOPE_BITS],
    &'a [Gf2_128],
    &'a [Gf2_128; VOPE_BITS],
) {
    let (exec_masks, vope_masks) = masks.split_at_mut(execute_bits);
    let vope_masks: &[bool; VOPE_BITS] = (&*vope_masks)
        .try_into()
        .expect("vope tail is VOPE_BITS wide");
    let (exec_macs, vope_macs) = macs.split_at(execute_bits);
    let vope_macs: &[Gf2_128; VOPE_BITS] =
        vope_macs.try_into().expect("vope tail is VOPE_BITS wide");
    (exec_masks, vope_masks, exec_macs, vope_macs)
}

impl<T> Vm for Prover<T>
where
    T: RCOTReceiver<bool, Block> + Flush,
{
    type Error = ZkVmError;

    fn write(&mut self, ptr: u32, w: Write<'_>) -> Result<(), ZkVmError> {
        let memory = self
            .global
            .memory_mut()
            .ok_or(ZkVmError::Core(CoreError::MemoryNotDefined))?;
        match w {
            Write::Private(data) => {
                memory.write_bytes(ptr, data).map_err(ZkVmError::Trap)?;
                self.pending_io.stage_private(ptr, data);
                self.global
                    .set_memory_visibility(ptr, data.len(), Visibility::Private);
            }
            Write::Public(data) => {
                memory.write_bytes(ptr, data).map_err(ZkVmError::Trap)?;
                self.global
                    .set_memory_visibility(ptr, data.len(), Visibility::Public);
            }
            Write::Blind(_) => {
                return Err(ZkVmError::Unsupported(
                    "prover cannot write blind values".into(),
                ));
            }
        }
        Ok(())
    }

    fn reveal(&mut self, ptr: u32, len: usize) -> Result<(), ZkVmError> {
        self.pending_reveal.union_mut(ptr..ptr + len as u32);
        Ok(())
    }

    fn read(&self, ptr: u32, len: usize) -> Result<&[u8], ZkVmError> {
        if self.global.memory_tainted(ptr, len) {
            return Err(ZkVmError::Internal(format!(
                "cannot read tainted memory at {:#x}",
                ptr
            )));
        }
        let memory = self
            .global
            .memory()
            .ok_or(ZkVmError::Core(CoreError::MemoryNotDefined))?;
        memory.read_bytes(ptr, len).map_err(ZkVmError::Trap)
    }

    #[tracing::instrument(level = "info", skip_all, fields(id = ?self.config.id(), role = "prover", func = self.module.func_name(func_idx).unwrap_or("?")))]
    async fn call(
        &mut self,
        io: &mut Context,
        func_idx: u32,
        params: Vec<Param>,
    ) -> Result<Option<Value>, ZkVmError> {
        let func = match self.module.function(func_idx) {
            Some(Function::Local(f)) => f,
            _ => return Err(ZkVmError::Core(CoreError::InvalidFunction(func_idx))),
        };
        let num_args = params.len() as u32;
        let num_results = func.func_type().results.len() as u32;
        let root_reg_base = Reg(num_results + num_args);

        let mut prologue =
            commit::prologue_delta(Role::Prover, root_reg_base, &params, &self.pending_io)?;

        let mut thread = Thread::new();
        thread.call(
            &self.module,
            &mut self.global,
            Call {
                func_idx,
                params: params.clone(),
            },
        )?;

        self.auth.regs = Registers::new();
        self.auth_clear.regs = Registers::new();
        let mut final_result = None;
        let mut chunk_idx: usize = 0;
        let reveal_ranges: Vec<Range<u32>> = self.pending_reveal.iter().collect();
        let mut reveal_pending = !reveal_ranges.is_empty();
        let mut any_zk_work = false;

        let trapped: Option<mpz_vm_core::Trap> = loop {
            let chunk_span = tracing::info_span!("prover.chunk", idx = chunk_idx);
            let _enter = chunk_span.enter();

            let chunk = capture::capture_chunk(
                &self.module,
                &mut self.global,
                &mut thread,
                capture::Limits {
                    chunk_cap: self.config.chunk_cap(),
                    segment_cost: crate::effective_segment_cost(
                        self.config.segment_cost(),
                        self.config.chunk_cap(),
                    ),
                },
                Role::Prover,
                None,
                &mut self.reveal_state,
            )?;
            tracing::debug!(
                events = chunk.trace.len(),
                cost = chunk.cost,
                done = chunk.done,
                segments = chunk.segments.len(),
                "captured chunk"
            );

            let outcome = ChunkOutcome {
                trap_at: chunk.trap.as_ref().map(|t| t.index),
                trap: chunk.trap.as_ref().map(|t| t.trap.clone()),
                revealed: chunk.reveals.clone(),
            };
            io.io_mut()
                .send(outcome)
                .await
                .map_err(|e| ZkVmError::IoSend(e.to_string()))?;

            let plan = segment::plan(&chunk, prologue.take());
            let log_end = plan.segments.last().expect("at least one segment").log.end;
            let execute_bits = plan.tape_len;

            let needs_zk = execute_bits > 0
                || any_zk_work
                || reveal_pending
                || chunk.trap.as_ref().is_some_and(|t| t.directive.is_some());
            if needs_zk {
                any_zk_work = true;
                self.prove_chunk(io, &chunk, &plan, &reveal_ranges, reveal_pending)
                    .await?;

                if reveal_pending {
                    self.publish_revealed(&reveal_ranges);
                    reveal_pending = false;
                }
            }

            if chunk.done {
                final_result = chunk.result;
            }
            self.global
                .access_log_mut()
                .expect("access log enabled")
                .drain_upto(log_end);
            chunk_idx += 1;
            if let Some(t) = chunk.trap {
                break Some(t.trap);
            }
            if chunk.done {
                break None;
            }
        };

        self.pending_io.clear();
        self.pending_reveal = RangeSet::default();
        if let Some(trap) = trapped {
            tracing::info!(chunks = chunk_idx, ?trap, "prover call trapped");
            return Err(ZkVmError::Trap(trap));
        }
        tracing::info!(chunks = chunk_idx, ?final_result, "prover call complete");
        Ok(final_result)
    }

    #[tracing::instrument(level = "info", skip_all, fields(id = ?self.config.id(), role = "prover"))]
    async fn commit(&mut self, io: &mut Context) -> Result<(), ZkVmError> {
        let prologue = commit::prologue_delta(Role::Prover, Reg(0), &[], &self.pending_io)?;
        let witness = match &prologue {
            Some(delta) => segment::plaintext_bits(delta)?,
            None => Vec::new(),
        };
        let reveal_ranges: Vec<Range<u32>> = self.pending_reveal.iter().collect();
        let reveal_pending = !reveal_ranges.is_empty();
        if witness.is_empty() && !reveal_pending {
            return Ok(());
        }

        let execute_bits = witness.len();
        let total = execute_bits + VOPE_BITS;
        let (mut masks, macs) = self.allocate(io, total).await?;
        let (exec_masks, vope_masks, exec_macs, vope_macs) =
            split_vope(&mut masks, &macs, execute_bits);

        if let Some(delta) = &prologue {
            for (m, w) in exec_masks.iter_mut().zip(&witness) {
                *m ^= *w;
            }
            let clear_wires: Vec<Gf2> = witness.iter().map(|&b| Gf2(b)).collect();
            segment::apply_delta(&mut self.auth_clear, delta, &clear_wires, &pub_bit_witness)?;
            let mac_wires: Vec<Gf2_128> = witness
                .iter()
                .enumerate()
                .map(|(i, &b)| prover_wire(exec_macs[i], b))
                .collect();
            segment::apply_delta(&mut self.auth, delta, &mac_wires, &pub_bit_prover)?;
        }

        let commitment = Commitment {
            adjust: exec_masks.iter().copied().collect(),
        };
        io.io_mut()
            .send(commitment)
            .await
            .map_err(|e| ZkVmError::IoSend(e.to_string()))?;
        let chi: [u8; 32] = io
            .io_mut()
            .expect_next()
            .await
            .map_err(|e| ZkVmError::IoRecv(e.to_string()))?;

        let mut revealed: Vec<u8> = Vec::new();
        let mut ctx = mpz_zk_core::Accumulate::new(&[], ChaCha12Rng::from_seed(chi));
        if reveal_pending {
            let memory = self
                .global
                .memory()
                .ok_or(ZkVmError::Core(CoreError::MemoryNotDefined))?;
            revealed = reveal::reveal_prover(&mut ctx, &self.auth, memory, &reveal_ranges)?;
        }
        let ProverOutput {
            u, v, assertions, ..
        } = ctx
            .finish()
            .map_err(|e| ZkVmError::Internal(e.to_string()))?;

        let (a_0, a_1) = vope_receiver(vope_masks, vope_macs);
        let proof = Proof {
            assertions,
            u: u + a_0,
            v: v + a_1,
            coefficients: Vec::new(),
        };
        io.io_mut()
            .send(ProofMessage {
                output: None,
                revealed,
                proof,
            })
            .await
            .map_err(|e| ZkVmError::IoSend(e.to_string()))?;

        if reveal_pending {
            self.publish_revealed(&reveal_ranges);
        }
        self.pending_io.clear();
        self.pending_reveal = RangeSet::default();
        Ok(())
    }

    fn call_local(
        &mut self,
        func_idx: u32,
        params: Vec<Param>,
    ) -> Result<Option<Value>, ZkVmError> {
        match self.module.function(func_idx) {
            Some(Function::Local(_)) => {}
            _ => return Err(ZkVmError::Core(CoreError::InvalidFunction(func_idx))),
        }
        if prepare_params(&params)? > 0 {
            return Err(ZkVmError::RequiresCommunication(
                "call_local requires public params; private or blind inputs need a proving round"
                    .into(),
            ));
        }
        if self.pending_io.cost_bits() > 0 || !self.pending_reveal.is_empty() {
            return Err(ZkVmError::RequiresCommunication(
                "queued inputs or reveals must be flushed with commit before call_local".into(),
            ));
        }

        let mut thread = Thread::new();
        thread.call(&self.module, &mut self.global, Call { func_idx, params })?;
        capture::run_local(&self.module, &mut self.global, &mut thread)
    }
}
