//! Implementation of the Single-Point COT (spcot) protocol in the [`Ferret`](https://eprint.iacr.org/2020/924.pdf) paper.

pub mod error;
pub mod msgs;
pub mod receiver;
pub mod sender;

#[cfg(test)]
mod tests {
    use mpz_core::prg::Prg;

    use super::{receiver::Receiver as SpcotReceiver, sender::Sender as SpcotSender};
    use crate::ferret::{ideal_cot::IdealCOT, CSP};

    #[test]
    fn spcot_test() {
        let mut ideal_cot = IdealCOT::init();
        let sender = SpcotSender::new();
        let receiver = SpcotReceiver::new();

        let mut prg = Prg::new();
        let sender_seed = prg.random_block();
        let delta = ideal_cot.delta;
        let receiver_seed = prg.random_block();

        let mut sender = sender.setup(delta, sender_seed);
        let mut receiver = receiver.setup(receiver_seed);

        let h = 8;
        let alpha = 3;

        // Extend
        let (msg_for_sender, msg_for_receiver) = ideal_cot.extend(h);

        let maskbits = receiver
            .extend_mask_bits(h, alpha, msg_for_receiver.clone())
            .unwrap();

        let msg_from_sender = sender.extend(h, msg_for_sender, maskbits).unwrap();

        let _ = receiver
            .extend(h, alpha, msg_for_receiver, msg_from_sender)
            .unwrap();

        // Check
        let (msg_for_sender, msg_for_receiver) = ideal_cot.extend(CSP);

        let check_from_receiver = receiver
            .check_pre(h, alpha, msg_for_receiver.clone())
            .unwrap();

        let check = sender
            .check(h, msg_for_sender, check_from_receiver)
            .unwrap();

        let _ = receiver.check(msg_for_receiver, check).unwrap();

        sender.state.vs[alpha] ^= sender.state.delta;
        assert_eq!(sender.state.vs, receiver.state.ws);
    }
}
