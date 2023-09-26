use itybity::IntoBits;

use crate::{
    components::Gate,
    types::{BinaryRepr, TypeError, Value},
};
use std::{
    collections::{BTreeMap, VecDeque},
    slice::Iter,
    sync::Arc,
};

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
    pub(crate) sub_circuits: Vec<SubCircuit>,
    pub(crate) break_points: VecDeque<usize>,
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
    pub fn gates(&self) -> GatesIterator {
        self.into_iter()
    }

    // pub fn gates(&self) -> Box<dyn Iterator<Item = Gate> + '_> {
    //     let iter = self
    //         .sub_circuits
    //         .iter()
    //         .enumerate()
    //         .flat_map(move |(k, sub_circ)| {
    //             let old_inputs = sub_circ
    //                 .circuit
    //                 .inputs
    //                 .iter()
    //                 .flat_map(|bin| bin.iter())
    //                 .collect::<Vec<_>>();
    //             let new_inputs = self.sub_circuits_inputs[k]
    //                 .iter()
    //                 .flat_map(|bin| bin.iter())
    //                 .collect::<Vec<_>>();

    //             let gates_iter = {
    //                 let offset = sub_circ.feed_offset;
    //                 let iter = sub_circ.circuit.gates().map(move |mut gate| {
    //                     gate.shift_right(offset);

    //                     let x = gate.x();
    //                     let y = gate.y();

    //                     if let Some(pos) =
    //                         old_inputs.iter().position(|node| node.id + offset == x.id)
    //                     {
    //                         gate.set_x(new_inputs[pos].id);
    //                     }

    //                     if let Some(y) = y {
    //                         if let Some(pos) =
    //                             old_inputs.iter().position(|node| node.id + offset == y.id)
    //                         {
    //                             gate.set_y(new_inputs[pos].id);
    //                         }
    //                     }

    //                     gate
    //                 });
    //                 Box::new(iter)
    //             };

    //             Box::new(gates_iter)
    //         })
    //         .chain(self.gates.iter().copied());

    //     Box::new(iter)
    // }

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

        let mut feeds: Vec<Option<bool>> = vec![None; self.feed_count()];
        feeds[0] = Some(false);
        feeds[1] = Some(true);

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

        for gate in self.gates() {
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
    pub(crate) feed_map: BTreeMap<usize, usize>,
    pub(crate) feed_offset: usize,
    pub(crate) circuit: Arc<Circuit>,
}

pub struct GatesIterator<'a> {
    gates: Iter<'a, Gate>,
    sub_circuits: Iter<'a, SubCircuit>,
    current_sub_circuit: Option<Iter<'a, Gate>>,
    sub_circuit_pos: usize,
    break_points: VecDeque<usize>,
    current_break_point: Option<usize>,
}

impl<'a> Iterator for GatesIterator<'a> {
    type Item = Gate;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(current_break_point) = self.current_break_point {
            if current_break_point == 0 {
                self.current_break_point = None;
                return self.next();
            }
            self.current_break_point = Some(current_break_point - 1);
            return self.gates.next().copied();
        }
        self.current_break_point = self.break_points.pop_front();

        if let Some(current_sub_circuit) = self.current_sub_circuit {
            if let Some(gate) = current_sub_circuit.next().cloned() {
                return Some(gate);
            }

            self.current_sub_circuit = None;
            return self.next();
        }
        if let Some(sub_circuit) = self.sub_circuits.next() {
            self.current_sub_circuit = return self.next();
        }
    }
}

impl<'a> IntoIterator for &'a Circuit {
    type Item = Gate;
    type IntoIter = GatesIterator<'a>;

    fn into_iter(self) -> Self::IntoIter {
        GatesIterator {
            gates: self.gates.iter(),
            sub_circuits: self.sub_circuits.iter(),
            sub_circuit_pos: 0,
            break_points: self.break_points,
            current_sub_circuit: None,
            current_break_point: None,
        }
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
