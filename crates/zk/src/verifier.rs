use async_trait::async_trait;
use blake3::Hasher;
use mpz_common::{Context, Flush};
use mpz_core::{Block, bitvec::BitVec};
use mpz_ot::rcot::{RCOTSender, RCOTSenderOutput};
use mpz_vm_core::{
    Call, Callable, Execute, Result as VmResult, VmError,
    memory::{
        DecodeFuture, Memory, Repr, Slice, View,
        binary::Binary,
        correlated::{Delta, Key},
    },
};
use mpz_zk_core::{Verifier as Core, VerifierError, store::VerifierStore};
use serio::stream::IoStreamExt;

use crate::{callstack::CallStack, config::VerifierConfig};

#[derive(Debug)]
pub struct Verifier<OT> {
    config: VerifierConfig,
    core: Core,
    store: VerifierStore,
    ot: OT,
    callstack: CallStack,
    /// Gate OTs pre-registered by `call_raw` and not yet consumed by a chunk.
    registered_gates: usize,
    /// Consistency-check sVOLE pre-registered by `call_raw` and not yet
    /// consumed by a chunk.
    registered_checks: usize,
    transcript: Hasher,
}

impl<OT> Verifier<OT> {
    /// Creates a new verifier.
    pub fn new(config: VerifierConfig, delta: Delta, ot: OT) -> Self {
        Self {
            config,
            core: Core::new(delta),
            store: VerifierStore::new(delta),
            ot,
            callstack: CallStack::default(),
            registered_gates: 0,
            registered_checks: 0,
            transcript: Hasher::default(),
        }
    }

    /// Returns the global MAC correlation, `delta`.
    pub fn delta(&self) -> &Delta {
        self.store.delta()
    }

    /// Returns the keys.
    ///
    /// # Arguments
    ///
    /// * `value` - The value to return the keys for.
    pub fn get_keys<R>(&self, value: R) -> Result<&[Key], VerifierError>
    where
        R: Repr<Binary>,
    {
        let slice = value.to_raw();
        let keys = self.store.try_get_keys(slice)?;

        Ok(keys)
    }
}

