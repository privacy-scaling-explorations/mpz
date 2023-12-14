use std::sync::Arc;

use mpz_vm_core::{
    executor::{
        local::{LocalCfExecutor, LocalMemoryExecutor},
        Executor,
    },
    machines::simple::*,
    ControlFlowInstr, Function, Globals, Instr, MemoryInstr, Thread,
};

type SimpleExecutor =
    Executor<DataInstr, Value, SimpleLocalExecutor, LocalCfExecutor, LocalMemoryExecutor>;

fn build_adder() -> Function<DataInstr, Value> {
    let instr = [
        Instr::Data(DataInstr::Add {
            a: 1,
            b: 2,
            dest: 0,
        }),
        Instr::ControlFlow(ControlFlowInstr::Return { ret: 0 }),
    ];

    Function {
        name: None,
        arity: 2,
        instr: Arc::new(instr),
    }
}

#[test]
fn test_simple_machine() {
    futures::executor::block_on(main());
}

async fn main() {
    let mut exec = SimpleExecutor::default();

    let mut globals = Globals::default();
    let adder_id = globals.func_table.insert(build_adder());

    let mut thread = Thread::<DataInstr, Value>::new(Arc::new(globals));

    let instr = [
        Instr::Memory(MemoryInstr::Copy { src: 1, dest: 3 }),
        Instr::Memory(MemoryInstr::Copy { src: 2, dest: 4 }),
        Instr::ControlFlow(ControlFlowInstr::Call {
            function: adder_id,
            arg_count: 2,
            dest: 2,
        }),
        Instr::Memory(MemoryInstr::Copy { src: 2, dest: 3 }),
        Instr::Memory(MemoryInstr::LoadLiteral {
            value: Value::U8(5),
            dest: 4,
        }),
        Instr::Data(DataInstr::Eq {
            a: 3,
            b: 4,
            dest: 3,
        }),
        Instr::ControlFlow(ControlFlowInstr::JumpIfTrue {
            offset: 3,
            condition: 3,
        }),
        Instr::Memory(MemoryInstr::Copy { src: 1, dest: 3 }),
        Instr::Memory(MemoryInstr::Copy { src: 2, dest: 4 }),
        Instr::ControlFlow(ControlFlowInstr::Call {
            function: adder_id,
            arg_count: 2,
            dest: 2,
        }),
        Instr::ControlFlow(ControlFlowInstr::Return { ret: 2 }),
    ];

    let main_func = Function {
        name: None,
        arity: 2,
        instr: Arc::new(instr),
    };

    thread.call_with_args(main_func, [Value::U8(1), Value::U8(4)]);

    let result = exec.execute(&mut thread).await.unwrap();

    println!("result: {:?}", result);
}
