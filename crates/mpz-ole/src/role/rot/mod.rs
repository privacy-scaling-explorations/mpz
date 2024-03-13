//! Provides an implementation of ROLEe based on random OT.

mod evaluator;
mod provider;

pub use evaluator::{ROLEeEvaluator, SharedROLEeEvaluator};
pub use provider::{ROLEeProvider, SharedROLEeProvider};

#[cfg(test)]
mod tests {
    use super::{ROLEeEvaluator, ROLEeProvider};
    use crate::{
        role::rot::{SharedROLEeEvaluator, SharedROLEeProvider},
        RandomOLEeEvaluate, RandomOLEeProvide,
    };
    use mpz_common::executor::test_st_executor;
    use mpz_fields::p256::P256;
    use mpz_ot::ideal::rot::ideal_random_ot_pair;

    #[tokio::test]
    async fn test_role() {
        let count = 12;
        let (rot_sender, rot_receiver) = ideal_random_ot_pair::<[u8; 32]>([0; 32]);

        let (mut ctx_provider, mut ctx_evaluator) = test_st_executor(10);

        let mut role_provider = ROLEeProvider::<32, _, P256, _>::new(rot_sender);
        let mut role_evaluator = ROLEeEvaluator::<32, _, P256, _>::new(rot_receiver);

        let (provider_res, evaluator_res) = tokio::join!(
            role_provider.provide_random(&mut ctx_provider, count),
            role_evaluator.evaluate_random(&mut ctx_evaluator, count)
        );

        let (ak, xk) = provider_res.unwrap();
        let (bk, yk) = evaluator_res.unwrap();

        ak.iter()
            .zip(bk)
            .zip(xk)
            .zip(yk)
            .for_each(|(((&a, b), x), y)| assert_eq!(y, a * b + x));
    }

    #[tokio::test]
    async fn test_role_shared() {
        let count = 12;
        let (rot_sender, rot_receiver) = ideal_random_ot_pair::<[u8; 32]>([0; 32]);

        let (mut ctx_provider, mut ctx_evaluator) = test_st_executor(10);

        let mut role_provider = ROLEeProvider::<32, _, P256, _>::new(rot_sender);
        let mut role_evaluator = ROLEeEvaluator::<32, _, P256, _>::new(rot_receiver);

        let (provider_res, evaluator_res) = tokio::join!(
            role_provider.setup(&mut ctx_provider, count),
            role_evaluator.setup(&mut ctx_evaluator, count)
        );

        provider_res.unwrap();
        evaluator_res.unwrap();

        let mut role_provider_shared = SharedROLEeProvider::new_leader(role_provider);
        let mut role_evaluator_shared = SharedROLEeEvaluator::new_follower(role_evaluator);

        let (ak, xk) = role_provider_shared
            .provide_random(&mut ctx_provider, count)
            .await
            .unwrap();

        let (bk, yk) = role_evaluator_shared
            .evaluate_random(&mut ctx_evaluator, count)
            .await
            .unwrap();

        ak.iter()
            .zip(bk)
            .zip(xk)
            .zip(yk)
            .for_each(|(((&a, b), x), y)| assert_eq!(y, a * b + x));
    }
}
