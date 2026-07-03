//! An append-only log of memory accesses with a call-global clock.
//!
//! When enabled on the [`Global`](crate::Global), a [`Thread`](crate::Thread)
//! records one [`Access`] per completed memory access — including the public
//! stores that take the silent fast path and never surface as a
//! [`Directive`](crate::Directive). Threads record their own memory ops;
//! embedders record host-call writes they perform outside any op (e.g. a
//! precompile's in-place output) via
//! [`Global::log_host_write`](crate::Global::log_host_write). The log is shared
//! public structure: it is identical on every party, so it never carries a
//! symbolic address even on the party that locally holds it. An embedder (e.g.
//! a future RAM argument) drains the resident prefix once it has consumed it.
//!
//! The clock is implicit by position: there is no per-entry clock field. The
//! log tracks a [`base`](AccessLog::base) clock; the `i`-th resident entry has
//! clock `base + i`, and [`clock`](AccessLog::clock) is the clock the next
//! access will receive.

/// The effective address of a memory access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessAddr {
    /// A concrete effective byte address, known to all parties.
    Public(u32),
    /// A symbolic address: the address register is symbolic, so the effective
    /// address is deliberately kept out of the shared log — on both parties,
    /// including the one that locally holds it.
    Symbolic,
}

/// Whether a memory access reads from or writes to linear memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessKind {
    /// A load from memory.
    Read,
    /// A store to memory.
    Write,
}

/// A single recorded memory access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Access {
    /// Whether the access reads or writes.
    pub kind: AccessKind,
    /// The effective address, or [`AccessAddr::Symbolic`] when the address
    /// register is symbolic.
    pub addr: AccessAddr,
    /// The number of bytes accessed (1, 2, 4, or 8).
    pub width: u8,
    /// The per-byte symbolic mask: bit `i` is set iff byte `i` of the access is
    /// symbolic. Zero when the address is symbolic.
    pub symbolic_mask: u8,
    /// The written bytes assembled as a little-endian `u64` for a
    /// concrete-value write; `None` for reads and symbolic-value writes.
    pub value: Option<u64>,
    /// Whether this access also surfaced as a [`Directive`](crate::Directive):
    /// `false` on the silent fast paths, `true` on the directive paths.
    pub emitted: bool,
    /// Whether this is an embedder-initiated (host-call) write recorded via
    /// [`Global::log_host_write`](crate::Global::log_host_write), as opposed
    /// to an access performed by a thread memory op.
    pub host: bool,
}

/// An append-only log of memory accesses with a call-global clock.
///
/// Entries are appended in access order and drained from the front by the
/// embedder. See the [module documentation](self) for the clock model.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccessLog {
    base: u64,
    entries: Vec<Access>,
}

impl AccessLog {
    /// Creates an empty log whose clock starts at zero.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Returns the clock the next appended access will receive: `base +
    /// entries.len()`.
    pub fn clock(&self) -> u64 {
        self.base + self.entries.len() as u64
    }

    /// Returns the clock of the first resident entry, equal to
    /// [`clock`](Self::clock) when the log is empty.
    pub fn base(&self) -> u64 {
        self.base
    }

    /// Returns the resident suffix of entries; entry `i` has clock `base() +
    /// i`.
    pub fn entries(&self) -> &[Access] {
        &self.entries
    }

    /// Appends `access`, assigning it the current [`clock`](Self::clock).
    pub(crate) fn push(&mut self, access: Access) {
        self.entries.push(access);
    }

    /// Drops every resident entry whose clock is `< clock`, advancing
    /// [`base`](Self::base) accordingly.
    ///
    /// [`clock`](Self::clock) is unchanged: dropping entries never invents or
    /// forgets future clocks. A `clock` at or before [`base`](Self::base) drops
    /// nothing; a `clock` at or beyond the end drops every resident entry and
    /// leaves the log empty with `base == clock()`.
    pub fn drain_upto(&mut self, clock: u64) {
        let drop = clock
            .saturating_sub(self.base)
            .min(self.entries.len() as u64) as usize;
        self.entries.drain(..drop);
        self.base += drop as u64;
    }
}
