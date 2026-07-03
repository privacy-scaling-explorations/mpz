use mpz_common::{Context, Flush};
use mpz_core::Block;
use mpz_fields::gf2_128::Gf2_128;
use mpz_ot_core::rcot::{RCOTSender, RCOTSenderOutput};
use mpz_vm_core::{
    Call, Error as CoreError, Global, Param, Reg, Thread, Visibility, Vm, Write, value::Value,
};
use mpz_vm_ir::{Function, Module};
use rand::Rng;
use rand_chacha::{ChaCha12Rng, rand_core::SeedableRng};
use rangeset::set::RangeSet;
use rayon::prelude::*;
use serio::{SinkExt, stream::IoStreamExt};
use std::ops::Range;
use tracing::Instrument;

use mpz_vm_memory::{AuthState, Bit, Registers};
use mpz_zk_core::{Commitment, MAC_ONE, MAC_ZERO, VerifierOutput, verifier_wire, vope_sender};

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
pub struct Verifier<T> {
    module: Module,
    global: Global,
    svole: T,
    pending_io: PendingIo,
    pending_reveal: RangeSet<u32>,
    reveal_state: host::RevealState,
    auth: AuthState,
    delta: Gf2_128,
    config: Config,
}

impl<T> Verifier<T>
where
    T: RCOTSender<Block>,
{
    pub fn new(module: Module, svole: T) -> Result<Self, ZkVmError> {
        Self::new_with_config(module, svole, Config::default())
    }

    pub fn new_with_config(module: Module, svole: T, config: Config) -> Result<Self, ZkVmError> {
        let mut global = Global::new(&module)?;
        global.enable_access_log();
        let delta: Gf2_128 = zerocopy::transmute!(svole.delta());
        if delta.to_inner() & 1 != 1 {
            return Err(ZkVmError::DeltaLsb);
        }
        let auth = AuthState::new(Bit(MAC_ZERO), Bit(MAC_ONE + delta));
        Ok(Self {
            module,
            global,
            svole,
            pending_io: PendingIo::default(),
            pending_reveal: RangeSet::default(),
            reveal_state: host::RevealState::default(),
            auth,
            delta,
            config,
        })
    }
}

