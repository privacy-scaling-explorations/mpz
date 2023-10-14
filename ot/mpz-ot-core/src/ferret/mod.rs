//! An implementation of the [`Ferret`](https://eprint.iacr.org/2020/924.pdf) protocol.

pub mod spcot;

/// Computational security parameter
pub const CSP: usize = 128;

pub mod ideal_cot {
    //! Ideal functionality of COT.
    use mpz_core::{prg::Prg, Block};

    use super::spcot::msgs::{CotMsgForReceiver, CotMsgForSender};

    #[allow(missing_docs)]
    pub struct IdealCOT {
        pub delta: Block,
        pub counter: usize,
        pub prg: Prg,
    }

    impl IdealCOT {
        /// Initiate the functionality
        pub fn init() -> Self {
            let mut prg = Prg::new();
            let delta = prg.random_block();
            IdealCOT {
                delta,
                counter: 0,
                prg,
            }
        }

        /// Performs the extension with random choice bits.
        ///
        /// # Argument
        ///
        /// * `counter` - The number of COT to extend.
        pub fn extend(&mut self, counter: usize) -> (CotMsgForSender, CotMsgForReceiver) {
            let mut qs = vec![Block::ZERO; counter];
            let mut rs = vec![false; counter];

            self.prg.random_blocks(&mut qs);
            self.prg.random_bools(&mut rs);

            let ts: Vec<Block> = qs
                .iter()
                .zip(rs.iter())
                .map(|(&q, &r)| if r { q ^ self.delta } else { q })
                .collect();

            self.counter += counter;
            (CotMsgForSender { qs }, CotMsgForReceiver { rs, ts })
        }

        /// Perform the checks.
        ///
        /// # Arguments
        ///
        /// `sender_msg` - The message that the ideal COT sends to the sender.
        /// `receiver_msg` - The message that the ideal COT sends to the receiver.
        pub fn check(self, sender_msg: CotMsgForSender, receiver_msg: CotMsgForReceiver) -> bool {
            let CotMsgForSender { qs } = sender_msg;
            let CotMsgForReceiver { rs, ts } = receiver_msg;

            qs.into_iter().zip(ts).zip(rs).all(
                |((q, t), r)| {
                    if !r {
                        q == t
                    } else {
                        q == t ^ self.delta
                    }
                },
            )
        }
    }

    #[test]
    fn ideal_cot_test() {
        let num = 100;
        let mut ideal_cot = IdealCOT::init();
        let (sender, receiver) = ideal_cot.extend(num);

        assert!(ideal_cot.check(sender, receiver));
    }
}
