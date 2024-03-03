use std::pin::Pin;

use futures::Future;
use serio::{IoSink, IoStream};

use crate::ThreadId;

/// A thread context.
pub trait Context: Send {
    /// The type of I/O channel used by the thread.
    type Io: IoSink + IoStream + Send + Unpin + 'static;

    /// Returns the thread ID.
    fn id(&self) -> &ThreadId;

    /// Returns a mutable reference to the thread's I/O channel.
    fn io_mut(&mut self) -> &mut Self::Io;

    /// Maybe forks the thread and executes the provided closures concurrently.
    ///
    /// Implementations may not be able to fork, in which case the closures are executed
    /// sequentially.
    fn maybe_join<A, B, RA, RB>(&mut self, a: A, b: B) -> impl Future<Output = (RA, RB)> + Send
    where
        A: for<'a> FnOnce(&'a mut Self) -> Pin<Box<dyn Future<Output = RA> + Send + 'a>> + Send,
        B: for<'a> FnOnce(&'a mut Self) -> Pin<Box<dyn Future<Output = RB> + Send + 'a>> + Send,
        RA: Send,
        RB: Send;

    /// Maybe forks the thread and executes the provided closures concurrently, returning an error
    /// if one of the closures fails.
    ///
    /// This method is short circuiting, meaning that it returns as soon as one of the closures
    /// fails, potentially canceling the other.
    ///
    /// Implementations may not be able to fork, in which case the closures are executed
    /// sequentially.
    fn maybe_try_join<A, B, RA, RB, E>(
        &mut self,
        a: A,
        b: B,
    ) -> impl Future<Output = Result<(RA, RB), E>> + Send
    where
        A: for<'a> FnOnce(&'a mut Self) -> Pin<Box<dyn Future<Output = Result<RA, E>> + Send + 'a>>
            + Send,
        B: for<'a> FnOnce(&'a mut Self) -> Pin<Box<dyn Future<Output = Result<RB, E>> + Send + 'a>>
            + Send,
        RA: Send,
        RB: Send,
        E: Send;
}

/// A single-threaded context.
pub struct STContext<Io> {
    id: ThreadId,
    io: Io,
}

impl<Io> STContext<Io> {
    /// Creates a new single-threaded context.
    pub fn new(io: Io) -> Self {
        Self {
            id: ThreadId::default(),
            io,
        }
    }
}

impl<Io> Context for STContext<Io>
where
    Io: IoSink + IoStream + Send + Unpin + 'static,
{
    type Io = Io;

    fn id(&self) -> &ThreadId {
        &self.id
    }

    fn io_mut(&mut self) -> &mut Self::Io {
        &mut self.io
    }

    fn maybe_join<A, B, RA, RB>(&mut self, a: A, b: B) -> impl Future<Output = (RA, RB)> + Send
    where
        A: for<'a> FnOnce(&'a mut Self) -> Pin<Box<dyn Future<Output = RA> + Send + 'a>> + Send,
        B: for<'a> FnOnce(&'a mut Self) -> Pin<Box<dyn Future<Output = RB> + Send + 'a>> + Send,
        RA: Send,
        RB: Send,
    {
        async move {
            let a = a(self).await;
            let b = b(self).await;
            (a, b)
        }
    }

    fn maybe_try_join<A, B, RA, RB, E>(
        &mut self,
        a: A,
        b: B,
    ) -> impl Future<Output = Result<(RA, RB), E>> + Send
    where
        A: for<'a> FnOnce(&'a mut Self) -> Pin<Box<dyn Future<Output = Result<RA, E>> + Send + 'a>>
            + Send,
        B: for<'a> FnOnce(&'a mut Self) -> Pin<Box<dyn Future<Output = Result<RB, E>> + Send + 'a>>
            + Send,
        RA: Send,
        RB: Send,
        E: Send,
    {
        async move {
            let a = a(self).await?;
            let b = b(self).await?;
            Ok((a, b))
        }
    }
}

#[cfg(feature = "test-utils")]
pub mod test_utils {
    use serio::channel::{duplex, MemoryDuplex};

    use super::*;

    /// Creates a pair of single-threaded contexts with memory I/O channels.
    pub fn test_st_context(io_buffer: usize) -> (STContext<MemoryDuplex>, STContext<MemoryDuplex>) {
        let (io_0, io_1) = duplex(io_buffer);

        (STContext::new(io_0), STContext::new(io_1))
    }
}

#[cfg(feature = "test-utils")]
pub use test_utils::*;

#[cfg(test)]
mod tests {
    use futures::executor::block_on;
    use serio::channel::duplex;

    use super::*;

    #[test]
    fn test_st_context_maybe_join() {
        let (io, _) = duplex(1);

        let mut ctx = STContext::new(io);

        assert_eq!(
            block_on(ctx.maybe_join(
                |ctx| Box::pin(async { ctx.id().clone() }),
                |ctx| Box::pin(async { ctx.id().clone() }),
            )),
            (ThreadId::default(), ThreadId::default())
        );
    }

    #[test]
    fn test_st_context_maybe_try_join() {
        let (io, _) = duplex(1);

        let mut ctx = STContext::new(io);

        block_on(ctx.maybe_try_join(
            |ctx| Box::pin(async { Ok::<_, ()>(ctx.id().clone()) }),
            |_| Box::pin(async { Err::<ThreadId, ()>(()) }),
        ))
        .unwrap_err()
    }
}
