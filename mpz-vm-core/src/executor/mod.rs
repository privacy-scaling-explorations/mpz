pub mod local;

use std::marker::PhantomData;

use futures::Future;

use crate::{
    call_stack::CallStack,
    error::{ControlFlowError, DataInstructionError, ExecutorError, MemoryInstructionError},
    register::Registers,
    ControlFlowInstr, DataInstruction, ExecStatus, Globals, Instr, MemoryInstr, Thread, Value,
};

pub trait DataExecutor<I, V>
where
    I: DataInstruction<V>,
    V: Value,
{
    fn execute(
        &mut self,
        instr: I,
        registers: &mut Registers<V>,
    ) -> impl Future<Output = Result<(), DataInstructionError>> + Send;
}

pub trait ControlFlowExecutor<I, V>
where
    I: DataInstruction<V>,
    V: Value,
{
    /// Executes a control flow instruction.
    ///
    /// # Arguments
    ///
    /// * `instr` - The instruction to execute.
    /// * `registers` - A thread's registers.
    /// * `call_stack` - A thread's call stack.
    /// * `globals` - The VM's globals.
    fn execute(
        &mut self,
        instr: ControlFlowInstr,
        registers: &mut Registers<V>,
        call_stack: &mut CallStack<I, V>,
        globals: &Globals<I, V>,
    ) -> impl Future<Output = Result<ExecStatus<V>, ControlFlowError>> + Send;
}

pub trait MemoryExecutor<I, V>
where
    I: DataInstruction<V>,
    V: Value,
{
    fn execute(
        &mut self,
        instr: MemoryInstr<V>,
        registers: &mut Registers<V>,
        globals: &Globals<I, V>,
    ) -> impl Future<Output = Result<(), MemoryInstructionError>> + Send;
}

pub struct Executor<I, V, D, C, M> {
    data: D,
    cf: C,
    memory: M,
    _pd: PhantomData<(I, V)>,
}

impl<I, V, D, C, M> Executor<I, V, D, C, M> {
    /// Creates a new executor.
    pub fn new(data: D, cf: C, memory: M) -> Self {
        Self {
            data,
            cf,
            memory,
            _pd: PhantomData,
        }
    }
}

impl<I, V, D, C, M> Executor<I, V, D, C, M>
where
    I: DataInstruction<V>,
    V: Value,
    D: DataExecutor<I, V>,
    C: ControlFlowExecutor<I, V>,
    M: MemoryExecutor<I, V>,
{
    /// Executes the next instruction in the thread.
    pub async fn execute_instr(
        &mut self,
        thread: &mut Thread<I, V>,
    ) -> Result<ExecStatus<V>, ExecutorError> {
        let Some(instr) = thread.call_stack.next_instr() else {
            return Ok(ExecStatus::Return(None));
        };

        println!("registers: {:?}", &thread.registers.get_mut()[..10]);

        match instr {
            Instr::Data(instr) => {
                self.data.execute(instr, &mut thread.registers).await?;
            }
            Instr::ControlFlow(instr) => {
                return self
                    .cf
                    .execute(
                        instr,
                        &mut thread.registers,
                        &mut thread.call_stack,
                        &thread.globals,
                    )
                    .await
                    .map_err(ExecutorError::from);
            }
            Instr::Memory(instr) => {
                self.memory
                    .execute(instr, &mut thread.registers, &thread.globals)
                    .await?;
            }
        }

        Ok(ExecStatus::Pending)
    }

    /// Executes the thread until it returns.
    pub async fn execute(&mut self, thread: &mut Thread<I, V>) -> Result<Option<V>, ExecutorError> {
        loop {
            if let ExecStatus::Return(val) = self.execute_instr(thread).await? {
                return Ok(val);
            }
        }
    }
}

impl<I, V, D, C, M> Default for Executor<I, V, D, C, M>
where
    D: Default,
    C: Default,
    M: Default,
{
    fn default() -> Self {
        Self::new(Default::default(), Default::default(), Default::default())
    }
}
