//! Provides implementations of random OLE with errors (ROLEe) protocols.

use std::sync::Arc;

use crate::{OLEError, RandomOLEeProvide};
use async_trait::async_trait;
use mpz_common::{sync::Mutex, Context};
use mpz_fields::Field;
use serde::{de::DeserializeOwned, Serialize};

pub mod rot;

// A shared ROLEe provider.
pub struct SharedROLEeProvider<T> {
    inner: Arc<Mutex<T>>,
}

impl<T> SharedROLEeProvider<T> {
    pub fn new(role_provider: T) -> Self {
        Self {
            inner: Arc::new(Mutex::new_follower(role_provider)),
        }
    }
}

#[async_trait]
impl<F: Field + Serialize + DeserializeOwned, C: Context, T: RandomOLEeProvide<C, F> + Send>
    RandomOLEeProvide<C, F> for SharedROLEeProvider<T>
where
    Self: Send,
{
    async fn provide_random(
        &mut self,
        ctx: &mut C,
        count: usize,
    ) -> Result<(Vec<F>, Vec<F>), OLEError> {
        let mut inner = self.inner.lock(ctx).await?;
        inner.provide_random(ctx, count).await
    }
}
