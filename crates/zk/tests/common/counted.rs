//! Test-only RCOT wrappers which count allocations and consumptions.
//!
//! Wrapping keeps the counters out of the OT implementations and lets the
//! counters be read after the OT has been moved into a VM (via a cloneable
//! [`Counters`] handle).

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use mpz_common::{Context, Flush};
use mpz_core::Block;
use mpz_ot::rcot::{RCOTReceiver, RCOTReceiverOutput, RCOTSender, RCOTSenderOutput};

/// Cloneable handle to a wrapper's cumulative OT counters.
#[derive(Debug, Clone, Default)]
pub struct Counters {
    allocated: Arc<AtomicUsize>,
    consumed: Arc<AtomicUsize>,
}

impl Counters {
    /// Returns the total number of OTs allocated so far.
    pub fn allocated(&self) -> usize {
        self.allocated.load(Ordering::Relaxed)
    }

    /// Returns the total number of OTs consumed so far.
    pub fn consumed(&self) -> usize {
        self.consumed.load(Ordering::Relaxed)
    }

    fn add_allocated(&self, count: usize) {
        self.allocated.fetch_add(count, Ordering::Relaxed);
    }

    fn add_consumed(&self, count: usize) {
        self.consumed.fetch_add(count, Ordering::Relaxed);
    }
}

/// Wraps an RCOT receiver, counting allocations and consumptions.
#[derive(Debug)]
pub struct CountedReceiver<T> {
    inner: T,
    counters: Counters,
}

impl<T> CountedReceiver<T> {
    /// Creates a new counting receiver wrapping `inner`.
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            counters: Counters::default(),
        }
    }

    /// Returns a shared handle to this wrapper's counters.
    pub fn counters(&self) -> Counters {
        self.counters.clone()
    }
}

impl<T> RCOTReceiver<bool, Block> for CountedReceiver<T>
where
    T: RCOTReceiver<bool, Block>,
{
    type Error = T::Error;
    type Future = T::Future;

    fn alloc(&mut self, count: usize) -> Result<(), Self::Error> {
        self.inner.alloc(count)?;
        self.counters.add_allocated(count);
        Ok(())
    }

    fn available(&self) -> usize {
        self.inner.available()
    }

    fn try_recv_rcot(
        &mut self,
        count: usize,
    ) -> Result<RCOTReceiverOutput<bool, Block>, Self::Error> {
        let output = self.inner.try_recv_rcot(count)?;
        self.counters.add_consumed(count);
        Ok(output)
    }

    fn queue_recv_rcot(&mut self, count: usize) -> Result<Self::Future, Self::Error> {
        self.inner.queue_recv_rcot(count)
    }
}

#[async_trait]
impl<T> Flush for CountedReceiver<T>
where
    T: Flush + Send,
{
    type Error = T::Error;

    fn wants_flush(&self) -> bool {
        self.inner.wants_flush()
    }

    async fn flush(&mut self, ctx: &mut Context) -> Result<(), Self::Error> {
        self.inner.flush(ctx).await
    }
}

/// Wraps an RCOT sender, counting allocations and consumptions.
#[derive(Debug)]
pub struct CountedSender<T> {
    inner: T,
    counters: Counters,
}

impl<T> CountedSender<T> {
    /// Creates a new counting sender wrapping `inner`.
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            counters: Counters::default(),
        }
    }

    /// Returns a shared handle to this wrapper's counters.
    pub fn counters(&self) -> Counters {
        self.counters.clone()
    }
}

impl<T> RCOTSender<Block> for CountedSender<T>
where
    T: RCOTSender<Block>,
{
    type Error = T::Error;
    type Future = T::Future;

    fn alloc(&mut self, count: usize) -> Result<(), Self::Error> {
        self.inner.alloc(count)?;
        self.counters.add_allocated(count);
        Ok(())
    }

    fn available(&self) -> usize {
        self.inner.available()
    }

    fn delta(&self) -> Block {
        self.inner.delta()
    }

    fn try_send_rcot(&mut self, count: usize) -> Result<RCOTSenderOutput<Block>, Self::Error> {
        let output = self.inner.try_send_rcot(count)?;
        self.counters.add_consumed(count);
        Ok(output)
    }

    fn queue_send_rcot(&mut self, count: usize) -> Result<Self::Future, Self::Error> {
        self.inner.queue_send_rcot(count)
    }
}

#[async_trait]
impl<T> Flush for CountedSender<T>
where
    T: Flush + Send,
{
    type Error = T::Error;

    fn wants_flush(&self) -> bool {
        self.inner.wants_flush()
    }

    async fn flush(&mut self, ctx: &mut Context) -> Result<(), Self::Error> {
        self.inner.flush(ctx).await
    }
}
