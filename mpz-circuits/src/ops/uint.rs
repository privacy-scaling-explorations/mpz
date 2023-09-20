use std::ops::{BitAnd, BitOr, BitXor, Not};

use crate::{
    types::{BinaryRepr, U128, U16, U32, U64, U8},
    Tracer,
};

use super::{binary, WrappingAdd, WrappingSub};

macro_rules! impl_wrapping_add_uint {
    ($ty:ident, $const_ty:ident, $len:expr) => {
        impl<'a> WrappingAdd<Tracer<'a, $ty>> for Tracer<'a, $ty> {
            type Output = Tracer<'a, $ty>;

            fn wrapping_add(self, rhs: Tracer<'a, $ty>) -> Self::Output {
                let mut state = self.state.borrow_mut();

                let nodes = binary::const_wrapping_add_nbit::<$len>(
                    &mut state,
                    self.to_inner().nodes(),
                    rhs.to_inner().nodes(),
                );

                let value = <$ty>::new(nodes);

                drop(state);

                Tracer::new(self.state, value)
            }
        }
    };
}

impl_wrapping_add_uint!(U8, u8, 8);
impl_wrapping_add_uint!(U16, u16, 16);
impl_wrapping_add_uint!(U32, u32, 32);
impl_wrapping_add_uint!(U64, u64, 64);
impl_wrapping_add_uint!(U128, u128, 128);

macro_rules! impl_wrapping_sub_uint {
    ($ty:ident, $const_ty:ident, $len:expr) => {
        impl<'a> WrappingSub<Tracer<'a, $ty>> for Tracer<'a, $ty> {
            type Output = Tracer<'a, $ty>;

            fn wrapping_sub(self, rhs: Tracer<'a, $ty>) -> Self::Output {
                let mut state = self.state.borrow_mut();

                let (nodes, _) = binary::const_wrapping_sub_nbit::<$len>(
                    &mut state,
                    self.to_inner().nodes(),
                    rhs.to_inner().nodes(),
                );

                let value = <$ty>::new(nodes);

                drop(state);

                Tracer::new(self.state, value)
            }
        }
    };
}

impl_wrapping_sub_uint!(U8, u8, 8);
impl_wrapping_sub_uint!(U16, u16, 16);
impl_wrapping_sub_uint!(U32, u32, 32);
impl_wrapping_sub_uint!(U64, u64, 64);
impl_wrapping_sub_uint!(U128, u128, 128);

impl<'a> BitXor for Tracer<'a, BinaryRepr> {
    type Output = Tracer<'a, BinaryRepr>;

    fn bitxor(self, rhs: Self) -> Self::Output {
        match (self.value, rhs.value) {
            (BinaryRepr::Bit(a), BinaryRepr::Bit(b)) => {
                let c = Tracer::new(self.state, a) ^ Tracer::new(self.state, b);
                Tracer::new(self.state, c.into())
            }
            (BinaryRepr::U8(a), BinaryRepr::U8(b)) => {
                let c = Tracer::new(self.state, a) ^ Tracer::new(self.state, b);
                Tracer::new(self.state, c.into())
            }
            (BinaryRepr::U16(a), BinaryRepr::U16(b)) => {
                let c = Tracer::new(self.state, a) ^ Tracer::new(self.state, b);
                Tracer::new(self.state, c.into())
            }
            (BinaryRepr::U32(a), BinaryRepr::U32(b)) => {
                let c = Tracer::new(self.state, a) ^ Tracer::new(self.state, b);
                Tracer::new(self.state, c.into())
            }
            (BinaryRepr::U64(a), BinaryRepr::U64(b)) => {
                let c = Tracer::new(self.state, a) ^ Tracer::new(self.state, b);
                Tracer::new(self.state, c.into())
            }
            (BinaryRepr::U128(a), BinaryRepr::U128(b)) => {
                let c = Tracer::new(self.state, a) ^ Tracer::new(self.state, b);
                Tracer::new(self.state, c.into())
            }
            (BinaryRepr::Array(a), BinaryRepr::Array(b)) => Tracer::new(
                self.state,
                BinaryRepr::Array(
                    a.into_iter()
                        .zip(b)
                        .map(|(a, b)| {
                            let c = Tracer::new(self.state, a) ^ Tracer::new(self.state, b);
                            c.value
                        })
                        .collect(),
                ),
            ),
            (a, b) => panic!("types {:?} and {:?} are not compatible", a, b),
        }
    }
}

