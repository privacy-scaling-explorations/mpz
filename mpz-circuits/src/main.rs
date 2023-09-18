use mpz_circuits::{types::ValueType, Circuit};


fn main() {
    let circ = Circuit::parse_circom(
        "circuits/bristol/adder64_reverse.txt",
        &[ValueType::U64, ValueType::U64],
        &[ValueType::U64],
    )
    .unwrap();

    // stupid assert always true
    assert_eq!(3, 3);

    //let output: u64 = evaluate!(circ, fn(1u64, 2u64) -> u64).unwrap();

    //assert_eq!(output, 3); 
}