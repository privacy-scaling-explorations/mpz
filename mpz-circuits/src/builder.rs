use itybity::{BitIterable, IntoBits};

use crate::{
    circuit::SubCircuit,
    components::{Feed, Gate, Node},
    types::{BinaryLength, BinaryRepr, ToBinaryRepr, ValueType},
    Circuit, Tracer,
};
use std::{
    cell::RefCell,
    collections::HashMap,
    mem::{discriminant, replace, take},
    sync::Arc,
};

/// An error that can occur when building a circuit.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum BuilderError {
    #[error("missing wire connection: sink {0}")]
    MissingWire(usize),
    #[error("error appending circuit: {0}")]
    AppendError(String),
}

/// A circuit builder.
///
/// This type is used in conjunction with [`Tracer`](crate::Tracer) to build a circuit.
///
/// # Example
///
/// The following example shows how to build a circuit that adds two u8 inputs.
///
/// ```
/// use mpz_circuits::{CircuitBuilder, Tracer, ops::WrappingAdd};
/// use std::cell::RefCell;
///
/// let builder = CircuitBuilder::new();
///
/// // Add two u8 inputs to the circuit
/// let a = builder.add_input::<u8>();
/// let b = builder.add_input::<u8>();
///
/// // Add the two inputs together
/// let c = a.wrapping_add(b);
///
/// // Add the output to the circuit
/// builder.add_output(c);
///
/// // Build the circuit
/// let circuit = builder.build().unwrap();
/// ```
#[derive(Default)]
pub struct CircuitBuilder {
    state: RefCell<BuilderState>,
}

impl CircuitBuilder {
    /// Creates a new circuit builder
    pub fn new() -> Self {
        Self {
            state: RefCell::new(BuilderState::default()),
        }
    }

    /// Returns a reference to the internal state of the builder
    pub fn state(&self) -> &RefCell<BuilderState> {
        &self.state
    }

    /// Adds a new input to the circuit of the provided type
    ///
    /// # Returns
    ///
    /// The binary encoded form of the input.
    pub fn add_input<T: ToBinaryRepr + BinaryLength>(&self) -> Tracer<'_, T::Repr> {
        let mut state = self.state.borrow_mut();

        let value = state.add_value::<T>();
        state.inputs.push(value.clone().into());

        Tracer::new(&self.state, value)
    }

    /// Adds a new input to the circuit of the provided type
    ///
    /// # Arguments
    ///
    /// * `typ` - The type of the input.
    ///
    /// # Returns
    ///
    /// The binary encoded form of the input.
    pub fn add_input_by_type(&self, typ: ValueType) -> BinaryRepr {
        let mut state = self.state.borrow_mut();

        let value = state.add_value_by_type(typ);
        state.inputs.push(value.clone());

        value
    }

    /// Adds a new array input to the circuit of the provided type
    ///
    /// # Returns
    ///
    /// The binary encoded form of the array.
    pub fn add_array_input<T: ToBinaryRepr + BinaryLength, const N: usize>(
        &self,
    ) -> [Tracer<'_, T::Repr>; N]
    where
        [T::Repr; N]: Into<BinaryRepr>,
    {
        let mut state = self.state.borrow_mut();

        let values: [T::Repr; N] = std::array::from_fn(|_| state.add_value::<T>());
        state.inputs.push(values.clone().into());

        values.map(|v| Tracer::new(&self.state, v))
    }

    /// Adds a new `Vec<T>` input to the circuit of the provided type
    ///
    /// # Arguments
    ///
    /// * `len` - The length of the vector.
    ///
    /// # Returns
    ///
    /// The binary encoded form of the vector.
    pub fn add_vec_input<T: ToBinaryRepr + BinaryLength>(
        &self,
        len: usize,
    ) -> Vec<Tracer<'_, T::Repr>>
    where
        Vec<T::Repr>: Into<BinaryRepr>,
    {
        let mut state = self.state.borrow_mut();

        let values: Vec<T::Repr> = (0..len).map(|_| state.add_value::<T>()).collect();
        state.inputs.push(values.clone().into());

        values
            .into_iter()
            .map(|v| Tracer::new(&self.state, v))
            .collect()
    }

    /// Adds a new output to the circuit
    pub fn add_output(&self, value: impl Into<BinaryRepr>) {
        let mut state = self.state.borrow_mut();

        state.outputs.push(value.into());
    }

    /// Returns a tracer for a constant value
    pub fn get_constant<T: ToBinaryRepr + BitIterable>(&self, value: T) -> Tracer<'_, T::Repr> {
        let mut state = self.state.borrow_mut();

        let value = state.get_constant(value);
        Tracer::new(&self.state, value)
    }

    /// Appends an existing circuit
    ///
    /// # Arguments
    ///
    /// * `circ` - The circuit to append
    /// * `circ_new_input_wiring` - The inputs to the appended circuit
    ///
    /// # Returns
    ///
    /// The outputs of the appended circuit
    pub fn append(
        &self,
        circ: Arc<Circuit>,
        circ_new_input_wiring: &[BinaryRepr],
    ) -> Result<Vec<BinaryRepr>, BuilderError> {
        self.state.borrow_mut().append(circ, circ_new_input_wiring)
    }

    /// Builds the circuit
    pub fn build(self) -> Result<Circuit, BuilderError> {
        self.state.into_inner().build()
    }
}

