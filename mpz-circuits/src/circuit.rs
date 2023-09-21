use itybity::IntoBits;

use crate::{
    components::Gate,
    types::{BinaryRepr, TypeError, Value},
};
use std::sync::Arc;

/// An error that can occur when performing operations with a circuit.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum CircuitError {
    #[error("Invalid number of inputs: expected {0}, got {1}")]
    InvalidInputCount(usize, usize),
    #[error("Invalid number of outputs: expected {0}, got {1}")]
    InvalidOutputCount(usize, usize),
    #[error(transparent)]
    TypeError(#[from] TypeError),
}

/// A binary circuit.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Circuit {
    pub(crate) inputs: Vec<BinaryRepr>,
    pub(crate) outputs: Vec<BinaryRepr>,
    pub(crate) gates: Vec<Gate>,
    pub(crate) feed_count: usize,
    pub(crate) and_count: usize,
    pub(crate) xor_count: usize,
    pub(crate) constant_inputs: Vec<Value>,
    pub(crate) appended_circuits: Vec<Arc<Circuit>>,
    pub(crate) appended_circuits_input_feeds: Vec<Vec<BinaryRepr>>,
    pub(crate) gates_count: usize,
}

impl Circuit {
    /// Returns a reference to the inputs of the circuit.
    pub fn inputs(&self) -> &[BinaryRepr] {
        &self.inputs
    }

    /// Returns a reference to the outputs of the circuit.
    pub fn outputs(&self) -> &[BinaryRepr] {
        &self.outputs
    }

    /// Returns a reference to the gates of the circuit.
    pub fn gates(&self) -> Box<dyn Iterator<Item = Gate> + '_> {
        let mut feeds_so_far = 0;

        let iter = self
            .appended_circuits
            .iter()
            .enumerate()
            .flat_map(move |(k, circ)| {
                let gates_iter = if self.appended_circuits_input_feeds[k].is_empty() {
                    circ.gates()
                } else {
                    let old_inputs = circ
                        .inputs
                        .iter()
                        .flat_map(|bin| bin.iter())
                        .collect::<Vec<_>>();
                    let new_inputs = self.appended_circuits_input_feeds[k]
                        .iter()
                        .flat_map(|bin| bin.iter())
                        .collect::<Vec<_>>();

                    let iter = circ.gates().map(move |mut gate| {
                        gate.shift_right(feeds_so_far);

                        let x = gate.x();
                        let y = gate.y();

                        if let Some(pos) = old_inputs
                            .iter()
                            .position(|node| node.id + feeds_so_far == x.id)
                        {
                            gate.set_x(new_inputs[pos].id);
                        }

                        if let Some(y) = y {
                            if let Some(pos) = old_inputs
                                .iter()
                                .position(|node| node.id + feeds_so_far == y.id)
                            {
                                gate.set_y(new_inputs[pos].id);
                            }
                        }

                        gate
                    });
                    Box::new(iter)
                };

                feeds_so_far += circ.feed_count();
                Box::new(gates_iter)
            })
            .chain(self.gates.iter().copied());

        Box::new(iter)
    }

    /// Returns the number of feeds in the circuit.
    pub fn feed_count(&self) -> usize {
        self.feed_count
    }

    /// Returns the number of gates
    pub fn gates_count(&self) -> usize {
        self.gates_count
    }

    /// Returns the number of AND gates in the circuit.
    pub fn and_count(&self) -> usize {
        self.and_count
    }

    /// Returns the number of XOR gates in the circuit.
    pub fn xor_count(&self) -> usize {
        self.xor_count
    }

    /// Reverses the order of the inputs.
    pub fn reverse_inputs(mut self) -> Self {
        self.inputs.reverse();
        self
    }

    /// Reverses endianness of the input at the given index.
    ///
    /// This only has an effect on array inputs.
    ///
    /// # Arguments
    ///
    /// * `idx` - The index of the input to reverse.
    ///
    /// # Returns
    ///
    /// The circuit with the input reversed.
    pub fn reverse_input(mut self, idx: usize) -> Self {
        if let Some(BinaryRepr::Array(arr)) = self.inputs.get_mut(idx) {
            arr.reverse();
        }
        self
    }

    /// Reverses the order of the outputs.
    pub fn reverse_outputs(mut self) -> Self {
        self.outputs.reverse();
        self
    }

    /// Reverses endianness of the output at the given index.
    ///
    /// This only has an effect on array outputs.
    ///
    /// # Arguments
    ///
    /// * `idx` - The index of the output to reverse.
    ///
    /// # Returns
    ///
    /// The circuit with the output reversed.
    pub fn reverse_output(mut self, idx: usize) -> Self {
        if let Some(BinaryRepr::Array(arr)) = self.outputs.get_mut(idx) {
            arr.reverse();
        }
        self
    }

    /// Evaluate the circuit with the given inputs.
    ///
    /// # Arguments
    ///
    /// * `values` - The inputs to the circuit
    ///
    /// # Returns
    ///
    /// The outputs of the circuit.
    pub fn evaluate(&self, values: &[Value]) -> Result<Vec<Value>, CircuitError> {
        if values.len() != self.inputs.len() {
            return Err(CircuitError::InvalidInputCount(
                self.inputs.len(),
                values.len(),
            ));
        }

        let mut gate_outputs: Vec<Option<bool>> = vec![None; self.feed_count()];

        for (input, value) in self.inputs.iter().zip(values) {
            if input.value_type() != value.value_type() {
                return Err(TypeError::UnexpectedType {
                    expected: input.value_type(),
                    actual: value.value_type(),
                })?;
            }

            for (node, bit) in input.iter().zip(value.clone().into_iter_lsb0()) {
                gate_outputs[node.id] = Some(bit);
            }
        }

        for gate in self.gates() {
            match gate {
                Gate::Xor { x, y, z } => {
                    let x = gate_outputs[x.id].expect("Feed should be set");
                    let y = gate_outputs[y.id].expect("Feed should be set");

                    gate_outputs[z.id] = Some(x ^ y);
                }
                Gate::And { x, y, z } => {
                    let x = gate_outputs[x.id].expect("Feed should be set");
                    let y = gate_outputs[y.id].expect("Feed should be set");

                    gate_outputs[z.id] = Some(x & y);
                }
                Gate::Inv { x, z } => {
                    let x = gate_outputs[x.id].expect("Feed should be set");

                    gate_outputs[z.id] = Some(!x);
                }
            }
        }

        let outputs = self
            .outputs
            .iter()
            .map(|output| {
                let bits: Vec<bool> = output
                    .iter()
                    .map(|node| gate_outputs[node.id].expect("Feed should be set"))
                    .collect();

                output
                    .from_bin_repr(&bits)
                    .expect("Output should be decodable")
            })
            .collect();

        Ok(outputs)
    }
}

#[cfg(test)]
mod tests {
    use mpz_circuits_macros::evaluate;

    use crate::{ops::WrappingAdd, CircuitBuilder};

    use super::*;

    fn build_adder() -> Circuit {
        let builder = CircuitBuilder::new();

        let a = builder.add_input::<u8>();
        let b = builder.add_input::<u8>();

        let c = a.wrapping_add(b);

        builder.add_output(c);

        builder.build().unwrap()
    }

    #[test]
    fn test_evaluate() {
        let circ = build_adder();

        let out = evaluate!(circ, fn(1u8, 2u8) -> u8).unwrap();

        assert_eq!(out, 3u8);
    }
}
