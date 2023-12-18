use futures::SinkExt;
use mpz_garble_core::{
    encoding_state, msg::GarbleMessage, Delta, EncodedValue, Evaluator, Generator,
};
use mpz_vm_core::{
    error::DataInstructionError, executor::DataExecutor, machines::simple::DataInstr, Registers,
};
use utils_aio::stream::{ExpectStreamExt, IoStream};

use crate::{circuits::ADD_U8, value::Value};

pub struct EvaluatorExecutor<S> {
    stream: S,
}

impl<S: IoStream<GarbleMessage> + Send + Unpin>
    DataExecutor<DataInstr, Value<encoding_state::Active>> for EvaluatorExecutor<S>
{
    async fn execute(
        &mut self,
        instr: DataInstr,
        registers: &mut Registers<Value<encoding_state::Active>>,
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

impl<S> EvaluatorExecutor<S> {
    pub fn new(stream: S) -> Self {
        Self { stream }
    }
}

impl<S> EvaluatorExecutor<S>
where
    S: IoStream<GarbleMessage> + Send + Unpin,
{
    async fn add(
        &mut self,
        a: EncodedValue<encoding_state::Active>,
        b: EncodedValue<encoding_state::Active>,
    ) -> Result<EncodedValue<encoding_state::Active>, DataInstructionError> {
        let circ = match (&a, &b) {
            (EncodedValue::U8(_), EncodedValue::U8(_)) => ADD_U8.clone(),
            _ => todo!(),
        };

        let mut ev = Evaluator::new(circ, &[a, b]).unwrap();

        let gates = self.stream.expect_next().await.unwrap();
        let GarbleMessage::EncryptedGates(gates) = gates else {
            panic!()
        };

        ev.evaluate(gates.iter());

        let mut outputs = ev.outputs().unwrap();
        let output = outputs.pop().unwrap();

        Ok(output)
    }
}
