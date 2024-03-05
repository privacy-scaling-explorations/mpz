//! A coin-toss protocol.

use mpz_core::{
    cointoss::{Receiver as CoreReceiver, Sender as CoreSender},
    Block,
};
use serio::{stream::IoStreamExt, SinkExt};

use crate::context::Context;

pub use mpz_core::cointoss::msgs;
pub use mpz_core::cointoss::{receiver_state, sender_state};

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CointossError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("core error: {0}")]
    Core(#[from] mpz_core::cointoss::CointossError),
}

#[derive(Debug)]
pub struct Sender<T: sender_state::State = sender_state::Initialized> {
    inner: CoreSender<T>,
}

impl Sender {
    /// Create a new sender.
    pub fn new(seeds: Vec<Block>) -> Self {
        Self {
            inner: CoreSender::new(seeds),
        }
    }

    /// Sends the coin-toss commitment.
    pub async fn commit(
        self,
        ctx: &mut impl Context,
    ) -> Result<Sender<sender_state::Committed>, CointossError> {
        let (inner, commitment) = self.inner.send();
        ctx.io_mut().send(commitment).await?;
        Ok(Sender { inner })
    }

    /// Executes the coin-toss protocol to completion.
    pub async fn execute(self, ctx: &mut impl Context) -> Result<Vec<Block>, CointossError> {
        let (seeds, sender) = self.commit(ctx).await?.receive(ctx).await?;
        sender.finalize(ctx).await?;
        Ok(seeds)
    }
}

impl Sender<sender_state::Committed> {
    /// Receives the receiver's payload and computes the output of the coin-toss.
    pub async fn receive(
        self,
        ctx: &mut impl Context,
    ) -> Result<(Vec<Block>, Sender<sender_state::Received>), CointossError> {
        let payload = ctx.io_mut().expect_next().await?;
        let (seeds, sender) = self.inner.receive(payload)?;
        Ok((seeds, Sender { inner: sender }))
    }
}

impl Sender<sender_state::Received> {
    /// Finalizes the coin-toss, decommitting the sender's seeds.
    pub async fn finalize(self, ctx: &mut impl Context) -> Result<(), CointossError> {
        ctx.io_mut().send(self.inner.finalize()).await?;
        Ok(())
    }
}

#[derive(Debug)]
pub struct Receiver<T: receiver_state::State = receiver_state::Initialized> {
    inner: CoreReceiver<T>,
}

impl Receiver {
    /// Create a new receiver.
    pub fn new(seeds: Vec<Block>) -> Self {
        Self {
            inner: CoreReceiver::new(seeds),
        }
    }

    /// Reveals the receiver's seeds after receiving the sender's commitment.
    pub async fn receive(
        self,
        ctx: &mut impl Context,
    ) -> Result<Receiver<receiver_state::Received>, CointossError> {
        let commitment = ctx.io_mut().expect_next().await?;
        let (inner, payload) = self.inner.reveal(commitment)?;
        ctx.io_mut().send(payload).await?;
        Ok(Receiver { inner })
    }

    /// Executes the coin-toss protocol to completion.
    pub async fn execute(self, ctx: &mut impl Context) -> Result<Vec<Block>, CointossError> {
        self.receive(ctx).await?.finalize(ctx).await
    }
}

impl Receiver<receiver_state::Received> {
    /// Finalizes the coin-toss, returning the random seeds.
    pub async fn finalize(self, ctx: &mut impl Context) -> Result<Vec<Block>, CointossError> {
        let payload = ctx.io_mut().expect_next().await?;
        let seeds = self.inner.finalize(payload)?;
        Ok(seeds)
    }
}

/// Executes the coin-toss protocol as the sender.
///
/// # Arguments
///
/// * `seeds` - The seeds to use for the coin-toss.
/// * `ctx` - The thread context.
pub async fn cointoss_sender(
    seeds: Vec<Block>,
    ctx: &mut impl Context,
) -> Result<Vec<Block>, CointossError> {
    Sender::new(seeds).execute(ctx).await
}

/// Executes the coin-toss protocol as the receiver.
///
/// # Arguments
///
/// * `seeds` - The seeds to use for the coin-toss.
/// * `ctx` - The thread context.
pub async fn cointoss_receiver(
    seeds: Vec<Block>,
    ctx: &mut impl Context,
) -> Result<Vec<Block>, CointossError> {
    Receiver::new(seeds).execute(ctx).await
}

#[cfg(test)]
mod tests {
    use super::*;

    use futures::executor::block_on;

    use crate::executor::test_st_executor;

    #[test]
    fn test_cointoss() {
        let (mut ctx_a, mut ctx_b) = test_st_executor(8);
        block_on(async {
            futures::try_join!(
                cointoss_sender(vec![Block::ZERO, Block::ONES], &mut ctx_a),
                cointoss_receiver(vec![Block::ONES, Block::ZERO], &mut ctx_b),
            )
            .unwrap()
        });
    }
}
