//! An implementation of a garbled circuit generator.

mod config;
mod error;

use std::{
    collections::HashSet,
    ops::DerefMut,
    sync::{Arc, Mutex},
};

use futures::{Sink, SinkExt};
use mpz_circuits::{
    types::{Value, ValueType},
    Circuit,
};
use mpz_core::hash::Hash;
use mpz_garble_core::{
    encoding_state, msg::GarbleMessage, ChaChaEncoder, EncodedValue, Encoder,
    Generator as GeneratorCore,
};
use utils_aio::non_blocking_backend::{Backend, NonBlockingBackend};

use crate::{
    memory::EncodingMemory,
    ot::OTSendEncoding,
    value::{ValueId, ValueRef},
    AssignedValues,
};

pub use config::{GeneratorConfig, GeneratorConfigBuilder};
pub use error::GeneratorError;

/// A garbled circuit generator.
#[derive(Debug, Default)]
pub struct Generator {
    config: GeneratorConfig,
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    /// The encoder used to encode values
    encoder: ChaChaEncoder,
    /// Encodings of values
    memory: EncodingMemory<encoding_state::Full>,
    /// The set of values that are currently active.
    ///
    /// A value is considered active when it has been encoded and sent to the evaluator.
    ///
    /// This is used to guarantee that the same encoding is never used
    /// with different active values.
    active: HashSet<ValueId>,
}

impl Generator {
    /// Create a new generator.
    pub fn new(config: GeneratorConfig, encoder_seed: [u8; 32]) -> Self {
        Self {
            config,
            state: Mutex::new(State::new(ChaChaEncoder::new(encoder_seed))),
        }
    }