/// The internal state of the [`CircuitBuilder`]
#[derive(Debug)]
pub struct BuilderState {
    feed_id: usize,
    inputs: Vec<BinaryRepr>,
    outputs: Vec<BinaryRepr>,
    gates: Vec<Gate>,
    sub_circuit_wiring: Vec<(Vec<Node<Feed>>, Vec<Node<Feed>>)>,
    sub_circuits: Vec<Arc<SubCircuit>>,
    and_count: usize,
    xor_count: usize,
}

impl Default for BuilderState {
    fn default() -> Self {
        Self {
            // ids 0 and 1 are reserved for constant zero and one
            feed_id: 2,
            inputs: vec![],
            outputs: vec![],
            gates: vec![],
            sub_circuit_wiring: vec![],
            sub_circuits: vec![],
            and_count: 0,
            xor_count: 0,
        }
    }
}

impl BuilderState {
    /// Returns constant zero node.
    pub(crate) fn get_const_zero(&self) -> Node<Feed> {
        Node::<Feed>::new(0)
    }

    /// Returns constant one node.
    pub(crate) fn get_const_one(&self) -> Node<Feed> {
        Node::<Feed>::new(1)
    }

    /// Returns a value encoded using constant nodes.
    ///
    /// # Arguments
    ///
    /// * `value` - The value to encode.
    pub fn get_constant<T: ToBinaryRepr + BitIterable>(&mut self, value: T) -> T::Repr {
        let zero = self.get_const_zero();
        let one = self.get_const_one();

        let nodes: Vec<_> = value
            .into_iter_lsb0()
            .map(|bit| if bit { one } else { zero })
            .collect();

        T::new_bin_repr(&nodes).expect("Value should have correct bit length")
    }

    /// Adds a feed to the circuit.
    pub(crate) fn add_feed(&mut self) -> Node<Feed> {
        let feed = Node::<Feed>::new(self.feed_id);
        self.feed_id += 1;

        feed
    }

    /// Adds a value to the circuit.
    pub(crate) fn add_value<T: ToBinaryRepr + BinaryLength>(&mut self) -> T::Repr {
        let nodes: Vec<_> = (0..T::LEN).map(|_| self.add_feed()).collect();
        T::new_bin_repr(&nodes).expect("Value should have correct bit length")
    }

    /// Adds a value to the circuit by type.
    ///
    /// # Arguments
    ///
    /// * `typ` - The type of the value to add.
    pub(crate) fn add_value_by_type(&mut self, typ: ValueType) -> BinaryRepr {
        let nodes: Vec<_> = (0..typ.len()).map(|_| self.add_feed()).collect();
        typ.to_bin_repr(&nodes)
            .expect("Value should have correct bit length")
    }

    /// Adds an XOR gate to the circuit.
    ///
    /// # Arguments
    ///
    /// * `x` - The first input to the gate.
    /// * `y` - The second input to the gate.
    ///
    /// # Returns
    ///
    /// The output of the gate.
    pub(crate) fn add_xor_gate(&mut self, x: Node<Feed>, y: Node<Feed>) -> Node<Feed> {
        // if either input is a constant, we can simplify the gate
        if x.id() == 0 && y.id() == 0 {
            self.get_const_zero()
        } else if x.id() == 1 && y.id() == 1 {
            return self.get_const_zero();
        } else if x.id() == 0 {
            return y;
        } else if y.id() == 0 {
            return x;
        } else if x.id() == 1 {
            let out = self.add_feed();
            self.gates.push(Gate::Inv {
                x: y.into(),
                z: out,
            });
            return out;
        } else if y.id() == 1 {
            let out = self.add_feed();
            self.gates.push(Gate::Inv {
                x: x.into(),
                z: out,
            });
            return out;
        } else {
            let out = self.add_feed();
            self.gates.push(Gate::Xor {
                x: x.into(),
                y: y.into(),
                z: out,
            });
            self.xor_count += 1;
            return out;
        }
    }

