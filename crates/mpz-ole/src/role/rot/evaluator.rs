use crate::{msg::ROLEeMessage, Check, OLEError, RandomOLEeEvaluate};
use async_trait::async_trait;
use mpz_common::{sync::Mutex, Context};
use mpz_fields::Field;
use mpz_ole_core::role::ot::ROLEeEvaluator as ROLEeCoreEvaluator;
use mpz_ot::RandomOTReceiver;
use serde::{de::DeserializeOwned, Serialize};
use serio::{stream::IoStreamExt, SinkExt};
use std::fmt::Debug;
use std::{marker::PhantomData, sync::Arc};

/// An evaluator for ROLE with errors.
pub struct ROLEeEvaluator<const N: usize, T, F, C> {
    rot_receiver: T,
    role_core: ROLEeCoreEvaluator<N, F>,
    buffer: (Vec<F>, Vec<F>),
    context: PhantomData<C>,
}

impl<const N: usize, T, F, C> Debug for ROLEeEvaluator<N, T, F, C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ROLEeEvaluator {{ .. }}")
    }
}

impl<const N: usize, T, F: Field, C> ROLEeEvaluator<N, T, F, C> {
    /// Create a new [`ROLEeEvaluator`].
    pub fn new(rot_receiver: T) -> Self {
        // Check that the right N is used depending on the needed bit size of the field.
        let _: () = Check::<N, F>::IS_BITSIZE_CORRECT;

        Self {
            rot_receiver,
            role_core: ROLEeCoreEvaluator::default(),
            buffer: (vec![], vec![]),
            context: PhantomData,
        }
    }
}

impl<const N: usize, T, F: Field + Serialize + DeserializeOwned, C: Context>
    ROLEeEvaluator<N, T, F, C>
where
    T: RandomOTReceiver<C, bool, [u8; N]> + Send,
    Self: Send,
{
    /// Sets up `count` ROLEes.
    ///
    /// This allows to preprocess a number of ROLEes and buffer them.
    ///
    /// # Arguments
    ///
    /// * `ctx` - A context needed for IO.
    /// * `count` - The number of ROLEes to preprocess.
    pub async fn setup(&mut self, ctx: &mut C, count: usize) -> Result<(), OLEError> {
        let (factors, summands) = self.evaluate_random(ctx, count).await?;
        self.buffer.0.extend_from_slice(&factors);
        self.buffer.1.extend_from_slice(&summands);

        Ok(())
    }

    /// Returns `count` preprocessed ROLEes.
    ///
    /// This function returns buffered ROLEes where the setup has already been done.
    ///
    /// # Arguments
    ///
    /// * `count` - The number of ROLEes to return.
    pub fn take(&mut self, count: usize) -> Result<(Vec<F>, Vec<F>), OLEError> {
        if count > self.buffer.0.len() {
            return Err(OLEError::Preprocess);
        }

        let mut factors = self.buffer.0.split_off(count);
        let mut summands = self.buffer.1.split_off(count);

        // We want to consume in the same order, as they were created, hence we swap and return.
        std::mem::swap(&mut factors, &mut self.buffer.0);
        std::mem::swap(&mut summands, &mut self.buffer.1);

        Ok((factors, summands))
    }
}

#[async_trait]
impl<const N: usize, T, F: Field + Serialize + DeserializeOwned, C: Context>
    RandomOLEeEvaluate<C, F> for ROLEeEvaluator<N, T, F, C>
where
    T: RandomOTReceiver<C, bool, [u8; N]> + Send,
    Self: Send,
{
    async fn evaluate_random(
        &mut self,
        ctx: &mut C,
        count: usize,
    ) -> Result<(Vec<F>, Vec<F>), OLEError> {
        let (fi, tfi): (Vec<bool>, Vec<[u8; N]>) = self
            .rot_receiver
            .receive_random(ctx, count * F::BIT_SIZE as usize)
            .await?;

        let channel = ctx.io_mut();

        let (ui, ek): (Vec<F>, Vec<F>) = channel
            .expect_next::<ROLEeMessage<F>>()
            .await?
            .try_into_random_provider_msg()?;

        let dk: Vec<F> = self.role_core.sample_d(count);

        channel
            .send(ROLEeMessage::RandomEvaluatorMsg(dk.clone()))
            .await?;

        let (bk, yk) = self.role_core.generate_output(&fi, &tfi, &ui, &dk, &ek)?;

        Ok((bk, yk))
    }
}

/// A shared ROLEe evaluator.
#[derive(Debug, Clone)]
pub struct SharedROLEeEvaluator<const N: usize, T, F, C> {
    inner: Arc<Mutex<ROLEeEvaluator<N, T, F, C>>>,
}

impl<const N: usize, T, F, C> SharedROLEeEvaluator<N, T, F, C> {
    /// Creates a new shared instance as a leader.
    pub fn new_leader(role_evaluator: ROLEeEvaluator<N, T, F, C>) -> Self {
        Self {
            inner: Arc::new(Mutex::new_leader(role_evaluator)),
        }
    }

    /// Creates a new shared instance as a follower.
    pub fn new_follower(role_evaluator: ROLEeEvaluator<N, T, F, C>) -> Self {
        Self {
            inner: Arc::new(Mutex::new_follower(role_evaluator)),
        }
    }
}

#[async_trait]
impl<const N: usize, T, F: Field + Serialize + DeserializeOwned, C: Context>
    RandomOLEeEvaluate<C, F> for SharedROLEeEvaluator<N, T, F, C>
where
    T: RandomOTReceiver<C, bool, [u8; N]> + Send,
    Self: Send,
{
    async fn evaluate_random(
        &mut self,
        ctx: &mut C,
        count: usize,
    ) -> Result<(Vec<F>, Vec<F>), OLEError> {
        let mut inner = self.inner.lock(ctx).await?;
        inner.take(count)
    }
}
