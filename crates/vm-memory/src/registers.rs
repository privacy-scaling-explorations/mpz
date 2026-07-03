//! Sparse register file, layered for parallel seeding.
//!
//! A [`Registers`] is either a *plain* store — the carried chunk state, whose
//! `own` map holds every entry — or a *layered* worker seed: an empty `own`
//! overlay above a shared, read-only [`SharedRegs`] (the chunk-initial base
//! plus each segment boundary's delta). A worker writes only into `own`; reads
//! fall through `own` → newest visible delta → … → base. `own` is an [`Arc`]
//! so a plain store can be shared as a [`SharedRegs`] base without copying.
//!
//! Generic over the cell type so it can be reused for non-authenticated state
//! and unit-tested with primitives.

use std::{collections::HashMap, sync::Arc};

use mpz_vm_core::Reg;

/// One segment boundary's register delta: the registers it (re)wrote and the
/// ranges it reclaimed. A reclaim masks the layers below; a set overrides them.
#[derive(Debug, Clone)]
pub struct RegDelta<T> {
    pub sets: HashMap<Reg, T>,
    pub dropped: Vec<(Reg, u32)>,
}

/// Read-only register layers shared by every worker in a pass: the
/// chunk-initial base plus each boundary's [`RegDelta`], oldest first.
#[derive(Debug, Clone)]
pub struct SharedRegs<T> {
    base: Arc<HashMap<Reg, T>>,
    deltas: Vec<RegDelta<T>>,
}

impl<T> SharedRegs<T> {
    /// Begin a shared layer set from `base`'s entries, shared (not copied).
    pub fn new(base: &Registers<T>) -> Self {
        Self {
            base: Arc::clone(&base.own),
            deltas: Vec::new(),
        }
    }

    /// Append the next boundary's delta.
    pub fn push(&mut self, delta: RegDelta<T>) {
        self.deltas.push(delta);
    }

    /// Read `reg` across `deltas[..upto]` (newest first) then the base,
    /// honouring each delta's reclaims as masks.
    fn get(&self, reg: Reg, upto: usize) -> Option<&T> {
        for delta in self.deltas[..upto].iter().rev() {
            if let Some(v) = delta.sets.get(&reg) {
                return Some(v);
            }
            if covers(&delta.dropped, reg) {
                return None;
            }
        }
        self.base.get(&reg)
    }
}

/// Sparse register file keyed by absolute [`Reg`] index.
#[derive(Debug, Clone)]
pub struct Registers<T> {
    /// Entries written at this layer: the whole store when plain, the worker's
    /// own writes when layered. `Arc` so a plain store shares as a base for
    /// free.
    own: Arc<HashMap<Reg, T>>,
    /// Ranges reclaimed at this layer, masking the shared layers below. Empty
    /// for a plain store.
    own_dropped: Vec<(Reg, u32)>,
    /// The shared fallthrough and how many of its deltas this worker sees;
    /// `None` for a plain store.
    shared: Option<(Arc<SharedRegs<T>>, usize)>,
}

impl<T> Default for Registers<T> {
    fn default() -> Self {
        Self {
            own: Arc::new(HashMap::new()),
            own_dropped: Vec::new(),
            shared: None,
        }
    }
}

impl<T> Registers<T> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Borrow the value at `reg`, falling through to the shared layers.
    pub fn get(&self, reg: Reg) -> Option<&T> {
        if let Some(v) = self.own.get(&reg) {
            return Some(v);
        }
        if covers(&self.own_dropped, reg) {
            return None;
        }
        match &self.shared {
            Some((shared, upto)) => shared.get(reg, *upto),
            None => None,
        }
    }
}

impl<T: Clone> Registers<T> {
    /// A layered worker store: an empty overlay above `shared`, seeing its
    /// first `upto` deltas.
    pub fn layered(shared: Arc<SharedRegs<T>>, upto: usize) -> Self {
        Self {
            own: Arc::new(HashMap::new()),
            own_dropped: Vec::new(),
            shared: Some((shared, upto)),
        }
    }

    /// Set (or overwrite) the value at `reg` in this layer's overlay.
    pub fn set(&mut self, reg: Reg, value: T) {
        Arc::make_mut(&mut self.own).insert(reg, value);
    }

    /// Copy `src`'s value (read through the layers) to `dst`. No-op if `src`
    /// has no value.
    pub fn copy(&mut self, dst: Reg, src: Reg) {
        if let Some(v) = self.get(src).cloned() {
            self.set(dst, v);
        }
    }