    /// Adds an AND gate to the circuit.
    ///
    /// # Arguments
    ///
    /// * `x` - The first input to the gate.
    /// * `y` - The second input to the gate.
    ///
    /// # Returns
    ///
    /// The output of the gate.
    pub(crate) fn add_and_gate(&mut self, x: Node<Feed>, y: Node<Feed>) -> Node<Feed> {
        // if either input is a constant, we can simplify the gate
        if x.id() == 0 || y.id() == 0 {
            self.get_const_zero()
        } else if x.id() == 1 {
            return y;
        } else if y.id() == 1 {
            return x;
        } else {
            let out = self.add_feed();
            self.gates.push(Gate::And {
                x: x.into(),
                y: y.into(),
                z: out,
            });
            self.and_count += 1;
            return out;
        }
    }

    /// Adds an INV gate to the circuit.
    ///
    /// # Arguments
    ///
    /// * `x` - The input to the gate.
    ///
    /// # Returns
    ///
    /// The output of the gate.
    pub(crate) fn add_inv_gate(&mut self, x: Node<Feed>) -> Node<Feed> {
        if x.id() == 0 {
            self.get_const_one()
        } else if x.id() == 1 {
            return self.get_const_zero();
        } else {
            let out = self.add_feed();
            self.gates.push(Gate::Inv {
                x: x.into(),
                z: out,
            });
            return out;
        }
    }

    /// Appends an existing circuit
    ///
    /// # Arguments
    ///
    /// * `circ` - The circuit to append
    /// * `circ_new_input_wiring` - The inputs to the appended circuit
    ///
    /// # Returns
    ///
    /// The outputs of the appended circuit
    pub(crate) fn append(
        &mut self,
        circ: Arc<Circuit>,
        // This indicates the wires which should be connected to the inputs of the new circuit
        new_circuit_input_wiring: &[BinaryRepr],
    ) -> Result<Vec<BinaryRepr>, BuilderError> {
        // Now update the current builder state with the subcircuits of the new circuit
        // We need to shift the node ids of these subcircuits by the number of feeds we have so far
        self.build_sub_circuit();

        // Check if `new_circuit_input_wiring` is consistent with `circ.inputs()` and create
        // feedmap
        let input_feed_map = check_and_create_feed_map(circ.inputs(), new_circuit_input_wiring)?;

        let offset = self
            .sub_circuits
            .iter()
            .map(|c| c.feed_count)
            .sum::<usize>();
        self.sub_circuits.extend_from_slice(&circ.sub_circuits);
        let circuit_wiring_adapted = circ
            .sub_circuit_wiring
            .iter()
            .map(|(inputs, outputs)| {
                (
                    inputs
                        .iter()
                        .copied()
                        .map(|mut node| {
                            if let Some(n) = input_feed_map.get(&node) {
                                node = *n;
                            } else {
                                node.shift_right(offset);
                            }
                            node
                        })
                        .collect(),
                    outputs
                        .iter()
                        .copied()
                        .map(|mut node| {
                            node.shift_right(offset);
                            node
                        })
                        .collect(),
                )
            })
            .collect::<Vec<(Vec<Node<Feed>>, Vec<Node<Feed>>)>>();

        self.sub_circuit_wiring
            .extend_from_slice(&circuit_wiring_adapted);

        self.outputs = circ
            .outputs
            .iter()
            .cloned()
            .map(|mut binary| {
                binary.shift_right(offset);
                binary
            })
            .collect();

        Ok(self.outputs.clone())
    }

