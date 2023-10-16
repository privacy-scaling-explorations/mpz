use itybity::{BitIterable, IntoBits};

use crate::{
    circuit::{CircuitGate, SubCircuit},
    components::{Feed, Gate, Node},
    types::{BinaryLength, BinaryRepr, ToBinaryRepr, ValueType},
    Circuit, Tracer,
};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    mem::discriminant,
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
        state.add_input_internal(value.clone().into());

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
        state.add_input_internal(value.clone());

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
        state.add_input_internal(values.clone().into());

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
        state.add_input_internal(values.clone().into());

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
    /// * `builder_inputs` - The inputs to the appended circuit
    ///
    /// # Returns
    ///
    /// The outputs of the appended circuit
    pub fn append(
        &self,
        circ: Arc<Circuit>,
        builder_inputs: &[BinaryRepr],
    ) -> Result<Vec<BinaryRepr>, BuilderError> {
        self.state.borrow_mut().append(circ, builder_inputs)
    }

    /// Builds the circuit
    pub fn build_arc(self) -> Result<Arc<Circuit>, BuilderError> {
        self.build().map(Arc::new)
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
    input_feeds: HashSet<usize>,
    outputs: Vec<BinaryRepr>,
    gates: Vec<CircuitGate>,
    and_count: usize,
    xor_count: usize,
    sub_circuits: Vec<SubCircuit>,
    circuit_break_points: VecDeque<usize>,
}

impl Default for BuilderState {
    fn default() -> Self {
        Self {
            // ids 0 and 1 are reserved for constant zero and one
            feed_id: 2,
            inputs: vec![],
            input_feeds: HashSet::new(),
            outputs: vec![],
            gates: vec![],
            and_count: 0,
            xor_count: 0,
            sub_circuits: vec![],
            circuit_break_points: VecDeque::new(),
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
        let out = self.add_feed();
        let gate = Gate::Xor {
            x: x.into(),
            y: y.into(),
            z: out,
        };

        if self.input_feeds.contains(&x.id()) || self.input_feeds.contains(&y.id()) {
            self.gates.push(CircuitGate::InputGate(gate));
        } else {
            self.gates.push(CircuitGate::Gate(gate));
        }

        self.xor_count += 1;
        out
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
        let out = self.add_feed();
        let gate = Gate::And {
            x: x.into(),
            y: y.into(),
            z: out,
        };

        if self.input_feeds.contains(&x.id()) || self.input_feeds.contains(&y.id()) {
            self.gates.push(CircuitGate::InputGate(gate));
        } else {
            self.gates.push(CircuitGate::Gate(gate));
        }

        self.and_count += 1;
        out
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
        let out = self.add_feed();
        let gate = Gate::Inv {
            x: x.into(),
            z: out,
        };
        if self.input_feeds.contains(&x.id()) {
            self.gates.push(CircuitGate::InputGate(gate));
        } else {
            self.gates.push(CircuitGate::Gate(gate));
        }

        out
    }

    /// Appends an existing circuit
    ///
    /// # Arguments
    ///
    /// * `circ` - The circuit to append
    /// * `builder_inputs` - The inputs to the appended circuit
    ///
    /// # Returns
    ///
    /// The outputs of the appended circuit
    pub fn append(
        &mut self,
        circuit: Arc<Circuit>,
        builder_inputs: &[BinaryRepr],
    ) -> Result<Vec<BinaryRepr>, BuilderError> {
        if builder_inputs.len() != circuit.inputs().len() {
            return Err(BuilderError::AppendError(
                "Number of inputs does not match number of inputs in circuit".to_string(),
            ));
        }

        for (i, (builder_input, append_input)) in
            builder_inputs.iter().zip(circuit.inputs()).enumerate()
        {
            if discriminant(builder_input) != discriminant(append_input) {
                return Err(BuilderError::AppendError(format!(
                    "Input {i} type does not match input type in circuit, expected {}, got {}",
                    append_input, builder_input,
                )));
            }
        }

        // Update break points
        self.circuit_break_points
            .push_back(self.gates.len() - self.circuit_break_points.iter().sum::<usize>());

        // Update the outputs
        let mut outputs = circuit.outputs().to_vec();
        outputs
            .iter_mut()
            .for_each(|output| output.shift_right(self.feed_id));

        // Capture current offset
        let offset = self.feed_id;

        // Increment variables
        self.feed_id += circuit.feed_count();
        self.and_count += circuit.and_count();
        self.xor_count += circuit.xor_count();

        // Store the new circuit as sub-circuit and the input mappings
        let mut feed_map = HashMap::new();
        circuit
            .inputs()
            .iter()
            .zip(builder_inputs)
            .for_each(|(input, builder_input)| {
                input
                    .iter()
                    .zip(builder_input.iter())
                    .for_each(|(old_input, new_input)| {
                        feed_map.insert(old_input.id(), new_input.id());
                    });
            });

        let sub_circuit = SubCircuit {
            feed_offset: offset,
            circuit,
            feed_map,
        };
        self.sub_circuits.push(sub_circuit);

        Ok(outputs)
    }

    /// Builds the circuit.
    pub(crate) fn build(mut self) -> Result<Circuit, BuilderError> {
        let gates_count = self
            .sub_circuits
            .iter()
            .map(|g| g.circuit.gates_count())
            .sum::<usize>()
            + self.gates.len();

        // Update break points
        self.circuit_break_points
            .push_back(self.gates.len() - self.circuit_break_points.iter().sum::<usize>());

        let circuit = Circuit {
            inputs: self.inputs,
            outputs: self.outputs,
            gates: self.gates,
            feed_count: self.feed_id,
            and_count: self.and_count,
            xor_count: self.xor_count,
            sub_circuits: self.sub_circuits,
            break_points: self.circuit_break_points,
            gates_count,
        };

        Ok(circuit)
    }

    fn add_input_internal(&mut self, value: BinaryRepr) {
        value.iter().for_each(|node| {
            self.input_feeds.insert(node.id());
        });
        self.inputs.push(value.clone());
    }
}

#[cfg(test)]
mod test {
    use mpz_circuits_macros::evaluate;

    use crate::ops::WrappingAdd;

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

        let mut appended_outputs = builder.append(circ, &[a.into(), c.into()]).unwrap();

        let d = appended_outputs.pop().unwrap();

        builder.add_output(d);

        let circ = builder.build_arc().unwrap();

        let mut output = circ.evaluate(&[1u8.into(), 1u8.into()]).unwrap();

        let d: u8 = output.pop().unwrap().try_into().unwrap();

        // a + (a + b) = 2a + b
        assert_eq!(d, 3u8);
    }
}
