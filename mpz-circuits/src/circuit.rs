use itybity::IntoBits;
use yoke::{Yoke, Yokeable};

use crate::{
    components::Gate,
    types::{BinaryRepr, TypeError, Value},
    Feed, Node, Sink,
};
use std::{
    collections::{HashMap, VecDeque},
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
    pub(crate) gates: Vec<CircuitGate>,
    pub(crate) feed_count: usize,
    pub(crate) and_count: usize,
    pub(crate) xor_count: usize,
    pub(crate) sub_circuits: Vec<SubCircuit>,
    pub(crate) break_points: VecDeque<usize>,
    pub(crate) gates_count: usize,
}

#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub(crate) enum CircuitGate {
    Gate(Gate),
    InputGate(Gate),
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
    pub fn evaluate(self: Arc<Self>, values: &[Value]) -> Result<Vec<Value>, CircuitError> {
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

        for gate in Arc::clone(&self).into_gates_iterator() {
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

    /// Returns an iterator over the gates of the circuit.
    pub fn into_gates_iterator(self: Arc<Self>) -> CircuitIterator {
        CircuitIterator {
            inner: self.into_inner_gates_iterator(),
        }
    }

    fn into_inner_gates_iterator(self: Arc<Self>) -> InnerCircuitIterator {
        let mut break_points = self.break_points.clone();
        let current_break_point = break_points.pop_front();

        let gates = Yoke::<GateSlice<'static>, Arc<Self>>::attach_to_cart(
            Arc::clone(&self),
            |circuit: &Circuit| GateSlice {
                inner: &circuit.gates,
            },
        );

        let sub_circuits = Yoke::<SubCircuitSlice<'static>, Arc<Self>>::attach_to_cart(
            Arc::clone(&self),
            |circuit: &Circuit| SubCircuitSlice {
                inner: &circuit.sub_circuits,
            },
        );

        InnerCircuitIterator {
            next_gate: None,
            circuit: Arc::clone(&self),
            gates,
            current_gate_pos: 0,
            current_sub_circuit_pos: 0,
            sub_circuits,
            current_sub_circuit: None,
            break_points,
            current_break_point,
        }
    }
}

#[derive(Yokeable)]
struct GateSlice<'a> {
    inner: &'a [CircuitGate],
}

#[derive(Yokeable)]
struct SubCircuitSlice<'a> {
    inner: &'a [SubCircuit],
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub(crate) struct SubCircuit {
    pub(crate) feed_map: HashMap<usize, usize>,
    pub(crate) feed_offset: usize,
    pub(crate) circuit: Arc<Circuit>,
}

impl IntoIterator for &SubCircuit {
    type Item = CircuitGate;
    type IntoIter = SubCircuitIterator;

    fn into_iter(self) -> Self::IntoIter {
        let circuit = Arc::clone(&self.circuit);

        SubCircuitIterator {
            feed_map: self.feed_map.clone(),
            feed_offset: self.feed_offset,
            gates_iter: Box::new(circuit.into_inner_gates_iterator()),
        }
    }
}

pub(crate) struct SubCircuitIterator {
    feed_map: HashMap<usize, usize>,
    feed_offset: usize,
    gates_iter: Box<InnerCircuitIterator>,
}

impl Iterator for SubCircuitIterator {
    type Item = CircuitGate;

    fn next(&mut self) -> Option<Self::Item> {
        let adapt_gates = |x: Node<Sink>, y: Option<Node<Sink>>, z: Node<Feed>| {
            let mut x = x.id();
            let mut y = y.map(|y| y.id());

            if let Some(new_x) = self.feed_map.get(&(x - self.feed_offset)) {
                x = *new_x;
            }

            if let Some(ref mut y) = y {
                if let Some(new_y) = self.feed_map.get(&(*y - self.feed_offset)) {
                    *y = *new_y;
                }
            }

            (x, y, z.id())
        };

        let new_gate = match self.gates_iter.next()? {
            CircuitGate::Gate(gate) => gate.copy_with(
                gate.x().id + self.feed_offset,
                gate.y().unwrap_or(Node::new(0)).id() + self.feed_offset,
                gate.z().id + self.feed_offset,
            ),
            CircuitGate::InputGate(gate) => {
                let adapted_gates = adapt_gates(
                    Node::new(gate.x().id + self.feed_offset),
                    Some(Node::new(
                        gate.y().unwrap_or(Node::new(0)).id() + self.feed_offset,
                    )),
                    Node::new(gate.z().id + self.feed_offset),
                );
                gate.copy_with(
                    adapted_gates.0,
                    adapted_gates.1.unwrap_or_default(),
                    adapted_gates.2,
                )
            }
        };

        Some(CircuitGate::Gate(new_gate))
    }
}

