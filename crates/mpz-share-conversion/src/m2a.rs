use crate::{Evaluate, Provide, Role, ShareConversionError};
use async_trait::async_trait;
use mpz_common::Context;
use mpz_fields::Field;
use mpz_ole::{OLEeEvaluate, OLEeProvide};

#[async_trait]
pub trait M2A<C: Context, F: Field, R: Role> {
    async fn convert(
        &mut self,
        ctx: &mut C,
        shares: Vec<F>,
    ) -> Result<Vec<F>, ShareConversionError>;
}

#[async_trait]
impl<C: Context, F: Field, T: OLEeProvide<C, F> + Send> M2A<C, F, Provide> for T {
    async fn convert(
        &mut self,
        ctx: &mut C,
        mul_shares: Vec<F>,
    ) -> Result<Vec<F>, ShareConversionError> {
        let mut add_shares = self
            .provide(ctx, mul_shares)
            .await
            .map_err(ShareConversionError::from)?;

        add_shares.iter_mut().for_each(|share| *share = -*share);
        Ok(add_shares)
    }
}

#[async_trait]
impl<C: Context, F: Field, T: OLEeEvaluate<C, F> + Send> M2A<C, F, Evaluate> for T {
    async fn convert(
        &mut self,
        ctx: &mut C,
        mul_shares: Vec<F>,
    ) -> Result<Vec<F>, ShareConversionError> {
        self.evaluate(ctx, mul_shares)
            .await
            .map_err(ShareConversionError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::M2A;
    use mpz_common::executor::test_st_executor;
    use mpz_core::{prg::Prg, Block};
    use mpz_fields::{p256::P256, UniformRand};
    use mpz_ole::ideal::ole::ideal_ole_pair;
    use rand::SeedableRng;

    #[tokio::test]
    async fn test_a2m() {
        let count = 12;
        let from_seed = Prg::from_seed(Block::ZERO);
        let mut rng = from_seed;

        let mul_shares_alice: Vec<P256> = (0..count).map(|_| P256::rand(&mut rng)).collect();
        let mul_shares_bob: Vec<P256> = (0..count).map(|_| P256::rand(&mut rng)).collect();

        let (mut alice, mut bob) = ideal_ole_pair::<P256>();

        let (mut ctx_provider, mut ctx_evaluator) = test_st_executor(10);

        let add_shares_alice = alice
            .convert(&mut ctx_provider, mul_shares_alice.clone())
            .await
            .unwrap();
        let add_shares_bob = bob
            .convert(&mut ctx_evaluator, mul_shares_bob.clone())
            .await
            .unwrap();

        mul_shares_alice
            .iter()
            .zip(mul_shares_bob)
            .zip(add_shares_alice)
            .zip(add_shares_bob)
            .for_each(|(((&a, b), x), y)| assert_eq!(x + y, a * b));
    }
}
