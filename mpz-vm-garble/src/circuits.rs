use std::sync::Arc;

use mpz_circuits::{once_cell::sync::Lazy, ops::WrappingAdd, Circuit, CircuitBuilder};

pub(crate) static ADD_U8: Lazy<Arc<Circuit>> = Lazy::new(|| {
    let builder = CircuitBuilder::new();

    let a = builder.add_input::<u8>();
    let b = builder.add_input::<u8>();

    let c = a.wrapping_add(b);

    builder.add_output(c);

    Arc::new(builder.build().unwrap())
});
