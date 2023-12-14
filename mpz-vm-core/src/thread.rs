use std::sync::Arc;

use crate::{
    call_stack::CallStack,
    register::{Registers, ARGUMENT_REGISTER},
    Function, Globals,
};

/// A single thread of execution.
pub struct Thread<I, V> {
    pub globals: Arc<Globals<I, V>>,
    pub call_stack: CallStack<I, V>,
    pub registers: Registers<V>,
}

impl<I, V> Thread<I, V> {
    /// Creates a new thread.
    pub fn new(globals: Arc<Globals<I, V>>) -> Self {
        Self {
            globals,
            call_stack: CallStack::default(),
            registers: Registers::default(),
        }
    }

    /// Adds a new call frame to the stack.
    pub fn call(&mut self, func: Function<I, V>) {
        self.call_stack.call(func, 0);
    }

    /// Adds a new call frame to the stack, with the provided arguments.
    pub fn call_with_args(&mut self, func: Function<I, V>, args: impl Into<Vec<V>>) {
        let args: Vec<_> = args.into();
        for (id, arg) in args.into_iter().enumerate() {
            self.registers[ARGUMENT_REGISTER + id as u16] = Some(arg);
        }
        self.call(func);
    }
}
