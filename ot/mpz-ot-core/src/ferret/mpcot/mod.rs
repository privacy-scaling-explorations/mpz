//! Implementation of the Multiple-Point COT (mpcot) protocol in the [`Ferret`](https://eprint.iacr.org/2020/924.pdf) paper.

pub mod error;
pub mod msgs;
pub mod receiver;
pub mod receiver_regular;
pub mod sender;
pub mod sender_regular;

#[cfg(test)]
mod tests {
    use super::{
        receiver::Receiver as MpcotReceiver, receiver_regular::Receiver as RegularReceiver,
        sender::Sender as MpcotSender, sender_regular::Sender as RegularSender,
    };
    use crate::ideal::ideal_spcot::{IdealSpcot, SpcotMsgForReceiver, SpcotMsgForSender};
    use mpz_core::prg::Prg;

    #[test]
    fn mpcot_general_test() {
        let mut prg = Prg::new();
        let delta = prg.random_block();
        let mut ideal_spcot = IdealSpcot::new_with_delta(delta);

        let sender = MpcotSender::new();
        let receiver = MpcotReceiver::new();

        // receiver chooses hash and setup.
        let hash_seed = prg.random_block();
        let (mut receiver_pre, hash_seed) = receiver.setup(hash_seed);
        // sender receives the hash and setup.
        let mut sender_pre = sender.setup(delta, hash_seed);

        // extend once.
        let alphas = [0, 1, 3, 4, 2];
        let t = alphas.len();
        let n = 10;
        // sender generates the messages to invoke ideal spcot.
        let (mut sender, sender_queries) = sender_pre.pre_extend(t as u32, n).unwrap();

        let (mut receiver, mut queries) = receiver_pre.pre_extend(&alphas, n).unwrap();

        assert!(sender_queries
            .iter()
            .zip(queries.iter())
            .all(|(x, (y, _))| *x == *y));

        queries.iter_mut().for_each(|(x, _)| *x = 1 << (*x));

        let (sender_spcot_msg, receiver_spcot_msg) = ideal_spcot.extend(&queries);

        let SpcotMsgForSender { v: st } = sender_spcot_msg;
        let SpcotMsgForReceiver { w: rt } = receiver_spcot_msg;

        let mut output_sender = sender.extend(&st).unwrap();
        let output_receiver = receiver.extend(&rt).unwrap();

        for i in alphas {
            output_sender[i as usize] ^= delta;
        }

        assert_eq!(output_sender, output_receiver);

        // extend twice.
        let alphas = [5, 1, 7, 2];
        let t = alphas.len();
        let n = 16;
        // sender generates the messages to invoke ideal spcot.
        let (mut sender, sender_queries) = sender_pre.pre_extend(t as u32, n).unwrap();

        let (mut receiver, mut queries) = receiver_pre.pre_extend(&alphas, n).unwrap();

        assert!(sender_queries
            .iter()
            .zip(queries.iter())
            .all(|(x, (y, _))| *x == *y));

        queries.iter_mut().for_each(|(x, _)| *x = 1 << (*x));

        let (sender_spcot_msg, receiver_spcot_msg) = ideal_spcot.extend(&queries);

        let SpcotMsgForSender { v: st } = sender_spcot_msg;
        let SpcotMsgForReceiver { w: rt } = receiver_spcot_msg;

        let mut output_sender = sender.extend(&st).unwrap();
        let output_receiver = receiver.extend(&rt).unwrap();

        for i in alphas {
            output_sender[i as usize] ^= delta;
        }

        assert_eq!(output_sender, output_receiver);
    }

    #[test]
    fn mpcot_regular_test() {
        let mut prg = Prg::new();
        let delta = prg.random_block();
        let mut ideal_spcot = IdealSpcot::new_with_delta(delta);

        let sender = RegularSender::new();
        let receiver = RegularReceiver::new();

        let mut sender_pre = sender.setup(delta);
        let mut receiver_pre = receiver.setup();

        // extend once.
        let alphas = [0, 3, 4, 7, 9];
        let t = alphas.len();
        let n = 10;

        // sender generates the messages to invoke ideal spcot.
        let (mut sender, sender_queries) = sender_pre.pre_extend(t as u32, n).unwrap();
        let (mut receiver, mut queries) = receiver_pre.pre_extend(&alphas, n).unwrap();

        assert!(sender_queries
            .iter()
            .zip(queries.iter())
            .all(|(x, (y, _))| *x == *y));

        queries.iter_mut().for_each(|(x, _)| *x = 1 << (*x));

        let (sender_spcot_msg, receiver_spcot_msg) = ideal_spcot.extend(&queries);

        let SpcotMsgForSender { v: st } = sender_spcot_msg;
        let SpcotMsgForReceiver { w: rt } = receiver_spcot_msg;

        let mut output_sender = sender.extend(&st).unwrap();
        let output_receiver = receiver.extend(&rt).unwrap();

        for i in alphas {
            output_sender[i as usize] ^= delta;
        }

        assert_eq!(output_sender, output_receiver);

        // extend twice.
        let alphas = [0, 3, 7, 9, 14, 15];
        let t = alphas.len();
        let n = 16;

        // sender generates the messages to invoke ideal spcot.
        let (mut sender, sender_queries) = sender_pre.pre_extend(t as u32, n).unwrap();
        let (mut receiver, mut queries) = receiver_pre.pre_extend(&alphas, n).unwrap();

        assert!(sender_queries
            .iter()
            .zip(queries.iter())
            .all(|(x, (y, _))| *x == *y));

        queries.iter_mut().for_each(|(x, _)| *x = 1 << (*x));

        let (sender_spcot_msg, receiver_spcot_msg) = ideal_spcot.extend(&queries);

        let SpcotMsgForSender { v: st } = sender_spcot_msg;
        let SpcotMsgForReceiver { w: rt } = receiver_spcot_msg;

        let mut output_sender = sender.extend(&st).unwrap();
        let output_receiver = receiver.extend(&rt).unwrap();

        for i in alphas {
            output_sender[i as usize] ^= delta;
        }

        assert_eq!(output_sender, output_receiver);
    }
}