impl<T> Verifier<T>
where
    T: RCOTSender<Block> + Flush,
{
    #[tracing::instrument(level = "debug", skip(self, io))]
    async fn allocate(
        &mut self,
        io: &mut Context,
        total: usize,
    ) -> Result<Vec<Gf2_128>, ZkVmError> {
        self.svole
            .alloc(total)
            .map_err(|e| ZkVmError::SvoleAlloc(e.to_string()))?;
        self.svole
            .flush(io)
            .await
            .map_err(|e| ZkVmError::SvoleFlush(e.to_string()))?;
        let RCOTSenderOutput { keys, .. } = self
            .svole
            .try_send_rcot(total)
            .map_err(|e| ZkVmError::SvoleIo(e.to_string()))?;
        Ok(keys.into_iter().map(|k| zerocopy::transmute!(k)).collect())
    }

    #[tracing::instrument(level = "debug", name = "accumulate", skip_all)]
    #[allow(clippy::too_many_arguments)]
    fn accumulate_pass(
        &self,
        chunk: &ChunkCapture,
        plan: &segment::Plan,
        exec_keys: &[Gf2_128],
        exec_adjust: &[bool],
        chi: [u8; 32],
        output: Option<Value>,
        revealed: &[u8],
        reveal_ranges: &[Range<u32>],
        reveal_pending: bool,
    ) -> Result<VAccOut, ZkVmError> {
        let delta = self.delta;
        let delta_wires: Vec<Vec<Gf2_128>> = plan
            .deltas
            .iter()
            .map(|d| {
                d.tape
                    .clone()
                    .map(|i| verifier_wire(exec_keys[i], exec_adjust[i], delta))
                    .collect()
            })
            .collect();

        let module = &self.module;
        let auth_base = &self.auth;
        let last = plan.segments.len() - 1;
        let pub_bit = move |b: bool| if b { MAC_ONE + delta } else { MAC_ZERO };
        let shared = segment::build_shared(auth_base, plan, &delta_wires, &pub_bit)?;

        let results: Vec<(Gf2_128, [u8; 32], Option<AuthState>)> = plan
            .segments
            .par_iter()
            .enumerate()
            .map(|(j, seg)| -> Result<_, ZkVmError> {
                let mut auth = segment::layered_auth(auth_base, &shared, seg.layers);

                let mut rng = ChaCha12Rng::from_seed(chi);
                rng.set_word_pos((seg.chi_gates as u128) * 4);
                let mut ctx = mpz_zk_core::Verifier::new(
                    delta,
                    &exec_keys[seg.tape.clone()],
                    &exec_adjust[seg.tape.clone()],
                    rng,
                )
                .map_err(|e| ZkVmError::Internal(e.to_string()))?;
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
                    segment::assert_boundary(&auth, &b.delta, wires, &mut ctx)?;
                }

                let mut last_auth = None;
                if j == last {
                    match &chunk.trap {
                        Some(t) => {
                            if let Some(directive) = &t.directive {
                                replay::replay_trap(directive, &t.trap, &auth, &mut ctx)?;
                            }
                        }
                        None => {
                            if chunk.result_symbolic {
                                finalize::bind_output(&state, &mut ctx, &auth, output)?;
                            }
                        }
                    }
                    if reveal_pending {
                        reveal::reveal_verifier(&mut ctx, &auth, reveal_ranges, revealed)?;
                    }
                    last_auth = Some(auth.flatten());
                }

                let VerifierOutput { w, assertions, .. } = ctx
                    .finish()
                    .map_err(|e| ZkVmError::Internal(e.to_string()))?;
                Ok((w, assertions, last_auth))
            })
            .collect::<Result<_, _>>()?;

        let mut w = Gf2_128::new(0);
        let mut hasher = blake3::Hasher::new();
        let mut final_auth = None;
        for (w_j, h_j, last_auth) in results {
            w = w + w_j;
            hasher.update(&h_j);
            if let Some(auth) = last_auth {
                final_auth = Some(auth);
            }
        }

        Ok(VAccOut {
            w,
            assertions: *hasher.finalize().as_bytes(),
            auth: final_auth.expect("last segment produces final state"),
        })
    }
}

struct VAccOut {
    w: Gf2_128,
    assertions: [u8; 32],
    auth: AuthState,
}