    /// Convenience method for grabbing a lock to the state.
    fn state(&self) -> impl DerefMut<Target = State> + '_ {
        self.state.lock().unwrap()
    }

    /// Returns the seed used to generate encodings.
    pub(crate) fn seed(&self) -> Vec<u8> {
        self.state().encoder.seed()
    }

    /// Returns the encoding for a value.
    pub fn get_encoding(&self, value: &ValueRef) -> Option<EncodedValue<encoding_state::Full>> {
        self.state().memory.get_encoding(value)
    }

    pub(crate) fn get_encodings_by_id(
        &self,
        ids: &[ValueId],
    ) -> Option<Vec<EncodedValue<encoding_state::Full>>> {
        let state = self.state();

        ids.iter()
            .map(|id| state.memory.get_encoding_by_id(id))
            .collect::<Option<Vec<_>>>()
    }

    /// Generate encodings for a slice of values
    pub(crate) fn generate_encodings(
        &self,
        values: &[(ValueId, ValueType)],
    ) -> Result<(), GeneratorError> {
        let mut state = self.state();

        for (id, ty) in values {
            _ = state.encode_by_id(id, ty)?;
        }

        Ok(())
    }

    /// Transfer active encodings for the provided assigned values.
    ///
    /// # Arguments
    ///
    /// - `id` - The ID of this operation
    /// - `values` - The assigned values
    /// - `sink` - The sink to send the encodings to the evaluator
    /// - `ot` - The OT sender
    pub async fn setup_assigned_values<
        S: Sink<GarbleMessage, Error = std::io::Error> + Unpin,
        OT: OTSendEncoding,
    >(
        &self,
        id: &str,
        values: &AssignedValues,
        sink: &mut S,
        ot: &OT,
    ) -> Result<(), GeneratorError> {
        let ot_send_values = values.blind.clone();
        let mut direct_send_values = values.public.clone();
        direct_send_values.extend(values.private.iter().cloned());

        futures::try_join!(
            self.ot_send_active_encodings(id, &ot_send_values, ot),
            self.direct_send_active_encodings(&direct_send_values, sink)
        )?;

        Ok(())
    }

    /// Sends the encodings of the provided value to the evaluator via oblivious transfer.
    ///
    /// # Arguments
    ///
    /// - `id` - The ID of this operation
    /// - `values` - The values to send
    /// - `ot` - The OT sender
    pub async fn ot_send_active_encodings<OT: OTSendEncoding>(
        &self,
        id: &str,
        values: &[(ValueId, ValueType)],
        ot: &OT,
    ) -> Result<(), GeneratorError> {
        if values.is_empty() {
            return Ok(());
        }

        let full_encodings = {
            let mut state = self.state();
            // Filter out any values that are already active, setting them active otherwise.
            let mut values = values
                .iter()
                .filter(|(id, _)| state.active.insert(id.clone()))
                .collect::<Vec<_>>();
            values.sort_by_key(|(id, _)| id.clone());

            values
                .iter()
                .map(|(id, ty)| state.encode_by_id(id, ty))
                .collect::<Result<Vec<_>, GeneratorError>>()?
        };

        ot.send(id, full_encodings).await?;

        Ok(())
    }

    /// Directly sends the active encodings of the provided values to the evaluator.
    ///
    /// # Arguments
    ///
    /// - `values` - The values to send
    /// - `sink` - The sink to send the encodings to the evaluator
    pub async fn direct_send_active_encodings<
        S: Sink<GarbleMessage, Error = std::io::Error> + Unpin,
    >(
        &self,
        values: &[(ValueId, Value)],
        sink: &mut S,
    ) -> Result<(), GeneratorError> {
        if values.is_empty() {
            return Ok(());
        }

        let active_encodings = {
            let mut state = self.state();
            // Filter out any values that are already active, setting them active otherwise.
            let mut values = values
                .iter()
                .filter(|(id, _)| state.active.insert(id.clone()))
                .collect::<Vec<_>>();
            values.sort_by_key(|(id, _)| id.clone());

            values
                .iter()
                .map(|(id, value)| {
                    let full_encoding = state.encode_by_id(id, &value.value_type())?;
                    Ok(full_encoding.select(value.clone())?)
                })
                .collect::<Result<Vec<_>, GeneratorError>>()?
        };

        sink.send(GarbleMessage::ActiveValues(active_encodings))
            .await?;

        Ok(())
    }

    /// Generate a garbled circuit, streaming the encrypted gates to the evaluator in batches.
    ///
    /// Returns the encodings of the outputs, and optionally a hash of the circuit.
    ///
    /// # Arguments
    ///
    /// * `circ` - The circuit to garble
    /// * `inputs` - The inputs of the circuit
    /// * `outputs` - The outputs of the circuit
    /// * `sink` - The sink to send the garbled circuit to the evaluator
    /// * `hash` - Whether to hash the circuit
    pub async fn generate<S: Sink<GarbleMessage, Error = std::io::Error> + Unpin>(
        &self,
        circ: Arc<Circuit>,
        inputs: &[ValueRef],
        outputs: &[ValueRef],
        sink: &mut S,
        hash: bool,
    ) -> Result<(Vec<EncodedValue<encoding_state::Full>>, Option<Hash>), GeneratorError> {
        let (delta, inputs) = {
            let state = self.state();
            let delta = state.encoder.delta();
            let inputs = inputs
                .iter()
                .map(|value| {
                    state
                        .memory
                        .get_encoding(value)
                        .ok_or(GeneratorError::MissingEncoding(value.clone()))
                })
                .collect::<Result<Vec<_>, _>>()?;

            (delta, inputs)
        };

        let mut gen = if hash {
            GeneratorCore::new_with_hasher(circ.clone(), delta, &inputs)?
        } else {
            GeneratorCore::new(circ.clone(), delta, &inputs)?
        };

        let mut batch: Vec<_>;
        let batch_size = self.config.batch_size;
        while !gen.is_complete() {
            // Move the generator to another thread to produce the next batch
            // then send it back
            (gen, batch) = Backend::spawn(move || {
                let batch = gen.by_ref().take(batch_size).collect();
                (gen, batch)
            })
            .await;

            if !batch.is_empty() {
                sink.send(GarbleMessage::EncryptedGates(batch)).await?;
            }
        }

        let encoded_outputs = gen.outputs()?;
        let hash = gen.hash();

        if self.config.encoding_commitments {
            let commitments = encoded_outputs
                .iter()
                .map(|output| output.commit())
                .collect();

            sink.send(GarbleMessage::EncodingCommitments(commitments))
                .await?;
        }

        // Add the outputs to the memory and set as active.
        let mut state = self.state();
        for (output, encoding) in outputs.iter().zip(encoded_outputs.iter()) {
            state.memory.set_encoding(output, encoding.clone())?;
            output.iter().for_each(|id| {
                state.active.insert(id.clone());
            });
        }

        Ok((encoded_outputs, hash))
    }

    /// Send value decoding information to the evaluator.
    ///
    /// # Arguments
    ///
    /// * `values` - The values to decode
    /// * `sink` - The sink to send the decodings with
    pub async fn decode<S: Sink<GarbleMessage, Error = std::io::Error> + Unpin>(
        &self,
        values: &[ValueRef],
        sink: &mut S,
    ) -> Result<(), GeneratorError> {
        let decodings = {
            let state = self.state();
            values
                .iter()
                .map(|value| {
                    state
                        .memory
                        .get_encoding(value)
                        .ok_or(GeneratorError::MissingEncoding(value.clone()))
                        .map(|encoding| encoding.decoding())
                })
                .collect::<Result<Vec<_>, _>>()?
        };

        sink.send(GarbleMessage::ValueDecodings(decodings)).await?;

        Ok(())
    }
}

impl State {
    fn new(encoder: ChaChaEncoder) -> Self {
        Self {
            encoder,
            ..Default::default()
        }
    }

    #[allow(dead_code)]
    fn encode(
        &mut self,
        value: &ValueRef,
        ty: &ValueType,
    ) -> Result<EncodedValue<encoding_state::Full>, GeneratorError> {
        match (value, ty) {
            (ValueRef::Value { id }, ty) if !ty.is_array() => self.encode_by_id(id, ty),
            (ValueRef::Array(array), ValueType::Array(elem_ty, len)) if array.len() == *len => {
                let encodings = array
                    .ids()
                    .iter()
                    .map(|id| self.encode_by_id(id, elem_ty))
                    .collect::<Result<Vec<_>, _>>()?;

                Ok(EncodedValue::Array(encodings))
            }
            _ => panic!("invalid value and type combination: {:?} {:?}", value, ty),
        }
    }

    fn encode_by_id(
        &mut self,
        id: &ValueId,
        ty: &ValueType,
    ) -> Result<EncodedValue<encoding_state::Full>, GeneratorError> {
        let encoding = self.encoder.encode_by_type(id.to_u64(), ty);

        // Returns error if the encoding already exists
        self.memory.set_encoding_by_id(id, encoding.clone())?;

        Ok(encoding)
    }
}
