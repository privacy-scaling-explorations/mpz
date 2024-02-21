//! Define ideal functionality of ROT with random choice bit.

use mpz_core::{prg::Prg, Block};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// The message that sender receives from the ROT functionality.
pub struct RotMsgForSender {
    /// The random blocks that sender receives from the ROT functionality.
    pub qs: Vec<[Block; 2]>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// The message that receiver receives from the ROT functionality.
pub struct RotMsgForReceiver {
    /// The random bits that receiver receives from the ROT functionality.
    pub rs: Vec<bool>,
    /// The chosen blocks that receiver receives from the ROT functionality.
    pub ts: Vec<Block>,
}
#[allow(missing_docs)]
#[derive(Debug)]
pub struct IdealROT {
    counter: usize,
    prg: Prg,
}

impl IdealROT {
    /// Initiate the functionality
    pub fn new() -> Self {
        let prg = Prg::new();
        IdealROT { counter: 0, prg }
    }

    /// Performs the extension with random choice bits.
    ///
    /// # Argument
    ///
    /// * `counter` - The number of ROT to extend.
    pub fn extend(&mut self, counter: usize) -> (RotMsgForSender, RotMsgForReceiver) {
        let mut qs1 = vec![Block::ZERO; counter];
        let mut qs2 = vec![Block::ZERO; counter];

        self.prg.random_blocks(&mut qs1);
        self.prg.random_blocks(&mut qs2);

        let qs: Vec<[Block; 2]> = qs1.iter().zip(qs2).map(|(&q1, q2)| [q1, q2]).collect();

        let mut rs = vec![false; counter];

        self.prg.random_bools(&mut rs);

        let ts: Vec<Block> = qs
            .iter()
            .zip(rs.iter())
            .map(|(&q, &r)| q[r as usize])
            .collect();

        self.counter += counter;
        (RotMsgForSender { qs }, RotMsgForReceiver { rs, ts })
    }

    /// Checks if the receiver gets the choices he made
    ///
    /// # Arguments
    ///
    /// * `sender_msg` - The message that the ideal ROT sends to the sender.
    /// * `receiver_msg` - The message that the ideal ROT sends to the receiver.
    pub fn check(&self, sender_msg: RotMsgForSender, receiver_msg: RotMsgForReceiver) -> bool {
        let RotMsgForSender { qs } = sender_msg;
        let RotMsgForReceiver { rs, ts } = receiver_msg;

        qs.into_iter()
            .zip(ts)
            .zip(rs)
            .all(|((q, t), r)| if r { q[1] == t } else { q[0] == t })
    }
}

impl Default for IdealROT {
    fn default() -> Self {
        Self::new()
    }
}
#[cfg(test)]
mod tests {
    use super::IdealROT;

    #[test]
    fn ideal_rot_test() {
        let num = 100;
        let mut ideal_rot = IdealROT::new();
        let (sender, receiver) = ideal_rot.extend(num);

        assert!(ideal_rot.check(sender, receiver));
    }
}