    /// Drop every register in `[base, base + count)` from this layer, masking
    /// it in the shared layers below.
    pub fn drop_range(&mut self, base: Reg, count: u32) {
        let end = base.saturating_add(count);
        Arc::make_mut(&mut self.own).retain(|&r, _| r < base || r >= end);
        if self.shared.is_some() {
            self.own_dropped.push((base, count));
        }
    }

    /// Flatten a layered store into a plain one, materialising base + visible
    /// deltas + own overlay into a single map. A no-op for a plain store.
    pub fn flatten(self) -> Self {
        let Some((shared, upto)) = self.shared else {
            return self;
        };
        let mut map: HashMap<Reg, T> = (*shared.base).clone();
        for delta in &shared.deltas[..upto] {
            for &(b, c) in &delta.dropped {
                let end = b.saturating_add(c);
                map.retain(|&r, _| r < b || r >= end);
            }
            for (r, v) in &delta.sets {
                map.insert(*r, v.clone());
            }
        }
        for &(b, c) in &self.own_dropped {
            let end = b.saturating_add(c);
            map.retain(|&r, _| r < b || r >= end);
        }
        for (r, v) in self.own.iter() {
            map.insert(*r, v.clone());
        }
        Self {
            own: Arc::new(map),
            own_dropped: Vec::new(),
            shared: None,
        }
    }
}

/// Whether `reg` falls in any `[base, base + count)` range.
fn covers(ranges: &[(Reg, u32)], reg: Reg) -> bool {
    ranges
        .iter()
        .any(|&(base, count)| reg.0 >= base.0 && reg.0 < base.0 + count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_and_get() {
        let mut r: Registers<u32> = Registers::new();
        assert!(r.get(Reg(7)).is_none());
        r.set(Reg(7), 42);
        assert_eq!(r.get(Reg(7)), Some(&42));
        r.set(Reg(7), 99);
        assert_eq!(r.get(Reg(7)), Some(&99));
    }

    #[test]
    fn copy_from_missing_is_noop() {
        let mut r: Registers<u32> = Registers::new();
        r.set(Reg(2), 5);
        r.copy(Reg(2), Reg(1)); // src missing
        assert_eq!(r.get(Reg(2)), Some(&5));
        assert!(r.get(Reg(1)).is_none());
    }

    #[test]
    fn drop_range_is_half_open() {
        let mut r: Registers<u32> = Registers::new();
        for i in 0..10 {
            r.set(Reg(i), i * 10);
        }
        r.drop_range(Reg(3), 4); // drops 3,4,5,6
        for i in 0..3 {
            assert_eq!(r.get(Reg(i)), Some(&(i * 10)), "kept {i}");
        }
        for i in 3..7 {
            assert!(r.get(Reg(i)).is_none(), "dropped {i}");
        }
        for i in 7..10 {
            assert_eq!(r.get(Reg(i)), Some(&(i * 10)), "kept {i}");
        }
    }

    #[test]
    fn drop_range_zero_count() {
        let mut r: Registers<u32> = Registers::new();
        r.set(Reg(5), 50);
        r.drop_range(Reg(5), 0);
        assert_eq!(r.get(Reg(5)), Some(&50));
    }

    #[test]
    fn layered_reads_fall_through_and_mask() {
        let mut base: Registers<u32> = Registers::new();
        base.set(Reg(0), 1);
        base.set(Reg(1), 2);
        base.set(Reg(2), 3);

        let mut shared = SharedRegs::new(&base);
        // delta 0 overwrites reg 1, drops reg 2.
        shared.push(RegDelta {
            sets: HashMap::from([(Reg(1), 20)]),
            dropped: vec![(Reg(2), 1)],
        });
        // delta 1 re-sets reg 2.
        shared.push(RegDelta {
            sets: HashMap::from([(Reg(2), 30)]),
            dropped: vec![],
        });
        let shared = Arc::new(shared);

        // Worker sees only delta 0.
        let w0 = Registers::layered(shared.clone(), 1);
        assert_eq!(w0.get(Reg(0)), Some(&1)); // base
        assert_eq!(w0.get(Reg(1)), Some(&20)); // delta 0
        assert_eq!(w0.get(Reg(2)), None); // dropped by delta 0

        // Worker sees both deltas.
        let mut w1 = Registers::layered(shared, 2);
        assert_eq!(w1.get(Reg(2)), Some(&30)); // re-set by delta 1
        w1.set(Reg(0), 100); // own overlay wins
        assert_eq!(w1.get(Reg(0)), Some(&100));
        w1.drop_range(Reg(1), 1); // own drop masks delta 0
        assert_eq!(w1.get(Reg(1)), None);

        assert_eq!(w1.flatten().get(Reg(2)), Some(&30));
    }
}
