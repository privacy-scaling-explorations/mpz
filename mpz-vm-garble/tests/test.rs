use std::sync::Arc;

use mpz_garble_core::{encoding_state, msg::GarbleMessage, ChaChaEncoder, EncodedValue, Encoder};
use mpz_vm_core::{
    executor::{
        local::{LocalCfExecutor, LocalMemoryExecutor},
        Executor,
    },
    machines::simple::{DataInstr, SimpleLocalExecutor},
    ControlFlowInstr, Function, Globals, Instr, MemoryInstr, Thread,
};
use mpz_vm_garble::Value;
use utils_aio::{
    duplex::{Duplex, MemoryDuplex},
    sink::IoSink,
};

type GeneratorExecutor<S> = Executor<
    DataInstr,
    Value<encoding_state::Full>,
    mpz_vm_garble::GeneratorExecutor<S>,
    LocalCfExecutor,
    LocalMemoryExecutor,
>;
type EvaluatorExecutor<S> = Executor<
    DataInstr,
    Value<encoding_state::Active>,
    mpz_vm_garble::EvaluatorExecutor<S>,
    LocalCfExecutor,
    LocalMemoryExecutor,
>;

fn build_adder<S: encoding_state::LabelState>() -> Function<DataInstr, Value<S>> {
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

fn build_main<S: encoding_state::LabelState>() -> Function<DataInstr, Value<S>> {
    let instr = [
        Instr::Memory(MemoryInstr::Copy { src: 1, dest: 4 }),
        Instr::Memory(MemoryInstr::Copy { src: 2, dest: 5 }),
        Instr::ControlFlow(ControlFlowInstr::Call {
            function: 0,
            arg_count: 2,
            dest: 3,
        }),
        Instr::Memory(MemoryInstr::Copy { src: 3, dest: 4 }),
        Instr::Memory(MemoryInstr::Copy { src: 2, dest: 5 }),
        Instr::ControlFlow(ControlFlowInstr::Call {
            function: 0,
            arg_count: 2,
            dest: 3,
        }),
        Instr::ControlFlow(ControlFlowInstr::Return { ret: 3 }),
    ];

    Function {
        name: None,
        arity: 2,
        instr: Arc::new(instr),
    }
}

async fn main() {
    let encoder = ChaChaEncoder::new([0u8; 32]);
    let (io_gen, io_ev) = MemoryDuplex::<GarbleMessage>::new();

    let a_encoding_full = EncodedValue::from(encoder.encode::<u8>(0));
    let b_encoding_full = EncodedValue::from(encoder.encode::<u8>(1));

    let a_encoding = a_encoding_full.select(2u8).unwrap();
    let b_encoding = b_encoding_full.select(3u8).unwrap();

    let gen_fut = async {
        let mut exec = GeneratorExecutor::new(
            mpz_vm_garble::GeneratorExecutor::new(io_gen, encoder.delta()),
            Default::default(),
            Default::default(),
        );

        let mut globals = Globals::default();
        let adder_id = globals
            .func_table
            .insert(build_adder::<encoding_state::Full>());

        let mut thread = Thread::<DataInstr, Value<encoding_state::Full>>::new(Arc::new(globals));

        thread.call_with_args(
            build_main(),
            [a_encoding_full.into(), b_encoding_full.into()],
        );

        let c_encoding_full = exec.execute(&mut thread).await.unwrap().unwrap();

        let Value::Encoded(c_encoding_full) = c_encoding_full else {
            panic!();
        };

        c_encoding_full
    };

    let ev_fut = async {
        let mut exec = EvaluatorExecutor::new(
            mpz_vm_garble::EvaluatorExecutor::new(io_ev),
            Default::default(),
            Default::default(),
        );

        let mut globals = Globals::default();
        let adder_id = globals
            .func_table
            .insert(build_adder::<encoding_state::Active>());

        let mut thread = Thread::<DataInstr, Value<encoding_state::Active>>::new(Arc::new(globals));

        thread.call_with_args(build_main(), [a_encoding.into(), b_encoding.into()]);

        let c_encoding = exec.execute(&mut thread).await.unwrap().unwrap();

        let Value::Encoded(c_encoding) = c_encoding else {
            panic!();
        };

        c_encoding
    };

    let (c_encoding_full, c_encoding) = futures::join!(gen_fut, ev_fut);

    let c = c_encoding.decode(&c_encoding_full.decoding()).unwrap();
    let c: u8 = c.try_into().unwrap();

    assert_eq!(c, 5u8);
}
