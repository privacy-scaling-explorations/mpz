use mpz_circuits::types::{StaticValueType, Value, ValueType};
use mpz_core::value::ValueRef;

use crate::{config::Visibility, Memory, MemoryError};

use super::{BufferedValue, DEAP};

impl Memory for DEAP {
    fn new_input<T: StaticValueType>(
        &self,
        id: &str,
        vis: Visibility,
    ) -> Result<ValueRef, MemoryError> {
        let mut state = self.state();

        let ty = T::value_type();
        let value_ref = state.value_registry.add_value(id, ty.clone())?;

        if let Visibility::Blind = vis {
            match &value_ref {
                ValueRef::Value { id } => {
                    _ = state
                        .buffer
                        .insert(id.clone(), BufferedValue::Blind { ty: ty.clone() })
                }
                ValueRef::Array(ids) => {
                    let ValueType::Array(elem_ty, _) = ty.clone() else {
                        panic!();
                    };

                    let elem_ty = *elem_ty;

                    ids.iter().for_each(|id| {
                        _ = state.buffer.insert(
                            id.clone(),
                            BufferedValue::Blind {
                                ty: elem_ty.clone(),
                            },
                        )
                    })
                }
            }
        } else {
            state.set_visibility(&value_ref, vis);
        }

        self.gen
            .generate_encoding_by_ref(&value_ref, &ty)
            .expect("ref and value match");

        Ok(value_ref)
    }

    fn new_input_array<T: StaticValueType>(
        &self,
        id: &str,
        vis: Visibility,
        len: usize,
    ) -> Result<ValueRef, MemoryError>
    where
        Vec<T>: Into<Value>,
    {
        let mut state = self.state();

        let ty = ValueType::Array(Box::new(T::value_type()), len);
        let value_ref = state.value_registry.add_value(id, ty.clone())?;
        state.set_visibility(&value_ref, vis);

        self.gen
            .generate_encoding_by_ref(&value_ref, &ty)
            .expect("ref and value match");

        Ok(value_ref)
    }

    fn new_output<T: StaticValueType>(&self, id: &str) -> Result<ValueRef, MemoryError> {
        let mut state = self.state();

        let value_ref = state.value_registry.add_value(id, T::value_type())?;

        Ok(value_ref)
    }

    fn new_output_array<T: StaticValueType>(
        &self,
        id: &str,
        len: usize,
    ) -> Result<ValueRef, MemoryError>
    where
        Vec<T>: Into<Value>,
    {
        let mut state = self.state();

        let ty = ValueType::Array(Box::new(T::value_type()), len);
        let value_ref = state.value_registry.add_value(id, ty)?;

        Ok(value_ref)
    }

    fn assign<T: StaticValueType>(
        &self,
        value_ref: &ValueRef,
        value: T,
    ) -> Result<(), MemoryError> {
        let mut state = self.state();

        let ty = state
            .value_registry
            .get_value_type_with_ref(value_ref)
            .ok_or_else(|| MemoryError::InvalidReference(value_ref.clone()))?;

        if T::value_type() != ty {
            panic!();
        }

        let value: Value = value.into();

        match value_ref {
            ValueRef::Value { id } => {
                let visibility = state.visibility.remove(id).unwrap();
                match visibility {
                    Visibility::Public => {
                        _ = state
                            .buffer
                            .insert(id.clone(), BufferedValue::Public { value })
                    }
                    Visibility::Private => {
                        _ = state
                            .buffer
                            .insert(id.clone(), BufferedValue::Private { value })
                    }
                    _ => unreachable!(),
                }
            }
            ValueRef::Array(ids) => {
                let Value::Array(elems) = value else {
                    panic!();
                };

                for (id, value) in ids.iter().zip(elems) {
                    let visibility = state.visibility.remove(id).unwrap();
                    match visibility {
                        Visibility::Public => {
                            _ = state
                                .buffer
                                .insert(id.clone(), BufferedValue::Public { value })
                        }
                        Visibility::Private => {
                            _ = state
                                .buffer
                                .insert(id.clone(), BufferedValue::Private { value })
                        }
                        _ => unreachable!(),
                    }
                }
            }
        }

        Ok(())
    }

    fn assign_array<T: StaticValueType>(
        &self,
        value_ref: &ValueRef,
        value: Vec<T>,
    ) -> Result<(), MemoryError>
    where
        Vec<T>: Into<Value>,
    {
        let ty = self
            .state()
            .value_registry
            .get_value_type_with_ref(value_ref)
            .ok_or_else(|| MemoryError::InvalidReference(value_ref.clone()))?;

        if ValueType::Array(Box::new(T::value_type()), value.len()) != ty {
            panic!();
        }

        for (id, elem) in value_ref.iter().zip(value) {
            self.assign(&ValueRef::Value { id: id.clone() }, elem)?;
        }

        Ok(())
    }

    fn get_value(&self, id: &str) -> Option<ValueRef> {
        self.state().value_registry.get_value(id)
    }

    fn get_value_type(&self, id: &str) -> Option<ValueType> {
        self.state().value_registry.get_value_type(id)
    }
}
