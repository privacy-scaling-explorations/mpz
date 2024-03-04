use std::sync::Arc;

use async_trait::async_trait;
use futures::Stream;
use mpz_common::{
    context::Context,
    sync::{Lock, Mutex, MutexBroker},
    ThreadId,
};
use mpz_core::Block;
use serio::{stream::IoStreamExt as _, SinkExt as _};

use crate::{
    kos::{Sender, SenderError},
    OTError, OTReceiver, OTSender,
};

/// A shared KOS sender.
#[derive(Debug, Clone)]
pub struct SharedSender<BaseOT> {
    inner: Arc<Mutex<Sender<BaseOT>>>,
}

impl<BaseOT> SharedSender<BaseOT> {
    /// Creates a new shared sender.
    pub fn new<St: Stream<Item = ThreadId>>(
        sender: Sender<BaseOT>,
        stream: St,
    ) -> (Self, MutexBroker<St>) {
        let (sender, broker) = Mutex::new_follower(sender, stream);
        (
            Self {
                // KOS sender is always the follower.
                inner: Arc::new(sender),
            },
            broker,
        )
    }

    /// Returns a future that resolves when a lock has been obtained.
    pub fn lock<'a>(&'a self, id: &'a ThreadId) -> Lock<'a, Sender<BaseOT>> {
        self.inner.lock(id)
    }
}

#[async_trait]
impl<Ctx, BaseOT> OTSender<Ctx, [Block; 2]> for SharedSender<BaseOT>
where
    Ctx: Context,
    BaseOT: OTReceiver<Ctx, bool, Block> + Send + 'static,
{
    async fn send(&mut self, ctx: &mut Ctx, msgs: &[[Block; 2]]) -> Result<(), OTError> {
        let mut keys = self
            .inner
            .lock(ctx.id())
            .await
            .unwrap()
            .take_keys(msgs.len())?;

        let derandomize = ctx.io_mut().expect_next().await?;

        keys.derandomize(derandomize).map_err(SenderError::from)?;
        let payload = keys.encrypt_blocks(msgs).map_err(SenderError::from)?;

        ctx.io_mut()
            .send(payload)
            .await
            .map_err(SenderError::from)?;

        Ok(())
    }
}
