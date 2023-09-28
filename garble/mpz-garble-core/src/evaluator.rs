use blake3::Hasher;

use crate::{
    circuit::EncryptedGate,
    encoding::{state, EncodedValue, Label},
};
use mpz_circuits::{types::TypeError, CircuitError, CircuitIterator, Gate};
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
}

/// Evaluates half-gate garbled AND gate
#[inline]
pub(crate) fn and_gate(
    cipher: &FixedKeyAes,
    x: &Label,
    y: &Label,
    encrypted_gate: &EncryptedGate,
    gid: usize,
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

    let w_g = hx ^ (encrypted_gate[0] & Block::SELECT_MASK[s_a]);
    let w_e = hy ^ (Block::SELECT_MASK[s_b] & (encrypted_gate[1] ^ x));

    Label::new(w_g ^ w_e)
}

/// Core evaluator type for evaluating a garbled circuit.
pub struct Evaluator<'a> {
    /// Cipher to use to encrypt the gates
    cipher: &'static FixedKeyAes,
    /// An iterator over the gates of a circuit
    circuit_iterator: CircuitIterator<'a>,
    /// Active label state
    active_labels: Vec<Option<Label>>,
    /// Current gate id
    gid: usize,
    /// Whether the evaluator is finished
    complete: bool,
    /// Hasher to use to hash the encrypted gates
    hasher: Option<Hasher>,
}

impl<'a> Evaluator<'a> {
    /// Creates a new evaluator for the given circuit.
    ///
    /// # Arguments
    ///
    /// * `circ` - The circuit to evaluate.
    /// * `inputs` - The inputs to the circuit.
    pub fn new(
        circuit_iterator: CircuitIterator<'a>,
        inputs: &[EncodedValue<state::Active>],
    ) -> Result<Self, EvaluatorError> {
        Self::new_with(circuit_iterator, inputs, None)
    }

    /// Creates a new evaluator for the given circuit. Evaluator will compute
    /// a hash of the encrypted gates while they are evaluated.
    ///
    /// # Arguments
    ///
    /// * `circ` - The circuit to evaluate.
    /// * `inputs` - The inputs to the circuit.
    pub fn new_with_hasher(
        circuit_iterator: CircuitIterator<'a>,
        inputs: &[EncodedValue<state::Active>],
    ) -> Result<Self, EvaluatorError> {
        Self::new_with(circuit_iterator, inputs, Some(Hasher::new()))
    }

    fn new_with(
        circuit_iterator: CircuitIterator<'a>,
        inputs: &[EncodedValue<state::Active>],
        hasher: Option<Hasher>,
    ) -> Result<Self, EvaluatorError> {
        if inputs.len() != circuit_iterator.circuit().inputs().len() {
            return Err(CircuitError::InvalidInputCount(
                circuit_iterator.circuit().inputs().len(),
                inputs.len(),
            ))?;
        }

        let mut active_labels: Vec<Option<Label>> =
            vec![None; circuit_iterator.circuit().feed_count()];
        active_labels[0] = Some(Label::ONE);
        active_labels[1] = Some(Label::ONE);

        for (encoded, input) in inputs.iter().zip(circuit_iterator.circuit().inputs()) {
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
            circuit_iterator,
            active_labels,
            gid: 1,
            complete: false,
            hasher,
        };

        // If circuit has no AND gates we can evaluate it immediately for cheap
        if ev.circuit_iterator.circuit().and_count() == 0 {
            ev.evaluate(std::iter::empty());
        }

        Ok(ev)
    }

    /// Evaluates the next batch of encrypted gates.
    #[inline]
    pub fn evaluate<'b>(&mut self, mut encrypted_gates: impl Iterator<Item = &'b EncryptedGate>) {
        let labels = &mut self.active_labels;

        // Process gates until we run out of encrypted gates
        while let Some(gate) = self.circuit_iterator.by_ref().next() {
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
                    if let Some(encrypted_gate) = encrypted_gates.next() {
                        if let Some(hasher) = &mut self.hasher {
                            hasher.update(&encrypted_gate.to_bytes());
                        }

                        let x = labels[node_x.id()].expect("feed should be initialized");
                        let y = labels[node_y.id()].expect("feed should be initialized");
                        let z = and_gate(self.cipher, &x, &y, encrypted_gate, self.gid);
                        labels[node_z.id()] = Some(z);
                        self.gid += 2;
                    } else {
                        // TODO: This is a problem, because we discard the gate in this branch
                        // because .next() has already incremented

                        // We ran out of encrypted gates, so we return until we get more
                        return;
                    }
                }
            }
        }
        self.complete = true;
    }

    /// Returns whether the evaluator has finished evaluating the circuit.
    pub fn is_complete(&self) -> bool {
        self.complete
    }

    /// Returns the active encoded outputs of the circuit.
    pub fn outputs(&self) -> Result<Vec<EncodedValue<state::Active>>, EvaluatorError> {
        if !self.is_complete() {
            return Err(EvaluatorError::NotFinished);
        }

        Ok(self
            .circuit_iterator
            .circuit()
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
