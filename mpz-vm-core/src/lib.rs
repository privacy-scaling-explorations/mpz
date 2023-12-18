pub mod call_stack;
pub mod error;
pub mod executor;
mod function;
mod globals;
mod instruction;
pub mod machines;
pub(crate) mod register;
pub(crate) mod table;
mod thread;
pub mod value;

pub use function::Function;
pub use globals::Globals;
pub use instruction::{ControlFlowInstr, DataInstruction, Instr, MemoryInstr};
pub use register::Registers;
pub use thread::Thread;
pub use value::Value;

/// The execution status of a thread.
pub enum ExecStatus<V> {
    /// Thread still has instructions to execute.
    Pending,
    /// Thread has finished executing.
    Return(Option<V>),
}
