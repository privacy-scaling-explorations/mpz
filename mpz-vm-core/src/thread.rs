use std::sync::Arc;

use crate::{call_stack::CallStack, register::Registers, Function, Globals};

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
}
