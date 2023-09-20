use crate::{
    components::{Feed, Gate, Node},
    types::{BinaryLength, BinaryRepr, ToBinaryRepr, ValueType},
    Circuit, Tracer,
};
use std::{
    cell::RefCell,
    mem::{discriminant, take},
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
    pub fn build(self) -> Result<Circuit, BuilderError> {
        self.state.into_inner().build()
    }
}

/// The internal state of the [`CircuitBuilder`]
#[derive(Default, Debug)]
pub struct BuilderState {
    feed_id: usize,
    inputs: Vec<BinaryRepr>,
    outputs: Vec<BinaryRepr>,
    gates: Vec<Gate>,
    and_count: usize,
    xor_count: usize,
    appended_circuits: Vec<Arc<Circuit>>,
    appended_circuits_inputs: Vec<Vec<BinaryRepr>>,
}

impl BuilderState {
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
        self.gates.push(Gate::Xor {
            x: x.into(),
            y: y.into(),
            z: out,
        });
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
        self.gates.push(Gate::And {
            x: x.into(),
            y: y.into(),
            z: out,
        });
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
        self.gates.push(Gate::Inv {
            x: x.into(),
            z: out,
        });
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
        circ: Arc<Circuit>,
        builder_inputs: &[BinaryRepr],
    ) -> Result<Vec<BinaryRepr>, BuilderError> {
        if builder_inputs.len() != circ.inputs().len() {
            return Err(BuilderError::AppendError(
                "Number of inputs does not match number of inputs in circuit".to_string(),
            ));
        }

        for (i, (builder_input, append_input)) in
            builder_inputs.iter().zip(circ.inputs()).enumerate()
        {
            if discriminant(builder_input) != discriminant(append_input) {
                return Err(BuilderError::AppendError(format!(
                    "Input {i} type does not match input type in circuit, expected {}, got {}",
                    append_input, builder_input,
                )));
            }
        }

        // Append self
        let current_circ = self.build_current();
        self.appended_circuits.push(current_circ.into());
        self.appended_circuits_inputs.push(vec![]);

        // Update the outputs
        self.outputs = circ.outputs().to_vec();
        self.outputs
            .iter_mut()
            .for_each(|output| output.shift_right(self.feed_id));

        // Store the new circuit and the input mappings
        self.appended_circuits.push(Arc::clone(&circ));
        self.appended_circuits_inputs.push(builder_inputs.to_vec());

        // Increment variables
        self.feed_id += circ.feed_count();
        self.and_count += circ.and_count();
        self.xor_count += circ.xor_count();

        Ok(self.outputs.clone())
    }

    /// Builds the circuit.
    pub(crate) fn build(self) -> Result<Circuit, BuilderError> {
        let gates_count = self
            .appended_circuits
            .iter()
            .map(|g| g.gates_count())
            .sum::<usize>()
            + self.gates.len();

        Ok(Circuit {
            inputs: self.inputs,
            outputs: self.outputs,
            gates: self.gates,
            feed_count: self.feed_id,
            and_count: self.and_count,
            xor_count: self.xor_count,
            appended_circuits: self.appended_circuits,
            appended_circuits_inputs: self.appended_circuits_inputs,
            gates_count,
        })
    }

    pub(crate) fn build_current(&mut self) -> Circuit {
        let gates = take(&mut self.gates);
        let gates_count = gates.len();

        Circuit {
            inputs: vec![],
            outputs: vec![],
            gates,
            feed_count: self.feed_id
                - self
                    .appended_circuits
                    .iter()
                    .map(|c| c.feed_count())
                    .sum::<usize>(),
            and_count: self.and_count
                - self
                    .appended_circuits
                    .iter()
                    .map(|c| c.and_count())
                    .sum::<usize>(),
            xor_count: self.xor_count
                - self
                    .appended_circuits
                    .iter()
                    .map(|c| c.xor_count())
                    .sum::<usize>(),
            appended_circuits: vec![],
            appended_circuits_inputs: vec![],
            gates_count,
        }
    }
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

        let _appended_outputs = builder.append(circ.into(), &[a.into(), c.into()]).unwrap();

        let circ = builder.build().unwrap();

        let mut output = circ.evaluate(&[2u8.into(), 7u8.into()]).unwrap();

        let d: u8 = output.pop().unwrap().try_into().unwrap();

        // a + (a + b) = 2a + b
        assert_eq!(d, 11u8);
    }
}
