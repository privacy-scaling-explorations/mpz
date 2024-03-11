use crate::{msg::ROLEeMessage, Check, OLEError, RandomOLEeProvide};
use async_trait::async_trait;
use mpz_common::{sync::Mutex, Context};
use mpz_fields::Field;
use mpz_ole_core::role::ot::ROLEeProvider as ROLEeCoreProvider;
use mpz_ot::RandomOTSender;
use serde::{de::DeserializeOwned, Serialize};
use serio::stream::IoStreamExt;
use serio::SinkExt;
use std::marker::PhantomData;
use std::sync::Arc;

/// A provider for ROLE with errors.
pub struct ROLEeProvider<const N: usize, T, F, C> {
    rot_sender: T,
    role_core: ROLEeCoreProvider<N, F>,
    buffer: (Vec<F>, Vec<F>),
    context: PhantomData<C>,
}

impl<const N: usize, T, F: Field, C> ROLEeProvider<N, T, F, C> {
    /// Creates a new [`ROLEeProvider`].
    pub fn new(rot_sender: T) -> Self {
        // Check that the right N is used depending on the needed bit size of the field.
        let _: () = Check::<N, F>::IS_BITSIZE_CORRECT;

        Self {
            rot_sender,
            role_core: ROLEeCoreProvider::default(),
            buffer: (vec![], vec![]),
            context: PhantomData,
        }
    }
}

impl<const N: usize, T, F: Field + Serialize + DeserializeOwned, C: Context>
    ROLEeProvider<N, T, F, C>
where
    T: RandomOTSender<C, [[u8; N]; 2]> + Send,
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
        let (factors, summands) = self.provide_random(ctx, count).await?;
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
impl<const N: usize, T, F: Field + Serialize + DeserializeOwned, C: Context> RandomOLEeProvide<C, F>
    for ROLEeProvider<N, T, F, C>
where
    T: RandomOTSender<C, [[u8; N]; 2]> + Send,
    Self: Send,
{
    async fn provide_random(
        &mut self,
        ctx: &mut C,
        count: usize,
    ) -> Result<(Vec<F>, Vec<F>), OLEError> {
        let ti01 = self
            .rot_sender
            .send_random(ctx, count * F::BIT_SIZE as usize)
            .await?;

        let (ck, ek) = self.role_core.sample_c_and_e(count);
        let (ui, t0i): (Vec<F>, Vec<F>) = self.role_core.create_correlation(&ti01, &ck)?;

        let channel = ctx.io_mut();

        channel
            .send(ROLEeMessage::RandomProviderMsg(ui, ek.clone()))
            .await?;

        let dk: Vec<F> = channel
            .expect_next::<ROLEeMessage<F>>()
            .await?
            .try_into_random_evaluator_msg()?;

        let (ak, xk) = self.role_core.generate_output(&t0i, &ck, &dk, &ek)?;

        Ok((ak, xk))
    }
}

/// A shared ROLEe provider.
pub struct SharedROLEeProvider<const N: usize, T, F, C> {
    inner: Arc<Mutex<ROLEeProvider<N, T, F, C>>>,
}

impl<const N: usize, T, F, C> SharedROLEeProvider<N, T, F, C> {
    /// Creates a new instance as a leader.
    pub fn new_leader(role_provider: ROLEeProvider<N, T, F, C>) -> Self {
        Self {
            inner: Arc::new(Mutex::new_leader(role_provider)),
        }
    }

    /// Creates a new instance as a follower.
    pub fn new_follower(role_provider: ROLEeProvider<N, T, F, C>) -> Self {
        Self {
            inner: Arc::new(Mutex::new_follower(role_provider)),
        }
    }
}

#[async_trait]
impl<const N: usize, T, F: Field + Serialize + DeserializeOwned, C: Context> RandomOLEeProvide<C, F>
    for SharedROLEeProvider<N, T, F, C>
where
    T: RandomOTSender<C, [[u8; N]; 2]> + Send,
    Self: Send,
{
    async fn provide_random(
        &mut self,
        ctx: &mut C,
        count: usize,
    ) -> Result<(Vec<F>, Vec<F>), OLEError> {
        let mut inner = self.inner.lock(ctx).await?;
        inner.take(count)
    }
}
