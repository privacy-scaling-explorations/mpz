use std::{sync::Arc, marker::PhantomData};

use blake3::Hasher;

use crate::{
    circuit::EncryptedGate,
    encoding::{state, EncodedValue, Label}, EncryptedRow, mode::GarbleMode, Normal,
};
use mpz_circuits::{types::TypeError, Circuit, CircuitError, Gate};
use mpz_core::{
    aes::{FixedKeyAes, FIXED_KEY_AES},
    hash::Hash,
    Block,
};

/// Errors that can occur during garbled circuit evaluation.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum EvaluatorError {
    #[error(transparent)]
    TypeError(#[from] TypeError),
    #[error(transparent)]
    CircuitError(#[from] CircuitError),
    #[error("evaluator not finished")]
    NotFinished,
    #[error("invalid row count, must be a multiple of {0}")]
    InvalidRowCount(usize),
}

/// Evaluates half-gate garbled AND gate
#[inline]
pub(crate) fn and_gate(
    cipher: &FixedKeyAes,
    x: &Label,
    y: &Label,
    gid: usize,
    rows: &mut impl Iterator<Item=EncryptedRow>,
) -> Label {
    let x = x.to_inner();
    let y = y.to_inner();

    let s_a = x.lsb();
    let s_b = y.lsb();

    let j = Block::new((gid as u128).to_be_bytes());
    let k = Block::new(((gid + 1) as u128).to_be_bytes());

    let mut h = [x, y];
    cipher.tccr_many(&[j, k], &mut h);

    let [hx, hy] = h;

    let t_g = rows.next().expect("row should be present");
    let t_e = rows.next().expect("row should be present");

    let w_g = hx ^ (t_g.0 & Block::SELECT_MASK[s_a]);
    let w_e = hy ^ (Block::SELECT_MASK[s_b] & (t_e.0 ^ x));

    Label::new(w_g ^ w_e)
}

/// Evaluates half-gate privacy-free garbled AND gate
#[inline]
pub(crate) fn and_gate_pf(
    cipher: &FixedKeyAes,
    x: &Label,
    y: &Label,
    gid: usize,
    rows: &mut impl Iterator<Item=EncryptedRow>,
) -> Label {
    let x = x.to_inner();
    let y = y.to_inner();

    let s_a = x.lsb() != 0;

    let j = Block::new((gid as u128).to_be_bytes());

    let mut hx = cipher.tccr(j, x);

    let t_e = rows.next().expect("row should be present");

    let z = if s_a {
        hx.set_lsb();
        hx ^ t_e.0 ^ y
    } else {
        hx.clear_lsb();
        hx
    };

    Label::new(z)
}

/// Core evaluator type for evaluating a garbled circuit.
pub struct Evaluator<M: GarbleMode = Normal> {
    /// Cipher to use to encrypt the gates
    cipher: &'static FixedKeyAes,
    /// Circuit to evaluate
    circ: Arc<Circuit>,
    /// Active label state
    active_labels: Vec<Option<Label>>,
    /// Current position in the circuit
    pos: usize,
    /// Current gate id
    gid: usize,
    /// Hasher to use to hash the encrypted gates
    hasher: Option<Hasher>,
    _mode: PhantomData<M>
}

impl<M: GarbleMode> Evaluator<M> {
    /// Creates a new evaluator for the given circuit.
    ///
    /// # Arguments
    ///
    /// * `circ` - The circuit to evaluate.
    /// * `inputs` - The inputs to the circuit.
    pub fn new(
        circ: Arc<Circuit>,
        inputs: &[EncodedValue<state::Active>],
    ) -> Result<Self, EvaluatorError> {
        Self::new_with(circ, inputs, None)
    }

    /// Creates a new evaluator for the given circuit. Evaluator will compute
    /// a hash of the encrypted gates while they are evaluated.
    ///
    /// # Arguments
    ///
    /// * `circ` - The circuit to evaluate.
    /// * `inputs` - The inputs to the circuit.
    pub fn new_with_hasher(
        circ: Arc<Circuit>,
        inputs: &[EncodedValue<state::Active>],
    ) -> Result<Self, EvaluatorError> {
        Self::new_with(circ, inputs, Some(Hasher::new()))
    }

    fn new_with(
        circ: Arc<Circuit>,
        inputs: &[EncodedValue<state::Active>],
        hasher: Option<Hasher>,
    ) -> Result<Self, EvaluatorError> {
        if inputs.len() != circ.inputs().len() {
            return Err(CircuitError::InvalidInputCount(
                circ.inputs().len(),
                inputs.len(),
            ))?;
        }

        let mut active_labels: Vec<Option<Label>> = vec![None; circ.feed_count()];
        for (encoded, input) in inputs.iter().zip(circ.inputs()) {
            if encoded.value_type() != input.value_type() {
                return Err(TypeError::UnexpectedType {
                    expected: input.value_type(),
                    actual: encoded.value_type(),
                })?;
            }

            for (label, node) in encoded.iter().zip(input.iter()) {
                active_labels[node.id()] = Some(*label);
            }
        }

        let mut ev = Self {
            cipher: &(*FIXED_KEY_AES),
            circ,
            active_labels,
            pos: 0,
            gid: 1,
            hasher,
            _mode: PhantomData
        };

        // If circuit has no AND gates we can evaluate it immediately for cheap
        if ev.circ.and_count() == 0 {
            ev.evaluate(vec![])?;
        }

        Ok(ev)
    }

    /// Evaluates the next batch of encrypted rows.
    #[inline]
    pub fn evaluate(&mut self, mut rows: Vec<EncryptedRow>) -> Result<(), EvaluatorError> {
        let row_count = rows.len();
        let and_count = row_count / M::ROWS_PER_AND_GATE;

        if row_count % M::ROWS_PER_AND_GATE != 0 {
            return Err(EvaluatorError::InvalidRowCount(M::ROWS_PER_AND_GATE));
        }

        if let Some(hasher) = &mut self.hasher {
            for row in &rows {
                hasher.update(&row.0.to_bytes());
            }
        }

        let labels = &mut self.active_labels;
        let mut rows = rows.into_iter();
        let mut i = 0;
        // Process gates until we run out of encrypted gates
        for gate in &self.circ.gates()[self.pos..] {
            match gate {
                Gate::Inv {
                    x: node_x,
                    z: node_z,
                } => {
                    let x = labels[node_x.id()].expect("feed should be initialized");
                    labels[node_z.id()] = Some(x);
                }
                Gate::Xor {
                    x: node_x,
                    y: node_y,
                    z: node_z,
                } => {
                    let x = labels[node_x.id()].expect("feed should be initialized");
                    let y = labels[node_y.id()].expect("feed should be initialized");
                    labels[node_z.id()] = Some(x ^ y);
                }
                Gate::And {
                    x: node_x,
                    y: node_y,
                    z: node_z,
                } => {
                    if i >= and_count {
                        break;
                    }

                    let x = labels[node_x.id()].expect("feed should be initialized");
                    let y = labels[node_y.id()].expect("feed should be initialized");
                    let z = M::evaluate_and_gate(self.cipher, &x, &y,  self.gid, &mut rows);
                    labels[node_z.id()] = Some(z);
                    self.gid += 2;
                    i += 1;
                }
            }
            self.pos += 1;
        }

        Ok(())
    }

    /// Returns whether the evaluator has finished evaluating the circuit.
    pub fn is_complete(&self) -> bool {
        self.pos >= self.circ.gates().len()
    }

    /// Returns the active encoded outputs of the circuit.
    pub fn outputs(&self) -> Result<Vec<EncodedValue<state::Active>>, EvaluatorError> {
        if !self.is_complete() {
            return Err(EvaluatorError::NotFinished);
        }

        Ok(self
            .circ
            .outputs()
            .iter()
            .map(|output| {
                let labels: Vec<Label> = output
                    .iter()
                    .map(|node| self.active_labels[node.id()].expect("feed should be initialized"))
                    .collect();

                EncodedValue::<state::Active>::from_labels(output.value_type(), &labels)
                    .expect("encoding should be correct")
            })
            .collect())
    }

    /// Returns the hash of the encrypted gates.
    pub fn hash(&self) -> Option<Hash> {
        self.hasher.as_ref().map(|hasher| {
            let hash: [u8; 32] = hasher.finalize().into();
            Hash::from(hash)
        })
    }
}