/// An iterator over the gates of a circuit
pub struct CircuitIterator {
    inner: InnerCircuitIterator,
}

impl CircuitIterator {
    /// Returns a reference to the underlying circuit.
    pub fn circuit(&self) -> &Circuit {
        self.inner.circuit.as_ref()
    }

    /// Returns an Arc to the underlying circuit.
    pub fn circuit_arc(&self) -> Arc<Circuit> {
        Arc::clone(&self.inner.circuit)
    }

    /// Returns a reference to the next gate without advancing the iterator.
    pub fn peek(&mut self) -> Option<&Gate> {
        if self.inner.next_gate.is_none() {
            self.inner.next_gate = self.next().map(CircuitGate::Gate);
        }
        self.inner.next_gate.as_ref().map(|g| match g {
            CircuitGate::Gate(g) => g,
            CircuitGate::InputGate(g) => g,
        })
    }
}

impl Iterator for CircuitIterator {
    type Item = Gate;

    fn next(&mut self) -> Option<Self::Item> {
        let gate = self.inner.next();
        gate.map(|g| match g {
            CircuitGate::Gate(g) => g,
            CircuitGate::InputGate(g) => g,
        })
    }
}

struct InnerCircuitIterator {
    next_gate: Option<CircuitGate>,
    circuit: Arc<Circuit>,
    gates: Yoke<GateSlice<'static>, Arc<Circuit>>,
    current_gate_pos: usize,
    current_sub_circuit_pos: usize,
    sub_circuits: Yoke<SubCircuitSlice<'static>, Arc<Circuit>>,
    current_sub_circuit: Option<SubCircuitIterator>,
    break_points: VecDeque<usize>,
    current_break_point: Option<usize>,
}

impl Iterator for InnerCircuitIterator {
    type Item = CircuitGate;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next_gate.is_some() {
            return self.next_gate.take();
        }

        if let Some(current_break_point) = self.current_break_point {
            if current_break_point == 0 {
                self.current_break_point = None;

                let current_sub_circuit_pos = self.current_sub_circuit_pos;
                self.current_sub_circuit_pos += 1;
                self.current_sub_circuit = self
                    .sub_circuits
                    .get()
                    .inner
                    .get(current_sub_circuit_pos)
                    .map(|c| c.into_iter());
                return self.next();
            }
            self.current_break_point = Some(current_break_point - 1);

            let current_gate_pos = self.current_gate_pos;
            self.current_gate_pos += 1;
            return self.gates.get().inner.get(current_gate_pos).copied();
        }

        if let Some(ref mut current_sub_circuit) = self.current_sub_circuit {
            if let Some(gate) = current_sub_circuit.next() {
                return Some(gate);
            }
            self.current_break_point = self.break_points.pop_front();
            return self.next();
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use mpz_circuits_macros::evaluate;

    use crate::{ops::WrappingAdd, CircuitBuilder};

    use super::*;

    fn build_adder() -> Arc<Circuit> {
        let builder = CircuitBuilder::new();

        let a = builder.add_input::<u8>();
        let b = builder.add_input::<u8>();

        let c = a.wrapping_add(b);

        builder.add_output(c);

        builder.build_arc().unwrap()
    }

    #[test]
    fn test_evaluate() {
        let circ = build_adder();

        let out = evaluate!(circ, fn(1u8, 2u8) -> u8).unwrap();

        assert_eq!(out, 3u8);
    }
}