macro_rules! impl_bitxor_uint {
    ($ty:ident, $const_ty:ident, $len:expr) => {
        impl<'a> BitXor<Tracer<'a, $ty>> for Tracer<'a, $ty> {
            type Output = Tracer<'a, $ty>;

            fn bitxor(self, rhs: Tracer<'a, $ty>) -> Self::Output {
                let mut state = self.state.borrow_mut();

                let value = <$ty>::new(binary::xor_nbit::<$len>(
                    &mut state,
                    self.to_inner().nodes(),
                    rhs.to_inner().nodes(),
                ));

                drop(state);

                Tracer::new(self.state, value)
            }
        }
    };
}

impl_bitxor_uint!(U8, u8, 8);
impl_bitxor_uint!(U16, u16, 16);
impl_bitxor_uint!(U32, u32, 32);
impl_bitxor_uint!(U64, u64, 64);
impl_bitxor_uint!(U128, u128, 128);

macro_rules! impl_bit_and_uint {
    ($ty:ident, $const_ty:ident, $len:expr) => {
        impl<'a> BitAnd<Tracer<'a, $ty>> for Tracer<'a, $ty> {
            type Output = Tracer<'a, $ty>;

            fn bitand(self, rhs: Tracer<'a, $ty>) -> Self::Output {
                let mut state = self.state.borrow_mut();

                let value = <$ty>::new(binary::and_nbit::<$len>(
                    &mut state,
                    self.to_inner().nodes(),
                    rhs.to_inner().nodes(),
                ));

                drop(state);

                Tracer::new(self.state, value)
            }
        }
    };
}

impl_bit_and_uint!(U8, u8, 8);
impl_bit_and_uint!(U16, u16, 16);
impl_bit_and_uint!(U32, u32, 32);
impl_bit_and_uint!(U64, u64, 64);
impl_bit_and_uint!(U128, u128, 128);

macro_rules! impl_bit_or_uint {
    ($ty:ident, $const_ty:ident, $len:expr) => {
        impl<'a> BitOr<Tracer<'a, $ty>> for Tracer<'a, $ty> {
            type Output = Tracer<'a, $ty>;

            fn bitor(self, rhs: Tracer<'a, $ty>) -> Self::Output {
                let mut state = self.state.borrow_mut();

                let value = <$ty>::new(binary::or_nbit::<$len>(
                    &mut state,
                    self.to_inner().nodes(),
                    rhs.to_inner().nodes(),
                ));

                drop(state);

                Tracer::new(self.state, value)
            }
        }
    };
}

impl_bit_or_uint!(U8, u8, 8);
impl_bit_or_uint!(U16, u16, 16);
impl_bit_or_uint!(U32, u32, 32);
impl_bit_or_uint!(U64, u64, 64);
impl_bit_or_uint!(U128, u128, 128);

macro_rules! impl_neg_uint {
    ($ty:ident) => {
        impl<'a> Not for Tracer<'a, $ty> {
            type Output = Tracer<'a, $ty>;

            fn not(self) -> Self::Output {
                let mut state = self.state.borrow_mut();

                let value = <$ty>::new(binary::inv_nbit(&mut state, self.to_inner().nodes()));

                Tracer::new(self.state, value)
            }
        }
    };
}

impl_neg_uint!(U8);
impl_neg_uint!(U16);
impl_neg_uint!(U32);
impl_neg_uint!(U64);
impl_neg_uint!(U128);

macro_rules! impl_convert_bytes {
    ($ty:ident, $len:expr) => {
        impl<'a> Tracer<'a, $ty> {
            /// Create a value from its representation as a byte array in big endian.
            pub fn from_be_bytes(bytes: [Tracer<'a, U8>; $len]) -> Tracer<'a, $ty> {
                let state = bytes[0].state;
                Tracer::new(state, $ty::from_be_bytes(bytes.map(|byte| byte.to_inner())))
            }

            /// Returns the representation of this type as a byte array in big endian.
            pub fn to_be_bytes(self) -> [Tracer<'a, U8>; $len] {
                self.value
                    .to_be_bytes()
                    .map(|value| Tracer::new(self.state, value))
            }

            /// Create a value from its representation as a byte array in little endian.
            pub fn from_le_bytes(bytes: [Tracer<'a, U8>; $len]) -> Tracer<'a, $ty> {
                let state = bytes[0].state;
                Tracer::new(state, $ty::from_le_bytes(bytes.map(|byte| byte.to_inner())))
            }

            /// Returns the representation of this type as a byte array in little endian.
            pub fn to_le_bytes(self) -> [Tracer<'a, U8>; $len] {
                self.value
                    .to_le_bytes()
                    .map(|value| Tracer::new(self.state, value))
            }
        }
    };
}

impl_convert_bytes!(U8, 1);
impl_convert_bytes!(U16, 2);
impl_convert_bytes!(U32, 4);
impl_convert_bytes!(U64, 8);
impl_convert_bytes!(U128, 16);
