use std::ops::{Index, IndexMut};

/// The number of registers available in the virtual machine.
pub(crate) const REGISTER_CAPACITY: usize = 1024;
/// The number of registers available in a call frame.
pub(crate) const REGISTER_WINDOW: usize = 256;
/// The return register, which is always register 0.
pub(crate) const RETURN_REGISTER: RegisterId = 0;
/// The first argument register.
pub(crate) const ARGUMENT_REGISTER: RegisterId = 1;

/// A register id.
pub type RegisterId = u16;

pub struct Registers<V> {
    registers: Box<[Option<V>]>,
    base: RegisterId,
}

impl<V> Index<RegisterId> for Registers<V> {
    type Output = Option<V>;

    fn index(&self, index: RegisterId) -> &Self::Output {
        &self.registers[self.base as usize + index as usize]
    }
}

impl<V> IndexMut<RegisterId> for Registers<V> {
    fn index_mut(&mut self, index: RegisterId) -> &mut Self::Output {
        &mut self.registers[self.base as usize + index as usize]
    }
}

impl<V> Default for Registers<V> {
    fn default() -> Self {
        let registers: [_; REGISTER_CAPACITY] = core::array::from_fn(|_| None);
        Self {
            registers: Box::new(registers),
            base: 0,
        }
    }
}

impl<V> Registers<V> {
    pub fn set_base(&mut self, base: RegisterId) {
        self.base = base;
    }

    pub fn get_mut(&mut self) -> &mut [Option<V>] {
        &mut self.registers[self.base as usize..self.base as usize + REGISTER_WINDOW]
    }
}
