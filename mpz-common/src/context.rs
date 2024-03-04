use async_trait::async_trait;

use scoped_futures::ScopedBoxFuture;
use serio::{IoSink, IoStream};

use crate::ThreadId;

/// A thread context.
#[async_trait]
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
    async fn maybe_join<'a, A, B, RA, RB>(&'a mut self, a: A, b: B) -> (RA, RB)
    where
        A: for<'b> FnOnce(&'b mut Self) -> ScopedBoxFuture<'a, 'b, RA> + Send + 'a,
        B: for<'b> FnOnce(&'b mut Self) -> ScopedBoxFuture<'a, 'b, RB> + Send + 'a,
        RA: Send + 'a,
        RB: Send + 'a;

    /// Maybe forks the thread and executes the provided closures concurrently, returning an error
    /// if one of the closures fails.
    ///
    /// This method is short circuiting, meaning that it returns as soon as one of the closures
    /// fails, potentially canceling the other.
    ///
    /// Implementations may not be able to fork, in which case the closures are executed
    /// sequentially.
    async fn maybe_try_join<'a, A, B, RA, RB, E>(&'a mut self, a: A, b: B) -> Result<(RA, RB), E>
    where
        A: for<'b> FnOnce(&'b mut Self) -> ScopedBoxFuture<'a, 'b, Result<RA, E>> + Send + 'a,
        B: for<'b> FnOnce(&'b mut Self) -> ScopedBoxFuture<'a, 'b, Result<RB, E>> + Send + 'a,
        RA: Send + 'a,
        RB: Send + 'a,
        E: Send + 'a;
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

#[async_trait]
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

    async fn maybe_join<'a, A, B, RA, RB>(&'a mut self, a: A, b: B) -> (RA, RB)
    where
        A: for<'b> FnOnce(&'b mut Self) -> ScopedBoxFuture<'a, 'b, RA> + Send + 'a,
        B: for<'b> FnOnce(&'b mut Self) -> ScopedBoxFuture<'a, 'b, RB> + Send + 'a,
        RA: Send + 'a,
        RB: Send + 'a,
    {
        let a = a(self).await;
        let b = b(self).await;
        (a, b)
    }

    async fn maybe_try_join<'a, A, B, RA, RB, E>(&'a mut self, a: A, b: B) -> Result<(RA, RB), E>
    where
        A: for<'b> FnOnce(&'b mut Self) -> ScopedBoxFuture<'a, 'b, Result<RA, E>> + Send + 'a,
        B: for<'b> FnOnce(&'b mut Self) -> ScopedBoxFuture<'a, 'b, Result<RB, E>> + Send + 'a,
        RA: Send + 'a,
        RB: Send + 'a,
        E: Send + 'a,
    {
        let a = a(self).await?;
        let b = b(self).await?;
        Ok((a, b))
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
    use scoped_futures::ScopedFutureExt;
    use serio::channel::duplex;

    use super::*;

    #[derive(Debug)]
    struct Test {
        a: (),
        b: (),
    }

    impl Test {
        async fn foo<Ctx: Context>(&mut self, ctx: &mut Ctx) {
            let a = &mut self.a;
            let b = &mut self.b;
            ctx.maybe_join(
                |ctx| async move { println!("{:?}:{:?}", ctx.id(), a) }.scope_boxed(),
                |ctx| async move { println!("{:?}:{:?}", ctx.id(), b) }.scope_boxed(),
            )
            .await;

            println!("{:?}", self);
        }
    }

    #[test]
    fn test_st_context_maybe_join() {
        let (io, _) = duplex(1);

        let mut ctx = STContext::new(io);

        let mut test = Test { a: (), b: () };

        block_on(test.foo(&mut ctx));
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
