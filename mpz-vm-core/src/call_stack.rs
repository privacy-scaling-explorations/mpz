use std::sync::Arc;

use crate::{register::RegisterId, DataInstruction, Function, Instr, Value};

pub use crate::error::CallStackError;

/// The index of an instruction in the current call frame.
pub type InstrIdx = u16;
/// The offset of a jump instruction.
pub type JumpOffset = i16;

/// A call frame.
pub struct CallFrame<I, V> {
    /// The function being called.
    function: Function<I, V>,
    /// The instruction to return to in the caller frame.
    return_idx: InstrIdx,
    /// Register stack base pointer.
    base: RegisterId,
}

/// A call stack.
pub struct CallStack<I, V> {
    /// The call stack.
    stack: Vec<CallFrame<I, V>>,
    /// Iterator over the instructions in the current call frame.
    current: InstrIter<I, V>,
}

impl<I, V> Default for CallStack<I, V> {
    fn default() -> Self {
        Self {
            stack: Default::default(),
            current: Default::default(),
        }
    }
}

impl<I, V> CallStack<I, V> {
    /// Returns `true` if the call stack is empty.
    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    /// Returns the number of call frames in the stack.
    pub fn len(&self) -> usize {
        self.stack.len()
    }

    /// Returns the current register stack base id.
    pub fn register_base(&self) -> RegisterId {
        self.stack.last().map(|frame| frame.base).unwrap_or(0)
    }

    /// Adds a new call frame to the stack.
    ///
    /// # Arguments
    ///
    /// * `func` - The function to call.
    /// * `base` - The register stack base id.
    pub fn call(&mut self, func: Function<I, V>, base: RegisterId) {
        let return_idx = self.current.get_idx();
        self.current.switch_frame(func.instr.clone(), 0);

        self.stack.push(CallFrame {
            function: func,
            return_idx,
            base,
        });
    }

    /// Returns from the current call frame.
    ///
    /// This pops the current call frame from the stack and returns to the previous call frame.
    pub fn return_call(&mut self) -> Result<(), CallStackError> {
        println!("returning call");
        let old_frame = self.stack.pop().ok_or(CallStackError::StackEmpty)?;
        if let Some(frame) = self.stack.last() {
            self.current
                .switch_frame(frame.function.instr.clone(), old_frame.return_idx);
        }

        Ok(())
    }

    /// Jumps in the current call frame to the instruction at the given offset.
    ///
    /// # Arguments
    ///
    /// * `offset` - The offset from the current instruction to jump to.
    pub fn jump(&mut self, offset: JumpOffset) -> Result<(), CallStackError> {
        let idx = self.current.get_idx() as isize;
        let new_idx = idx + offset as isize;

        if new_idx < 0 || new_idx > self.current.instr.len() as isize {
            return Err(CallStackError::InstrOutOfBounds);
        }

        self.current.set_idx(new_idx as InstrIdx);

        Ok(())
    }
}

impl<I: DataInstruction<V>, V: Value> CallStack<I, V> {
    /// Returns the next instruction in the current call frame.
    pub fn next_instr(&mut self) -> Option<Instr<I, V>> {
        self.current.next()
    }
}

/// An iterator over the instructions in a call frame.
struct InstrIter<I, V> {
    instr: Arc<[Instr<I, V>]>,
    ip: InstrIdx,
}

impl<I, V> Default for InstrIter<I, V> {
    fn default() -> Self {
        Self {
            instr: Arc::new([]),
            ip: 0,
        }
    }
}

impl<I, V> InstrIter<I, V> {
    /// Sets the instructions.
    fn switch_frame(&mut self, instr: Arc<[Instr<I, V>]>, idx: InstrIdx) {
        self.instr = instr;
        self.ip = idx;
    }

    /// Returns the current instruction index.
    fn get_idx(&self) -> InstrIdx {
        self.ip
    }

    /// Sets the current instruction index.
    fn set_idx(&mut self, idx: InstrIdx) {
        self.ip = idx;
    }
}

impl<I: Clone, V: Clone> Iterator for InstrIter<I, V> {
    type Item = Instr<I, V>;

    fn next(&mut self) -> Option<Self::Item> {
        let instr = self.instr.get(self.ip as usize).cloned();
        self.ip += 1;
        instr
    }
}