#[async_trait]
impl<OT> Execute for Verifier<OT>
where
    OT: RCOTSender<Block> + Flush + Send + 'static,
{
    fn wants_flush(&self) -> bool {
        self.ot.wants_flush() || self.store.wants_keys() || self.store.wants_flush()
    }

    async fn flush(&mut self, ctx: &mut Context) -> VmResult<()> {
        if self.ot.wants_flush() {
            self.ot.flush(ctx).await.map_err(VmError::execute)?;
        }

        if self.store.wants_keys() {
            let RCOTSenderOutput { keys, .. } = self
                .ot
                .try_send_rcot(self.store.key_count())
                .map_err(VmError::execute)?;
            let keys = Key::from_blocks(keys);

            self.store.set_keys(&keys).map_err(VmError::memory)?;
        }

        while self.store.wants_flush() {
            self.store.mark_flush_pending().map_err(VmError::memory)?;
            let flush = ctx.io_mut().expect_next().await?;
            self.store
                .receive_flush(flush, &mut self.transcript)
                .map_err(VmError::memory)?;
        }

        Ok(())
    }

    fn wants_preprocess(&self) -> bool {
        false
    }

    async fn preprocess(&mut self, _ctx: &mut Context) -> VmResult<()> {
        Ok(())
    }

    fn wants_execute(&self) -> bool {
        self.callstack.iter().any(|(call, _)| {
            call.inputs()
                .iter()
                .all(|input| self.store.is_committed_raw(*input))
        })
    }

    async fn execute(&mut self, ctx: &mut Context) -> VmResult<()> {
        // Every `execute` closes out its own residual partial batch, so a
        // previous call can never leave pending consistency checks behind.
        debug_assert_eq!(
            self.core.pending(),
            0,
            "execute entered with pending consistency checks"
        );

        // Stage one chunk: the readiness-filtered prefix of the callstack
        // totalling at most `chunk_cap` AND gates. Registering only the chunk's
        // OT demand keeps materialization bounded by the chunk window rather
        // than the whole callstack. Checks still fire every `batch_size` gates
        // within the chunk.
        let chunk: Vec<(Call, Slice)> = self
            .callstack
            .extract_if(self.config.chunk_cap(), |call| {
                call.inputs()
                    .iter()
                    .all(|input| self.store.is_committed_raw(*input))
            })
            .collect();

        if chunk.is_empty() {
            return Ok(());
        }

        // Total gate OTs the chunk needs, and the consistency checks it will
        // trigger. The check count mirrors the check-batch grouping below:
        // batches split on call boundaries, so accumulate whole calls until
        // `batch_size` is crossed.
        let batch_size = self.config.batch_size();
        let mut gates = 0usize;
        let mut checks = 0usize;
        let mut pending = self.core.pending();
        for (call, _) in &chunk {
            let and_count = call.circ().and_count();
            gates += and_count;
            pending += and_count;
            // Calls aren't split, so one spanning `batch_size` checks once.
            if pending >= batch_size {
                checks += 1;
                pending = 0;
            }
        }

        // The chunk's trailing partial batch is checked before returning, so
        // it is part of the chunk's staged OT demand too.
        if pending > 0 {
            checks += 1;
        }

        // `call_raw` may have already reserved some of this chunk's gate OTs,
        // and over-provisioned its check sVOLE.
        let pre_registered_gates = self.registered_gates.min(gates);
        let pre_registered_checks = self.registered_checks.min(checks);
        let demand = (gates - pre_registered_gates) + (checks - pre_registered_checks) * 128;
        if demand > 0 {
            self.ot.alloc(demand).map_err(VmError::execute)?;
        }
        self.registered_gates -= pre_registered_gates;
        self.registered_checks -= pre_registered_checks;

        // Materialize the chunk's OT demand before consuming it.
        self.flush(ctx).await?;

        let mut calls = chunk.into_iter().peekable();

        // Execute the staged chunk in consistency-check batches. The chunk is
        // the OT materialization window; checks still fire every `batch_size`
        // gates.
        while calls.peek().is_some() {
            let limit = self.config.batch_size().saturating_sub(self.core.pending());

            // Batches split on call boundaries: take whole calls until the
            // batch size is crossed.
            let mut batch = Vec::new();
            let mut gates = 0usize;
            for (call, output) in calls.by_ref() {
                gates += call.circ().and_count();
                batch.push((call, output));
                if gates >= limit {
                    break;
                }
            }

            let ready_calls: Vec<_> = batch
                .into_iter()
                .map(|(call, output)| {
                    let input_keys = call
                        .inputs()
                        .iter()
                        .flat_map(|input| {
                            self.store.try_get_keys(*input).expect("keys should be set")
                        })
                        .copied()
                        .collect::<Vec<_>>();
                    let (circ, _) = call.into_parts();
                    (circ, input_keys, output)
                })
                .collect();

            let mut tasks = Vec::with_capacity(ready_calls.len());
            for (circ, input_keys, output) in ready_calls {
                let RCOTSenderOutput {
                    keys: gate_keys, ..
                } = self
                    .ot
                    .try_send_rcot(circ.and_count())
                    .map_err(VmError::execute)?;
                let gate_keys = Key::from_blocks(gate_keys);

                let execute = self
                    .core
                    .execute(circ, &input_keys, &gate_keys)
                    .map_err(VmError::execute)?;
                tasks.push((execute, output));
            }

            let outputs = ctx
                .map(tasks, move |ctx, (mut execute, output)| {
                    Box::pin(async move {
                        let mut consumer = execute.consumer();
                        while consumer.wants_adjust() {
                            let adjust: BitVec = ctx.io_mut().expect_next().await?;
                            for bit in adjust {
                                consumer.next(bit);
                            }
                        }

                        let output_keys = execute.finish().map_err(VmError::execute)?;

                        Ok((output, output_keys))
                    })
                })
                .await
                .map_err(VmError::execute)?
                .into_iter()
                .collect::<VmResult<Vec<_>>>()?;

            for (output, output_keys) in outputs {
                self.store
                    .set_output_keys(output, &output_keys)
                    .map_err(VmError::memory)?;
            }

            if self.core.pending() >= self.config.batch_size() {
                let RCOTSenderOutput {
                    keys: svole_keys, ..
                } = self.ot.try_send_rcot(128).map_err(VmError::execute)?;

                let uv = ctx.io_mut().expect_next().await?;
                self.core
                    .check(&mut self.transcript, &svole_keys, uv)
                    .map_err(VmError::execute)?;
            }
        }

        // Check this chunk's residual partial batch.
        if self.core.wants_check() {
            let RCOTSenderOutput {
                keys: svole_keys, ..
            } = self.ot.try_send_rcot(128).map_err(VmError::execute)?;

            let uv = ctx.io_mut().expect_next().await?;
            self.core
                .check(&mut self.transcript, &svole_keys, uv)
                .map_err(VmError::execute)?;
        }

        Ok(())
    }
}

