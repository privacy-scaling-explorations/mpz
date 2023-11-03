//! Define ideal functionality of SPCOT.

use mpz_core::{prg::Prg, Block};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// The message that sender receivers from the SPCOT functionality.
pub struct SpcotMsgForSender {
    /// The random blocks that sender receives from the SPCOT functionality.
    pub v: Vec<Vec<Block>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// The message that receiver receives from the SPCOT functionality.
pub struct SpcotMsgForReceiver {
    /// The random blocks that receiver receives from the SPCOT functionality.
    pub w: Vec<Vec<Block>>,
}

#[allow(missing_docs)]
pub struct IdealSpcot {
    pub delta: Block,
    pub counter: usize,
    pub prg: Prg,
    pub alphas: Vec<u32>,
}

impl IdealSpcot {
    /// Initiate the functionality.
    pub fn init() -> Self {
        let mut prg = Prg::new();
        let delta = prg.random_block();
        IdealSpcot {
            delta,
            counter: 0,
            prg,
            alphas: Vec::default(),
        }
    }

    /// Initiate with a given delta
    pub fn init_with_delta(delta: Block) -> Self {
        let prg = Prg::new();
        IdealSpcot {
            delta,
            counter: 0,
            prg,
            alphas: Vec::default(),
        }
    }

    /// Performs the batch extension of SPCOT.
    ///
    /// # Argument
    ///
    /// * `pos` - The positions in each extension.
    pub fn extend(&mut self, pos: &[(usize, u32)]) -> (SpcotMsgForSender, SpcotMsgForReceiver) {
        let mut v = vec![];
        let mut w = vec![];

        for (n, alpha) in pos {
            assert!((*alpha as usize) < *n);
            let mut v_tmp = vec![Block::ZERO; *n];
            self.prg.random_blocks(&mut v_tmp);
            let mut w_tmp = v_tmp.clone();
            w_tmp[*alpha as usize] ^= self.delta;

            v.push(v_tmp);
            w.push(w_tmp);
            self.alphas.push(*alpha);
            self.counter += n;
        }
        (SpcotMsgForSender { v }, SpcotMsgForReceiver { w })
    }

    /// Performs the checks.
    ///
    /// # Arguments
    ///
    /// * `sender_msg` - The message that the ideal SPCOT sends to the sender.
    /// * `receiver_msg` - The message that the ideal SPCOT sends to the receiver.
    pub fn check(
        &mut self,
        sender_msg: SpcotMsgForSender,
        receiver_msg: SpcotMsgForReceiver,
    ) -> bool {
        let SpcotMsgForSender { mut v } = sender_msg;
        let SpcotMsgForReceiver { w } = receiver_msg;

        let res = v
            .iter_mut()
            .zip(w.iter())
            .zip(self.alphas.iter())
            .all(|((vs, ws), alpha)| {
                vs[*alpha as usize] ^= self.delta;
                vs == ws
            });

        self.alphas.clear();

        res
    }
}

#[cfg(test)]
mod tests {
    use crate::ideal::ideal_spcot::IdealSpcot;

    #[test]
    fn ideal_spcot_test() {
        let mut ideal_spcot = IdealSpcot::init();

        let (sender_msg, receiver_msg) = ideal_spcot.extend(&[(10, 2), (20, 3)]);

        assert!(ideal_spcot.check(sender_msg, receiver_msg));
    }
}