    /// Builds the circuit.
    pub(crate) fn build(mut self) -> Result<Circuit, BuilderError> {
        self.build_sub_circuit();
        // Shift all the node ids to the left by 2 to eliminate
        // the reserved constant nodes (which should be factored out during building)
        self.inputs.iter_mut().for_each(|input| input.shift_left(2));
        self.outputs
            .iter_mut()
            .for_each(|output| output.shift_left(2));

        let feed_count = self.sub_circuits.iter().map(|c| c.feed_count).sum();
        let and_count = self.sub_circuits.iter().map(|c| c.and_count).sum();
        let xor_count = self.sub_circuits.iter().map(|c| c.xor_count).sum();

        let circuit = Circuit {
            inputs: self.inputs,
            outputs: self.outputs,
            sub_circuit_wiring: self.sub_circuit_wiring,
            sub_circuits: self.sub_circuits,
            feed_count,
            and_count,
            xor_count,
        };

        Ok(circuit)
    }

    fn build_sub_circuit(&mut self) {
        // Create a subcircuit of the current non-finished circuit and append it
        // Also update the `sub_circuit_wiring` of the current builder state
        let mut sub_circuit = SubCircuit {
            gates: take(&mut self.gates),
            feed_count: replace(&mut self.feed_id, 2) - 2,
            and_count: take(&mut self.and_count),
            xor_count: take(&mut self.xor_count),
        };
        sub_circuit
            .gates
            .iter_mut()
            .for_each(|gate| gate.shift_left(2));

        self.sub_circuits.push(sub_circuit.into());
        self.sub_circuit_wiring.push((
            self.inputs
                .iter()
                .flat_map(|binary| {
                    binary.iter().copied().map(|mut node| {
                        node.shift_left(2);
                        node
                    })
                })
                .collect(),
            self.outputs
                .iter()
                .flat_map(|binary| {
                    binary.iter().copied().map(|mut node| {
                        node.shift_left(2);
                        node
                    })
                })
                .collect(),
        ));
    }
}

/// Checks if `feed_keys` and `feed_values` are consistent and returns a hashmap
fn check_and_create_feed_map(
    feed_keys: &[BinaryRepr],
    feed_values: &[BinaryRepr],
) -> Result<HashMap<Node<Feed>, Node<Feed>>, BuilderError> {
    if feed_keys.len() != feed_values.len() {
        return Err(BuilderError::AppendError(
            "Number of inputs does not match number of inputs in circuit".to_string(),
        ));
    }

    let mut feed_map: HashMap<Node<Feed>, Node<Feed>> = HashMap::default();
    for (i, (key_wires, value_wires)) in feed_keys.iter().zip(feed_values).enumerate() {
        if discriminant(key_wires) != discriminant(value_wires) {
            return Err(BuilderError::AppendError(format!(
                "Input {i} type does not match input type in circuit, expected {}, got {}",
                value_wires, key_wires,
            )));
        }
        for (key_node, value_node) in key_wires.iter().zip(value_wires.iter()) {
            feed_map.insert(*value_node, *key_node);
        }
    }

    Ok(feed_map)
}

#[cfg(test)]
mod test {
    use mpz_circuits_macros::evaluate;

    use crate::ops::WrappingAdd;

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
    fn test_builder() {
        let adder = build_adder();
        dbg!(adder);

        //        // Build second circuit
        //        let builder = CircuitBuilder::new();
        //
        //        let e = builder.add_input::<u8>();
        //        let f = builder.add_input::<u8>();
        //
        //        let g = e.wrapping_add(f);
        //
        //        let mut h = builder.append(adder.into(), &[f.into(), g.into()]).unwrap();
        //
        //        let h = h.pop().unwrap();
        //        builder.add_output(h);
        //
        //        let circuit = builder.build().unwrap();
        //        dbg!(&circuit.sub_circuits);
    }

    #[test]
    fn test_build_adder() {
        let circ = build_adder();

        let a = 1u8;
        let b = 255u8;
        let c = a.wrapping_add(b);

        let output = evaluate!(circ, fn(a, b) -> u8).unwrap();

        assert_eq!(output, c);
    }

    #[test]
    fn test_append() {
        let circ = build_adder();

        let builder = CircuitBuilder::new();

        let a = builder.add_input::<u8>();
        let b = builder.add_input::<u8>();

        let c = a.wrapping_add(b);

        let mut appended_outputs = builder.append(circ.into(), &[a.into(), c.into()]).unwrap();

        let d = appended_outputs.pop().unwrap();

        builder.add_output(d);

        let circ = builder.build().unwrap();

        let mut output = circ.evaluate(&[1u8.into(), 1u8.into()]).unwrap();

        let d: u8 = output.pop().unwrap().try_into().unwrap();

        // a + (a + b) = 2a + b
        assert_eq!(d, 3u8);
    }
}
