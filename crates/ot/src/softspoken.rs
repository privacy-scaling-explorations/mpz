//! Correlated OT extension via SoftSpoken, run over a base OT.
//!
//! [`Sender`] and [`Receiver`] wrap the core protocol with the message I/O
//! needed to run it against a peer.

mod receiver;
mod sender;

pub use receiver::Receiver;
pub use sender::Sender;

pub use mpz_ot_core::softspoken::{
    ReceiverConfig, ReceiverConfigBuilder, ReceiverConfigBuilderError, SenderConfig,
    SenderConfigBuilder, SenderConfigBuilderError,
};

#[cfg(test)]
mod tests {
    use mpz_core::Block;
    use rand::{SeedableRng, rngs::StdRng};

    use super::*;

    use crate::{ideal::ot::ideal_ot, test::test_rcot};

    #[tokio::test]
    async fn test_softspoken_rcot() {
        let mut rng = StdRng::seed_from_u64(0);
        let (base_sender, base_receiver) = ideal_ot();
        let delta = Block::random(&mut rng);
        let sender = Sender::new(SenderConfig::default(), delta, base_receiver);
        let receiver = Receiver::new(ReceiverConfig::default(), base_sender);

        test_rcot(sender, receiver, 128, 1).await;
    }

    #[tokio::test]
    async fn test_softspoken_rcot_multiple_rounds() {
        let mut rng = StdRng::seed_from_u64(0);
        let (base_sender, base_receiver) = ideal_ot();
        let delta = Block::random(&mut rng);
        let sender = Sender::new(SenderConfig::default(), delta, base_receiver);
        let receiver = Receiver::new(ReceiverConfig::default(), base_sender);

        test_rcot(sender, receiver, 200, 4).await;
    }
}
