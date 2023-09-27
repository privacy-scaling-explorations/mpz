use mpz_circuits::{
    circuits::AES128,
    types::{StaticValueType, Value},
};
use mpz_garble_core::msg::GarbleMessage;
use mpz_ot::mock::mock_ot_shared_pair;
use utils_aio::duplex::MemoryDuplex;

use mpz_garble::{Evaluator, Generator, GeneratorConfigBuilder, ValueRegistry};

#[tokio::test]
async fn test_semi_honest() {
    let (mut gen_channel, mut ev_channel) = MemoryDuplex::<GarbleMessage>::new();
    let (ot_send, ot_recv) = mock_ot_shared_pair();

    let gen = Generator::new(
        GeneratorConfigBuilder::default().build().unwrap(),
        [0u8; 32],
    );
    let ev = Evaluator::default();

    let mut value_registry = ValueRegistry::default();

    let key = [69u8; 16];
    let msg = [42u8; 16];

    let key_ref = value_registry
        .add_value("key", <[u8; 16]>::value_type())
        .unwrap();
    let msg_ref = value_registry
        .add_value("msg", <[u8; 16]>::value_type())
        .unwrap();
    let ciphertext_ref = value_registry
        .add_value("ciphertext", <[u8; 16]>::value_type())
        .unwrap();

    let gen_fut = async {
        let key = key_ref
            .iter()
            .cloned()
            .zip(key)
            .map(|(k, v)| (k, Value::from(v)))
            .collect::<Vec<_>>();
        let msg = msg_ref
            .iter()
            .cloned()
            .map(|k| (k, u8::value_type()))
            .collect::<Vec<_>>();

        gen.setup_inputs("test", &[], &key, &msg, &mut gen_channel, &ot_send)
            .await
            .unwrap();

        gen.generate(
            AES128.clone(),
            &[key_ref.clone(), msg_ref.clone()],
            &[ciphertext_ref.clone()],
            &mut gen_channel,
            false,
        )
        .await
        .unwrap();
    };

    let ev_fut = async {
        let key = key_ref
            .iter()
            .cloned()
            .map(|k| (k, u8::value_type()))
            .collect::<Vec<_>>();
        let msg = msg_ref
            .iter()
            .cloned()
            .zip(msg)
            .map(|(k, v)| (k, Value::from(v)))
            .collect::<Vec<_>>();

        ev.setup_inputs("test", &[], &msg, &key, &mut ev_channel, &ot_recv)
            .await
            .unwrap();

        _ = ev
            .evaluate(
                AES128.clone(),
                &[key_ref.clone(), msg_ref.clone()],
                &[ciphertext_ref.clone()],
                &mut ev_channel,
            )
            .await
            .unwrap();
    };

    tokio::join!(gen_fut, ev_fut);

    let ciphertext_full_encoding = gen.get_encoding(&ciphertext_ref).unwrap();
    let ciphertext_active_encoding = ev.get_encoding(&ciphertext_ref).unwrap();

    let decoding = ciphertext_full_encoding.decoding();
    let ciphertext: [u8; 16] = ciphertext_active_encoding
        .decode(&decoding)
        .unwrap()
        .try_into()
        .unwrap();

    let expected: [u8; 16] = {
        use aes::{
            cipher::{BlockEncrypt, KeyInit},
            Aes128,
        };

        let mut msg = msg.into();

        let cipher = Aes128::new_from_slice(&key).unwrap();
        cipher.encrypt_block(&mut msg);

        msg.into()
    };

    assert_eq!(ciphertext, expected)
}
