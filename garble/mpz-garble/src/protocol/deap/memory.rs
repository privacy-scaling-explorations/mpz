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
                    _ = state.buffer.insert(id.clone(), BufferedValue::Blind { ty })
                }
                ValueRef::Array(ids) => {
                    let ValueType::Array(elem_ty, _) = ty else {
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
        let value_ref = state.value_registry.add_value(id, ty)?;
        state.set_visibility(&value_ref, vis);

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

    // fn new_public_input<T: StaticValueType>(
    //     &self,
    //     id: &str,
    //     value: T,
    // ) -> Result<ValueRef, crate::MemoryError> {
    //     let mut state = self.state();

    //     let ty = T::value_type();
    //     let value_ref = state.value_registry.add_value(id, ty)?;

    //     state.add_input_config(
    //         &value_ref,
    //         ValueConfig::new_public::<T>(value_ref.clone(), value).expect("config is valid"),
    //     );

    //     Ok(value_ref)
    // }

    // fn new_public_array_input<T: StaticValueType>(
    //     &self,
    //     id: &str,
    //     value: Vec<T>,
    // ) -> Result<ValueRef, crate::MemoryError>
    // where
    //     Vec<T>: Into<Value>,
    // {
    //     let mut state = self.state();

    //     let value: Value = value.into();
    //     let ty = value.value_type();
    //     let value_ref = state.value_registry.add_value(id, ty)?;

    //     state.add_input_config(
    //         &value_ref,
    //         ValueConfig::new_public::<T>(value_ref.clone(), value).expect("config is valid"),
    //     );

    //     Ok(value_ref)
    // }

    // fn new_public_input_by_type(&self, id: &str, value: Value) -> Result<ValueRef, MemoryError> {
    //     let mut state = self.state();

    //     let ty = value.value_type();
    //     let value_ref = state.value_registry.add_value(id, ty.clone())?;

    //     state.add_input_config(
    //         &value_ref,
    //         ValueConfig::new(value_ref.clone(), ty, Some(value), Visibility::Public)
    //             .expect("config is valid"),
    //     );

    //     Ok(value_ref)
    // }

    // fn new_private_input<T: StaticValueType>(
    //     &self,
    //     id: &str,
    //     value: Option<T>,
    // ) -> Result<ValueRef, crate::MemoryError> {
    //     let mut state = self.state();

    //     let ty = T::value_type();
    //     let value_ref = state.value_registry.add_value(id, ty)?;

    //     state.add_input_config(
    //         &value_ref,
    //         ValueConfig::new_private::<T>(value_ref.clone(), value).expect("config is valid"),
    //     );

    //     Ok(value_ref)
    // }

    // fn new_private_array_input<T: StaticValueType>(
    //     &self,
    //     id: &str,
    //     value: Option<Vec<T>>,
    //     len: usize,
    // ) -> Result<ValueRef, crate::MemoryError>
    // where
    //     Vec<T>: Into<Value>,
    // {
    //     let mut state = self.state();

    //     let ty = ValueType::new_array::<T>(len);
    //     let value_ref = state.value_registry.add_value(id, ty)?;

    //     state.add_input_config(
    //         &value_ref,
    //         ValueConfig::new_private_array::<T>(value_ref.clone(), value, len)
    //             .expect("config is valid"),
    //     );

    //     Ok(value_ref)
    // }

    // fn new_private_input_by_type(
    //     &self,
    //     id: &str,
    //     ty: &ValueType,
    //     value: Option<Value>,
    // ) -> Result<ValueRef, MemoryError> {
    //     if let Some(value) = &value {
    //         if &value.value_type() != ty {
    //             return Err(TypeError::UnexpectedType {
    //                 expected: ty.clone(),
    //                 actual: value.value_type(),
    //             })?;
    //         }
    //     }

    //     let mut state = self.state();

    //     let value_ref = state.value_registry.add_value(id, ty.clone())?;

    //     state.add_input_config(
    //         &value_ref,
    //         ValueConfig::new(value_ref.clone(), ty.clone(), value, Visibility::Private)
    //             .expect("config is valid"),
    //     );

    //     Ok(value_ref)
    // }

    // fn new_output<T: StaticValueType>(&self, id: &str) -> Result<ValueRef, crate::MemoryError> {
    //     let mut state = self.state();

    //     let ty = T::value_type();
    //     let value_ref = state.value_registry.add_value(id, ty)?;

    //     Ok(value_ref)
    // }

    // fn new_array_output<T: StaticValueType>(
    //     &self,
    //     id: &str,
    //     len: usize,
    // ) -> Result<ValueRef, crate::MemoryError>
    // where
    //     Vec<T>: Into<Value>,
    // {
    //     let mut state = self.state();

    //     let ty = ValueType::new_array::<T>(len);
    //     let value_ref = state.value_registry.add_value(id, ty)?;

    //     Ok(value_ref)
    // }

    // fn new_output_by_type(&self, id: &str, ty: &ValueType) -> Result<ValueRef, MemoryError> {
    //     let mut state = self.state();

    //     let value_ref = state.value_registry.add_value(id, ty.clone())?;

    //     Ok(value_ref)
    // }

    fn get_value(&self, id: &str) -> Option<ValueRef> {
        self.state().value_registry.get_value(id)
    }

    fn get_value_type(&self, id: &str) -> Option<ValueType> {
        self.state().value_registry.get_value_type(id)
    }
}
