//! Implementation of the Single-Point COT (spcot) protocol in the [`Ferret`](https://eprint.iacr.org/2020/924.pdf) paper.

pub mod error;
pub mod msgs;
pub mod receiver;
pub mod sender;

#[cfg(test)]
mod tests {
    use mpz_core::prg::Prg;

    use super::{receiver::Receiver as SpcotReceiver, sender::Sender as SpcotSender};
    use crate::ferret::{
        ideal_cot::{CotMsgForReceiver, CotMsgForSender, IdealCOT},
        CSP,
    };

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

        let CotMsgForReceiver { rs, ts } = msg_for_receiver;
        let CotMsgForSender { qs } = msg_for_sender;
        let maskbits = receiver.extend_mask_bits(h, alpha, &rs).unwrap();

        let msg_from_sender = sender.extend(h, &qs, maskbits).unwrap();

        receiver.extend(h, alpha, &ts, msg_from_sender).unwrap();

        // Check
        let (msg_for_sender, msg_for_receiver) = ideal_cot.extend(CSP);

        let CotMsgForReceiver {
            rs: x_star,
            ts: z_star,
        } = msg_for_receiver;

        let CotMsgForSender { qs: y_star } = msg_for_sender;

        let check_from_receiver = receiver.check_pre(h, alpha, &x_star).unwrap();

        let (mut output_sender, check) = sender.check(h, &y_star, check_from_receiver).unwrap();

        let output_receiver = receiver.check(&z_star, check).unwrap();

        output_sender[alpha] ^= delta;
        assert_eq!(output_sender, output_receiver);
    }
}
