use crate::{call_stack::JumpOffset, register::RegisterId, table::FunctionId, Value};

/// An instruction which operates on data, such as performing arithmetic.
pub trait DataInstruction<V>: Clone + Send + Sync {}

impl<V, T> DataInstruction<V> for T
where
    V: Value,
    T: Clone + Send + Sync,
{
}

/// An instruction.
///
/// An instruction is generic over the data instruction set, which will vary
/// depending on the type of virtual machine.
#[derive(Debug, Clone)]
pub enum Instr<I, V> {
    /// A data instruction.
    Data(I),
    /// A control flow instruction.
    ControlFlow(ControlFlowInstr),
    /// A memory instruction.
    Memory(MemoryInstr<V>),
}

/// A control flow instruction.
#[derive(Debug, Clone)]
pub enum ControlFlowInstr {
    Jump {
        /// The jump offset.
        offset: JumpOffset,
    },
    JumpIfTrue {
        /// The jump offset.
        offset: JumpOffset,
        /// The register to check.
        condition: RegisterId,
    },
    JumpIfFalse {
        /// The jump offset.
        offset: JumpOffset,
        /// The register to check.
        condition: RegisterId,
    },
    Call {
        /// The function to call.
        function: FunctionId,
        /// The number of arguments provided in the call.
        arg_count: u8,
        /// The register to store the return value in.
        dest: RegisterId,
    },
    Return {
        /// The register to return.
        ret: RegisterId,
    },
    Branch,
}

/// A memory instruction.
#[derive(Debug, Clone)]
pub enum MemoryInstr<V> {
    Store {
        /// The source register.
        src: RegisterId,
        /// The memory address.
        addr: RegisterId,
    },
    Load {
        /// The memory address.
        addr: RegisterId,
        /// The destination register.
        dest: RegisterId,
    },
    LoadLiteral {
        /// The value to load.
        value: V,
        /// The destination register.
        dest: RegisterId,
    },
    Move {
        /// The source register.
        src: RegisterId,
        /// The destination register.
        dest: RegisterId,
    },
    Copy {
        /// The source register.
        src: RegisterId,
        /// The destination register.
        dest: RegisterId,
    },
}
