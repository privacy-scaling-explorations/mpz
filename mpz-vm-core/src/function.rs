use std::sync::Arc;

use crate::Instr;

/// A function.
pub struct Function<I, V> {
    /// The function's name.
    pub name: Option<String>,
    /// Number of function arguments.
    pub arity: u8,
    /// Instructions comprising the function code.
    pub instr: Arc<[Instr<I, V>]>,
}

impl<I, V> Clone for Function<I, V> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            arity: self.arity.clone(),
            instr: self.instr.clone(),
        }
    }
}