impl<T> Vm for Verifier<T>
where
    T: RCOTSender<Block> + Flush,
{
    type Error = ZkVmError;

    fn write(&mut self, ptr: u32, w: Write<'_>) -> Result<(), ZkVmError> {
        let memory = self
            .global
            .memory_mut()
            .ok_or(ZkVmError::Core(CoreError::MemoryNotDefined))?;
        match w {
            Write::Public(data) => {
                memory.write_bytes(ptr, data).map_err(ZkVmError::Trap)?;
                self.global
                    .set_memory_visibility(ptr, data.len(), Visibility::Public);
            }
            Write::Blind(len) => {
                self.pending_io.write_private(ptr, len);
                self.global
                    .set_memory_visibility(ptr, len, Visibility::Blind);
            }
            Write::Private(_) => {
                return Err(ZkVmError::Unsupported(
                    "verifier cannot write private values".into(),
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

    #[tracing::instrument(level = "info", skip_all, fields(id = ?self.config.id(), role = "verifier", func = self.module.func_name(func_idx).unwrap_or("?")))]
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
            commit::prologue_delta(Role::Verifier, root_reg_base, &params, &self.pending_io)?;

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
        let mut final_output = None;
        let mut chunk_idx: usize = 0;
        let reveal_ranges: Vec<Range<u32>> = self.pending_reveal.iter().collect();
        let mut reveal_pending = !reveal_ranges.is_empty();
        let mut trapped: Option<mpz_vm_core::Trap> = None;
        let mut any_zk_work = false;

        loop {
            let chunk_span = tracing::info_span!("verifier.chunk", idx = chunk_idx);
            let _enter = chunk_span.enter();

            let outcome: ChunkOutcome = io
                .io_mut()
                .expect_next()
                .await
                .map_err(|e| ZkVmError::IoRecv(e.to_string()))?;
            if outcome.trap_at.is_some() != outcome.trap.is_some() {
                return Err(ZkVmError::Internal(
                    "trap outcome inconsistent: trap_at and trap must agree".into(),
                ));
            }
            self.reveal_state.merge(outcome.revealed.clone());

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
                Role::Verifier,
                outcome.trap_at.zip(outcome.trap.clone()),
                &mut self.reveal_state,
            )?;
            tracing::debug!(
                events = chunk.trace.len(),
                cost = chunk.cost,
                done = chunk.done,
                segments = chunk.segments.len(),
                "captured chunk"
            );

            let plan = segment::plan(&chunk, prologue.take());
            let log_end = plan.segments.last().expect("at least one segment").log.end;
            let execute_bits = plan.tape_len;

            let needs_zk = execute_bits > 0
                || any_zk_work
                || reveal_pending
                || chunk.trap.as_ref().is_some_and(|t| t.directive.is_some());
            if !needs_zk {
                self.global
                    .access_log_mut()
                    .expect("access log enabled")
                    .drain_upto(log_end);
                chunk_idx += 1;
                if let Some(point) = &chunk.trap {
                    if outcome.trap.as_ref() != Some(&point.trap) {
                        return Err(ZkVmError::Internal(
                            "announced trap reason does not match proven trap".into(),
                        ));
                    }
                    trapped = Some(point.trap.clone());
                    break;
                }
                if chunk.done {
                    final_output = chunk.result;
                    break;
                }
                continue;
            }
            any_zk_work = true;

            let total = execute_bits + VOPE_BITS;
            tracing::info!(
                cost = chunk.cost,
                total,
                segments = plan.segments.len(),
                "cost plan"
            );

            let keys = self.allocate(io, total).await?;
            let (exec_keys, vope_keys) = keys.split_at(execute_bits);
            let vope_keys: &[Gf2_128; VOPE_BITS] =
                vope_keys.try_into().expect("vope tail is VOPE_BITS wide");

            let (adjust, chi): (Vec<bool>, [u8; 32]) = async {
                let commitment: Commitment = io
                    .io_mut()
                    .expect_next()
                    .await
                    .map_err(|e| ZkVmError::IoRecv(e.to_string()))?;
                let adjust: Vec<bool> = commitment.adjust.iter().by_vals().collect();
                if adjust.len() != execute_bits {
                    return Err(ZkVmError::Internal(format!(
                        "commit adjust short: got {} want {}",
                        adjust.len(),
                        execute_bits
                    )));
                }
                let chi: [u8; 32] = rand::rng().random();
                io.io_mut()
                    .send(chi)
                    .await
                    .map_err(|e| ZkVmError::IoSend(e.to_string()))?;
                Ok((adjust, chi))
            }
            .instrument(tracing::debug_span!("exchange"))
            .await?;

            let ProofMessage {
                output,
                revealed,
                proof,
            } = io
                .io_mut()
                .expect_next()
                .await
                .map_err(|e| ZkVmError::IoRecv(e.to_string()))?;

            let out = self.accumulate_pass(
                &chunk,
                &plan,
                exec_keys,
                &adjust,
                chi,
                output,
                &revealed,
                &reveal_ranges,
                reveal_pending,
            )?;
            self.auth = out.auth;

            if out.assertions != proof.assertions {
                return Err(ZkVmError::BatchCheckFailed);
            }
            let b = vope_sender(vope_keys);
            if out.w + b != proof.u + self.delta * proof.v {
                return Err(ZkVmError::BatchCheckFailed);
            }

            if reveal_pending {
                let mut idx = 0;
                for r in &reveal_ranges {
                    let len = (r.end - r.start) as usize;
                    let memory = self
                        .global
                        .memory_mut()
                        .ok_or(ZkVmError::Core(CoreError::MemoryNotDefined))?;
                    memory
                        .write_bytes(r.start, &revealed[idx..idx + len])
                        .map_err(ZkVmError::Trap)?;
                    idx += len;
                    self.global
                        .set_memory_visibility(r.start, len, Visibility::Public);
                }
                reveal_pending = false;
            }
            self.global
                .access_log_mut()
                .expect("access log enabled")
                .drain_upto(log_end);
            chunk_idx += 1;
            if let Some(point) = &chunk.trap {
                if outcome.trap.as_ref() != Some(&point.trap) {
                    return Err(ZkVmError::Internal(
                        "announced trap reason does not match proven trap".into(),
                    ));
                }
                trapped = Some(point.trap.clone());
                break;
            }
            if chunk.done {
                final_output = if chunk.result_symbolic {
                    output
                } else {
                    chunk.result
                };
                break;
            }
        }

        self.pending_io.clear();
        self.pending_reveal = RangeSet::default();
        if let Some(trap) = trapped {
            tracing::info!(chunks = chunk_idx, ?trap, "verifier call trapped");
            return Err(ZkVmError::Trap(trap));
        }
        tracing::info!(chunks = chunk_idx, ?final_output, "verifier call complete");
        Ok(final_output)
    }

    #[tracing::instrument(level = "info", skip_all, fields(id = ?self.config.id(), role = "verifier"))]
    async fn commit(&mut self, io: &mut Context) -> Result<(), ZkVmError> {
        let prologue = commit::prologue_delta(Role::Verifier, Reg(0), &[], &self.pending_io)?;
        let reveal_ranges: Vec<Range<u32>> = self.pending_reveal.iter().collect();
        let reveal_pending = !reveal_ranges.is_empty();
        if prologue.is_none() && !reveal_pending {
            return Ok(());
        }

        let execute_bits = prologue.as_ref().map_or(0, |d| d.tape_len());
        let total = execute_bits + VOPE_BITS;
        let keys = self.allocate(io, total).await?;
        let (exec_keys, vope_keys) = keys.split_at(execute_bits);
        let vope_keys: &[Gf2_128; VOPE_BITS] =
            vope_keys.try_into().expect("vope tail is VOPE_BITS wide");

        let commitment: Commitment = io
            .io_mut()
            .expect_next()
            .await
            .map_err(|e| ZkVmError::IoRecv(e.to_string()))?;
        let adjust: Vec<bool> = commitment.adjust.iter().by_vals().collect();
        if adjust.len() != execute_bits {
            return Err(ZkVmError::Internal(format!(
                "commit adjust short: got {} want {}",
                adjust.len(),
                execute_bits
            )));
        }
        let chi: [u8; 32] = rand::rng().random();
        io.io_mut()
            .send(chi)
            .await
            .map_err(|e| ZkVmError::IoSend(e.to_string()))?;

        let ProofMessage {
            output: _,
            revealed,
            proof,
        } = io
            .io_mut()
            .expect_next()
            .await
            .map_err(|e| ZkVmError::IoRecv(e.to_string()))?;

        if let Some(delta) = &prologue {
            let pub_bit = |b: bool| if b { MAC_ONE + self.delta } else { MAC_ZERO };
            let wires: Vec<Gf2_128> = (0..execute_bits)
                .map(|i| verifier_wire(exec_keys[i], adjust[i], self.delta))
                .collect();
            segment::apply_delta(&mut self.auth, delta, &wires, &pub_bit)?;
        }

        let mut ctx = mpz_zk_core::Verifier::new(self.delta, &[], &[], ChaCha12Rng::from_seed(chi))
            .map_err(|e| ZkVmError::Internal(e.to_string()))?;
        if reveal_pending {
            reveal::reveal_verifier(&mut ctx, &self.auth, &reveal_ranges, &revealed)?;
        }
        let VerifierOutput { w, assertions, .. } = ctx
            .finish()
            .map_err(|e| ZkVmError::Internal(e.to_string()))?;

        if assertions != proof.assertions {
            return Err(ZkVmError::BatchCheckFailed);
        }
        let b = vope_sender(vope_keys);
        if w + b != proof.u + self.delta * proof.v {
            return Err(ZkVmError::BatchCheckFailed);
        }

        if reveal_pending {
            let mut idx = 0;
            for r in &reveal_ranges {
                let len = (r.end - r.start) as usize;
                let memory = self
                    .global
                    .memory_mut()
                    .ok_or(ZkVmError::Core(CoreError::MemoryNotDefined))?;
                memory
                    .write_bytes(r.start, &revealed[idx..idx + len])
                    .map_err(ZkVmError::Trap)?;
                idx += len;
                self.global
                    .set_memory_visibility(r.start, len, Visibility::Public);
            }
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
