use crate::{
    error::DataInstructionError,
    executor::DataExecutor,
    instruction::DataInstruction,
    register::{RegisterId, Registers},
    value::{IsTrue, IsZero, ValueError},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Bool(bool),
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
}

impl crate::Value for Value {}

impl IsTrue for Value {
    #[inline]
    fn is_true(&self) -> Result<bool, ValueError> {
        match self {
            Value::Bool(v) => Ok(*v),
            Value::U8(v) => Ok(*v != 0),
            Value::U16(v) => Ok(*v != 0),
            Value::U32(v) => Ok(*v != 0),
            Value::U64(v) => Ok(*v != 0),
        }
    }
}

impl IsZero for Value {
    #[inline]
    fn is_zero(&self) -> Result<bool, ValueError> {
        match self {
            Value::Bool(v) => Ok(!*v),
            Value::U8(v) => Ok(*v == 0),
            Value::U16(v) => Ok(*v == 0),
            Value::U32(v) => Ok(*v == 0),
            Value::U64(v) => Ok(*v == 0),
        }
    }
}

#[derive(Clone)]
pub enum DataInstr {
    /// Add two values.
    ///
    /// dest <- a + b
    Add {
        a: RegisterId,
        b: RegisterId,
        dest: RegisterId,
    },
    /// Subtract two values.
    ///
    /// dest <- a - b
    Sub {
        a: RegisterId,
        b: RegisterId,
        dest: RegisterId,
    },
    /// XOR two values.
    ///
    /// dest <- a ^ b
    Xor {
        a: RegisterId,
        b: RegisterId,
        dest: RegisterId,
    },
    /// Compare two values.
    ///
    /// dest <- a == b
    Eq {
        a: RegisterId,
        b: RegisterId,
        dest: RegisterId,
    },
}

impl DataInstruction<Value> for DataInstr {}

#[derive(Default)]
pub struct SimpleLocalExecutor;

impl DataExecutor<DataInstr, Value> for SimpleLocalExecutor {
    async fn execute(
        &mut self,
        instr: DataInstr,
        registers: &mut Registers<Value>,
    ) -> Result<(), DataInstructionError> {
        match instr {
            DataInstr::Add { a, b, dest } => {
                let a = registers[a].take().unwrap();
                let b = registers[b].take().unwrap();

                println!("a: {:?}, b: {:?}", a, b);

                match (a, b) {
                    (Value::U8(a), Value::U8(b)) => {
                        registers[dest] = Some(Value::U8(a.wrapping_add(b)));
                    }
                    _ => todo!(),
                }
            }
            DataInstr::Eq { a, b, dest } => {
                let a = registers[a].take().unwrap();
                let b = registers[b].take().unwrap();

                println!("eq: a {:?}, b {:?}", a, b);

                match (a, b) {
                    (Value::U8(a), Value::U8(b)) => {
                        registers[dest] = Some(Value::Bool(a == b));
                    }
                    _ => todo!(),
                }
            }
            _ => todo!(),
        }

        Ok(())
    }
}
