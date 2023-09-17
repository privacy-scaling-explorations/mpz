use itybity::IntoBits;

use crate::{
    components::Gate,
    types::{BinaryRepr, TypeError, Value},
    Feed, Node,
};
use std::{collections::HashMap, sync::Arc};

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
    // Translates the n-th circuit local input wires to the global input wires
    pub(crate) sub_circuit_wiring: Vec<HashMap<Node<Feed>, Node<Feed>>>,
    pub(crate) sub_circuits: Vec<Arc<SubCircuit>>,
    pub(crate) feed_count: usize,
    pub(crate) and_count: usize,
    pub(crate) xor_count: usize,
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

    /// Returns an iterator for the gates in the circuit.
    ///
    /// Shifts the gates to the right by the number of feeds in the subcircuits before
    pub fn gates(&self) -> impl Iterator<Item = Gate> + '_ {
        let mut all_iter = vec![];
        self.sub_circuits.iter().enumerate().for_each(|(k, circ)| {
            let wire_map = &self.sub_circuit_wiring[k];
            let iter = circ.gates.iter().copied().map(move |mut g| {
                let new_x = wire_map.get(&(g.x().into()));
                let new_y = if let Some(y) = g.y() {
                    wire_map.get(&(y.into()))
                } else {
                    None
                };
                let new_z = wire_map.get(&(g.z()));
                if let Some(new_x) = new_x {
                    g.set_x(new_x.id);
                }
                if let Some(new_y) = new_y {
                    g.set_y(new_y.id);
                }
                if let Some(new_z) = new_z {
                    g.set_z(new_z.id);
                }
                g
            });
            all_iter.push(iter);
        });

        all_iter.into_iter().flatten()
    }

    /// Returns the number of feeds in the circuit.
    pub fn feed_count(&self) -> usize {
        self.feed_count
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

        let mut feeds: Vec<Option<bool>> = vec![None; self.feed_count()];

        for (input, value) in self.inputs.iter().zip(values) {
            if input.value_type() != value.value_type() {
                return Err(TypeError::UnexpectedType {
                    expected: input.value_type(),
                    actual: value.value_type(),
                })?;
            }

            for (node, bit) in input.iter().zip(value.clone().into_iter_lsb0()) {
                feeds[node.id] = Some(bit);
            }
        }
        dbg!(&feeds);

        for (k, gate) in self.gates().enumerate() {
            dbg!(k);
            match gate {
                Gate::Xor { x, y, z } => {
                    let x = feeds[x.id].expect("Feed should be set");
                    let y = feeds[y.id].expect("Feed should be set");

                    feeds[z.id] = Some(x ^ y);
                }
                Gate::And { x, y, z } => {
                    let x = feeds[x.id].expect("Feed should be set");
                    let y = feeds[y.id].expect("Feed should be set");

                    feeds[z.id] = Some(x & y);
                }
                Gate::Inv { x, z } => {
                    let x = feeds[x.id].expect("Feed should be set");

                    feeds[z.id] = Some(!x);
                }
            }
        }

        let outputs = self
            .outputs
            .iter()
            .cloned()
            .map(|output| {
                let bits: Vec<bool> = output
                    .iter()
                    .map(|node| feeds[node.id].expect("Feed should be set"))
                    .collect();

                output
                    .from_bin_repr(&bits)
                    .expect("Output should be decodable")
            })
            .collect();

        Ok(outputs)
    }
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub(crate) struct SubCircuit {
    pub(crate) gates: Vec<Gate>,
    pub(crate) feed_count: usize,
    pub(crate) and_count: usize,
    pub(crate) xor_count: usize,
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
