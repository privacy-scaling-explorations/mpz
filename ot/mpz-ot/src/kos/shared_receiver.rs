use std::sync::Arc;

use async_trait::async_trait;
use futures::{channel::mpsc::channel, Stream};
use itybity::IntoBitIterator;
use mpz_common::{
    context::Context,
    sync::{Lock, Mutex},
    ThreadId,
};
use mpz_core::Block;
use serio::{stream::IoStreamExt, SinkExt};
use utils_aio::non_blocking_backend::{Backend, NonBlockingBackend};

use crate::{
    kos::{Receiver, ReceiverError},
    OTError, OTReceiver,
};

/// A shared KOS receiver.
#[derive(Debug, Clone)]
pub struct SharedReceiver<BaseOT> {
    inner: Arc<Mutex<Receiver<BaseOT>>>,
}

impl<BaseOT> SharedReceiver<BaseOT> {
    /// Creates a new shared receiver.
    pub fn new(receiver: Receiver<BaseOT>) -> (Self, impl Stream<Item = ThreadId>) {
        let (sender, receiver_channel) = channel(128);
        (
            Self {
                inner: Arc::new(Mutex::new_leader(receiver, sender)),
            },
            receiver_channel,
        )
    }

    /// Returns a future that resolves when a lock has been obtained.
    pub fn lock<'a>(&'a self, id: &'a ThreadId) -> Lock<'a, Receiver<BaseOT>> {
        self.inner.lock(id)
    }
}

#[async_trait]
impl<Ctx, BaseOT> OTReceiver<Ctx, bool, Block> for SharedReceiver<BaseOT>
where
    Ctx: Context,
    BaseOT: Send,
{
    async fn receive(&mut self, ctx: &mut Ctx, choices: &[bool]) -> Result<Vec<Block>, OTError> {
        let mut keys = self
            .inner
            .lock(ctx.id())
            .await
            .unwrap()
            .take_keys(choices.len())?;

        let choices = choices.into_lsb0_vec();
        let derandomize = keys.derandomize(&choices).map_err(ReceiverError::from)?;

        // Send derandomize message
        ctx.io_mut().send(derandomize).await?;

        // Receive payload
        let payload = ctx.io_mut().expect_next().await?;

        let received =
            Backend::spawn(move || keys.decrypt_blocks(payload).map_err(ReceiverError::from))
                .await?;

        Ok(received)
    }
}
