//! Composed IT-MAC state: per-register and per-global [`AuthValue`]s
//! plus per-byte [`Byte`]s for linear memory.
//!
//! [`AuthState`] is just the union of [`Registers<AuthValue>`] (used
//! for both the register file and the symbolic globals table) and
//! [`LinearMemory<Byte>`]. Frame layout is the caller's concern; this
//! type is a typed bag of authenticated values.

use mpz_fields::gf2_128::Gf2_128;

use crate::{
    auth::{AuthValue, Bit},
    memory::LinearMemory,
    registers::Registers,
};

/// Authenticated state for every symbolic register, symbolic global,
/// and tainted memory byte in the current run, over wire type `W`.
///
/// The accumulate/verify passes instantiate `W = Gf2_128` (IT-MAC wires);
/// the prover's cleartext commit pass uses `W = Gf2` (plaintext bits).
///
/// `regs` is frame-scoped; `globals` and `memory` are long-lived
/// (shared across calls), mirroring WASM's global and linear-memory
/// semantics.
#[derive(Debug, Clone)]
pub struct AuthState<W = Gf2_128> {
    pub regs: Registers<AuthValue<W>>,
    pub globals: Registers<AuthValue<W>>,
    pub memory: LinearMemory<W>,
}

impl<W: Copy> AuthState<W> {
    /// Create empty state whose memory uses `zero`/`one` as its public-0
    /// and public-1 wires.
    pub fn new(zero: Bit<W>, one: Bit<W>) -> Self {
        Self {
            regs: Registers::default(),
            globals: Registers::default(),
            memory: LinearMemory::new(zero, one),
        }
    }

    /// Flatten a layered seed (see [`Registers::layered`] /
    /// [`LinearMemory::layered`]) into a plain, owned state for carrying to the
    /// next chunk. A no-op when already plain.
    pub fn flatten(self) -> Self {
        Self {
            regs: self.regs.flatten(),
            globals: self.globals.flatten(),
            memory: self.memory.flatten(),
        }
    }
}
