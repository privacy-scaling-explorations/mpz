use std::sync::Arc;

use mpz_circuits::Circuit;
use mpz_garble_core::{encoding_state, EncodedValue, EncryptedGate};
use mpz_vm_core::value::{IsTrue, IsZero, ValueError, ValueErrorKind};

use crate::circuits::ADD_U8;

#[derive(Debug, Clone)]
pub enum Value<S: encoding_state::LabelState> {
    Encoded(EncodedValue<S>),
    Plain(Plain),
}

impl mpz_vm_core::Value for Value<encoding_state::Full> {}
impl mpz_vm_core::Value for Value<encoding_state::Active> {}

impl<S: encoding_state::LabelState> From<EncodedValue<S>> for Value<S> {
    fn from(value: EncodedValue<S>) -> Self {
        Self::Encoded(value)
    }
}

impl<S: encoding_state::LabelState> IsZero for Value<S> {
    fn is_zero(&self) -> Result<bool, ValueError> {
        match self {
            Value::Encoded(_) => Err(ValueError::new(
                ValueErrorKind::UnsupportedOperation,
                "is_zero can not be called on an encoded value",
            )),
            Value::Plain(value) => match value {
                Plain::Bool(v) => Ok(!*v),
                Plain::U8(v) => Ok(*v == 0),
                Plain::U16(v) => Ok(*v == 0),
                Plain::U32(v) => Ok(*v == 0),
                Plain::U64(v) => Ok(*v == 0),
            },
        }
    }
}

impl<S: encoding_state::LabelState> IsTrue for Value<S> {
    fn is_true(&self) -> Result<bool, ValueError> {
        match self {
            Value::Encoded(_) => Err(ValueError::new(
                ValueErrorKind::UnsupportedOperation,
                "is_true can not be called on an encoded value",
            )),
            Value::Plain(value) => match value {
                Plain::Bool(v) => Ok(*v),
                Plain::U8(v) => Ok(*v != 0),
                Plain::U16(v) => Ok(*v != 0),
                Plain::U32(v) => Ok(*v != 0),
                Plain::U64(v) => Ok(*v != 0),
            },
        }
    }
}

#[derive(Debug, Clone)]
pub enum Plain {
    Bool(bool),
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
}

macro_rules! impl_add_plain {
    ($($typ:ident),*) => {
        pub(crate) fn add(self, rhs: Self) -> Result<Self, ValueError> {
            match (self, rhs) {
                (Self::Bool(_), Self::Bool(_)) => Err(ValueError::new(
                    ValueErrorKind::UnsupportedOperation,
                    "can not add bools",
                )),
                $( (Self::$typ(a), Self::$typ(b)) => Ok(Self::$typ(a.wrapping_add(b))), )*
                _ => Err(ValueError::new(
                    ValueErrorKind::UnsupportedOperation,
                    "can not add different types",
                )),
            }
        }
    }
}
impl Plain {
    impl_add_plain!(U8, U16, U32, U64);
}
