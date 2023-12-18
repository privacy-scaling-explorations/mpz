use futures::SinkExt;
use mpz_garble_core::{encoding_state, msg::GarbleMessage, Delta, EncodedValue, Generator};
use mpz_vm_core::{
    error::DataInstructionError, executor::DataExecutor, machines::simple::DataInstr, Registers,
};
use utils_aio::sink::IoSink;

use crate::{circuits::ADD_U8, value::Value};

pub struct GeneratorExecutor<S> {
    sink: S,
    delta: Delta,
}

impl<S: IoSink<GarbleMessage> + Send + Unpin> DataExecutor<DataInstr, Value<encoding_state::Full>>
    for GeneratorExecutor<S>
{
    async fn execute(
        &mut self,
        instr: DataInstr,
        registers: &mut Registers<Value<encoding_state::Full>>,
    ) -> Result<(), DataInstructionError> {
        match instr {
            DataInstr::Add { a, b, dest } => {
                let a = registers[a].take().unwrap();
                let b = registers[b].take().unwrap();

                match (a, b) {
                    (Value::Encoded(a), Value::Encoded(b)) => {
                        registers[dest] = Some(Value::Encoded(self.add(a, b).await?));
                    }
                    (Value::Plain(a), Value::Plain(b)) => {
                        registers[dest] = Some(Value::Plain(a.add(b)?));
                    }
                    _ => panic!("can not add encoded and plain values"),
                }
            }
            DataInstr::Eq { a, b, dest } => {
                let a = registers[a].take().unwrap();
                let b = registers[b].take().unwrap();

                println!("eq: a {:?}, b {:?}", a, b);

                todo!()
            }
            _ => todo!(),
        }

        Ok(())
    }
}

impl<S> GeneratorExecutor<S> {
    pub fn new(sink: S, delta: Delta) -> Self {
        Self { sink, delta }
    }
}

impl<S> GeneratorExecutor<S>
where
    S: IoSink<GarbleMessage> + Send + Unpin,
{
    async fn add(
        &mut self,
        a: EncodedValue<encoding_state::Full>,
        b: EncodedValue<encoding_state::Full>,
    ) -> Result<EncodedValue<encoding_state::Full>, DataInstructionError> {
        let circ = match (&a, &b) {
            (EncodedValue::U8(_), EncodedValue::U8(_)) => ADD_U8.clone(),
            _ => todo!(),
        };

        let mut gen = Generator::new(circ, self.delta, &[a, b]).unwrap();

        let gates = gen.by_ref().collect::<Vec<_>>();

        self.sink
            .send(GarbleMessage::EncryptedGates(gates))
            .await
            .unwrap();

        let mut outputs = gen.outputs().unwrap();
        let output = outputs.pop().unwrap();

        Ok(output)
    }
}
