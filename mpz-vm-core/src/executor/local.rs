use crate::{
    call_stack::CallStack,
    error::{ControlFlowError, MemoryInstructionError},
    executor::{ControlFlowExecutor, MemoryExecutor},
    register::{Registers, RETURN_REGISTER},
    ControlFlowInstr, DataInstruction, ExecStatus, Globals, MemoryInstr, Value,
};

#[derive(Default)]
pub struct LocalCfExecutor;

impl<I, V> ControlFlowExecutor<I, V> for LocalCfExecutor
where
    I: DataInstruction<V>,
    V: Value,
{
    async fn execute(
        &mut self,
        instr: ControlFlowInstr,
        registers: &mut Registers<V>,
        call_stack: &mut CallStack<I, V>,
        globals: &Globals<I, V>,
    ) -> Result<ExecStatus<V>, ControlFlowError> {
        match instr {
            ControlFlowInstr::Return { ret } => {
                registers[RETURN_REGISTER] = registers[ret].take();

                call_stack.return_call()?;
                registers.set_base(call_stack.register_base());

                if call_stack.is_empty() {
                    println!("returning from main");
                    return Ok(ExecStatus::Return(registers[RETURN_REGISTER].take()));
                }

                println!("returning from call");
            }
            ControlFlowInstr::Call {
                function,
                arg_count,
                dest,
            } => {
                let func = globals.func_table.get(function).unwrap().clone();

                if arg_count != func.arity {
                    panic!("wrong number of arguments");
                }

                println!("call func id: {function}");

                call_stack.call(func, dest);
                registers.set_base(dest);
            }
            ControlFlowInstr::JumpIfTrue { offset, condition } => {
                let condition = registers[condition].take().unwrap();
                if condition.is_true().unwrap() {
                    call_stack.jump(offset)?;
                }
            }
            _ => todo!(),
        }

        Ok(ExecStatus::Pending)
    }
}

#[derive(Default)]
pub struct LocalMemoryExecutor;

impl<I, V> MemoryExecutor<I, V> for LocalMemoryExecutor
where
    I: DataInstruction<V>,
    V: Value,
{
    async fn execute(
        &mut self,
        instr: MemoryInstr<V>,
        registers: &mut Registers<V>,
        _globals: &Globals<I, V>,
    ) -> Result<(), MemoryInstructionError> {
        match instr {
            MemoryInstr::LoadLiteral { value, dest } => {
                println!("load literal to {dest}");
                registers[dest] = Some(value);
            }
            MemoryInstr::Move { src, dest } => {
                println!("move {src} to {dest}");
                registers[dest] = registers[src].take();
            }
            MemoryInstr::Copy { src, dest } => {
                println!("copy {src} to {dest}");
                registers[dest] = registers[src].clone();
            }
            _ => todo!(),
        }

        Ok(())
    }
}