impl<OT> Callable<Binary> for Verifier<OT>
where
    OT: RCOTSender<Block>,
{
    fn call_raw(&mut self, call: Call) -> VmResult<Slice> {
        let output = self.store.alloc_output(call.circ().outputs().len());

        // Pre-register gate OTs up to the chunk window while calls are
        // submitted (the offline phase), so they can be materialized early.
        // `execute` tops the window up during execution (online).
        let count = call.circ().and_count();
        if count > 0 && self.registered_gates.saturating_add(count) <= self.config.chunk_cap() {
            self.ot.alloc(count).map_err(VmError::execute)?;
            self.registered_gates += count;
        }

        // Register the consistency checks this call's addition triggers, in
        // submission order. Checks are cheap (128 sVOLE each), so register them
        // all up front rather than discovering them online.
        if count > 0 {
            let batch_size = self.config.batch_size();
            let partial_len = self.callstack.and_count() % batch_size;
            let mut checks = (partial_len + count - 1) / batch_size;
            if partial_len == 0 {
                checks += 1;
            }
            if checks > 0 {
                self.ot.alloc(checks * 128).map_err(VmError::execute)?;
                self.registered_checks += checks;
            }
        }

        self.callstack.push((call, output));

        Ok(output)
    }
}

impl<OT> Memory<Binary> for Verifier<OT>
where
    OT: RCOTSender<Block>,
{
    type Error = VmError;

    fn is_alloc_raw(&self, slice: Slice) -> bool {
        self.store.is_alloc_raw(slice)
    }

    fn alloc_raw(&mut self, size: usize) -> VmResult<Slice> {
        self.store.alloc_raw(size).map_err(VmError::memory)
    }

    fn is_assigned_raw(&self, slice: Slice) -> bool {
        self.store.is_assigned_raw(slice)
    }

    fn assign_raw(&mut self, slice: Slice, data: BitVec) -> VmResult<()> {
        self.store.assign_raw(slice, data).map_err(VmError::memory)
    }

    fn is_committed_raw(&self, slice: Slice) -> bool {
        self.store.is_committed_raw(slice)
    }

    fn commit_raw(&mut self, slice: Slice) -> VmResult<()> {
        self.store.commit_raw(slice).map_err(VmError::memory)
    }

    fn get_raw(&self, slice: Slice) -> VmResult<Option<BitVec>> {
        self.store.get_raw(slice).map_err(VmError::memory)
    }

    fn decode_raw(&mut self, slice: Slice) -> VmResult<DecodeFuture<BitVec>> {
        self.store.decode_raw(slice).map_err(VmError::memory)
    }
}

impl<OT> View<Binary> for Verifier<OT>
where
    OT: RCOTSender<Block>,
{
    type Error = VmError;

    fn mark_public_raw(&mut self, slice: Slice) -> VmResult<()> {
        self.store.mark_public_raw(slice).map_err(VmError::view)
    }

    fn mark_private_raw(&mut self, _slice: Slice) -> VmResult<()> {
        Err(VmError::view(
            "marking as private is not allowed for zk verifier",
        ))
    }

    fn mark_blind_raw(&mut self, slice: Slice) -> VmResult<()> {
        self.store.mark_blind_raw(slice).map_err(VmError::view)?;
        self.ot.alloc(slice.len()).map_err(VmError::view)?;

        Ok(())
    }
}
