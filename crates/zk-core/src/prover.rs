use blake3::Hasher;
use itybity::{GetBit, Lsb0};
use mpz_circuits::Context;
use mpz_fields::{
    Accumulator,
    gf2::Gf2,
    gf2_128::{Gf2_128, Gf2_128Accumulator},
};
use rand_core::RngCore;

use typenum::{Max, Maximum, U0, U1};

use crate::{
    Error, MAC_ONE, MAC_ZERO, ProverOutput, Result,
    poly::{Degree, Expr, PlainCoeffs, PolyContext, ProverCoeffs, ProverPoly},
    util::{draw_chi, lsb, set_lsb},
};

/// The prover's witness-pass circuit context.
#[derive(Debug)]
pub struct Witness<'a> {
    witness: &'a mut [bool],
    cursor: usize,
}

impl<'a> Witness<'a> {
    /// Creates a witness-pass context.
    pub fn new(witness: &'a mut [bool]) -> Self {
        Self { witness, cursor: 0 }
    }

    /// Consumes the next tape entry to commit a private input `value` and
    /// returns its cleartext wire.
    ///
    /// # Panics
    ///
    /// Panics if the witness tape has been exhausted.
    pub fn input(&mut self, value: Gf2) -> Gf2 {
        let i = self.cursor;
        let slot = self
            .witness
            .get_mut(i)
            .expect("witness tape exhausted during input");
        *slot ^= value.0;
        self.cursor = i + 1;
        value
    }

    /// Returns the wire for a public input `value`.
    ///
    /// Public inputs consume no tape entry, since their value is known to both
    /// parties.
    pub fn input_public(&self, value: Gf2) -> Gf2 {
        value
    }

    /// Completes the witness pass.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] if the number of consumed tape entries does not match
    /// the tape length, indicating the circuits drew fewer inputs and AND
    /// gates than the tape provides.
    pub fn finish(self) -> Result<()> {
        if self.cursor != self.witness.len() {
            return Err(Error::tape_unconsumed(self.cursor, self.witness.len()));
        }
        Ok(())
    }
}

impl PolyContext for Witness<'_> {
    /// Plaintext evaluation: an expression is just its cleartext value, so the
    /// witness pass compiles polynomial gadgets down to field operations.
    type Coeffs = PlainCoeffs<Gf2>;

    fn lift(&self, wire: Gf2) -> Expr<PlainCoeffs<Gf2>, U1> {
        Expr::new(wire)
    }

    fn lift_const(&self, value: Gf2) -> Expr<PlainCoeffs<Gf2>, U0> {
        Expr::new(value)
    }

    fn materialize<N>(&mut self, expr: Expr<PlainCoeffs<Gf2>, N>) -> Gf2
    where
        N: Degree + Max<U1>,
        Maximum<N, U1>: Degree,
    {
        self.input(expr.plain())
    }

    fn assert_zero<N: Degree>(&mut self, expr: Expr<PlainCoeffs<Gf2>, N>) -> Result<()> {
        // The check binding constraints into the proof is built during the
        // accumulate pass; here the check only surfaces witness bugs early.
        if expr.plain() != Gf2::ZERO {
            return Err(Error::assert());
        }
        Ok(())
    }
}

impl Context for Witness<'_> {
    type Error = Error;
    type Wire = Gf2;
    type Field = Gf2;

    fn add(&mut self, a: Gf2, b: Gf2) -> Gf2 {
        a + b
    }

    fn sub(&mut self, a: Gf2, b: Gf2) -> Gf2 {
        a - b
    }

    fn mul(&mut self, a: Gf2, b: Gf2) -> Gf2 {
        let z = a * b;
        let i = self.cursor;
        let slot = self
            .witness
            .get_mut(i)
            .expect("witness tape exhausted: circuit has more AND gates than the tape");
        *slot ^= z.0;
        self.cursor = i + 1;
        z
    }

    fn constant(&mut self, v: Gf2) -> Gf2 {
        self.input_public(v)
    }

    fn assert_const(&mut self, v: Gf2, expected: Gf2) -> Result<()> {
        // The hash binding assertions into the proof is built during the
        // accumulate pass; here the check only surfaces witness bugs early.
        if v != expected {
            return Err(Error::assert());
        }
        Ok(())
    }
}

/// The prover's accumulate-pass circuit context (pass 2).
///
/// Walks the circuits a second time after the [`Witness`] pass, over the real
/// MAC tape and an installed challenge stream, folding every multiplication and
/// assertion directly into the running proof state. The `u` and `v`
/// accumulators defer reduction to [`finish`](Self::finish), which yields a
/// [`ProverOutput`].
///
/// The caller masks `(u, v)` with the VOPE correlation
/// ([`vope_receiver`](crate::vope_receiver)) before sending the proof.
#[derive(Debug)]
pub struct Accumulate<'a, R> {
    macs: &'a [Gf2_128],
    cursor: usize,
    assertions: Hasher,
    rng: R,
    u: Gf2_128Accumulator,
    v: Gf2_128Accumulator,
    poly: ProverPoly,
}

impl<'a, R> Accumulate<'a, R> {
    /// Creates the accumulate context over the MAC tape, drawing challenge
    /// weights from `rng`.
    ///
    /// Used to fold a sub-range of a trace whose commitment was produced
    /// elsewhere: `macs` covers the sub-range's tape entries, and `rng` is
    /// positioned to the sub-range's gate offset. The `(u, v)` outputs of the
    /// sub-ranges sum to the full trace's `(u, v)`, so sub-ranges can be folded
    /// in parallel and combined by field addition.
    ///
    /// Each multiplication and each polynomial constraint
    /// ([`PolyContext::assert_zero`] of degree ≥ 1, or
    /// [`PolyContext::materialize`]) consumes 16 bytes of the stream, so `rng`
    /// must be positioned to match the trace evaluated.
    pub fn new(macs: &'a [Gf2_128], rng: R) -> Self {
        Self {
            macs,
            cursor: 0,
            assertions: Hasher::default(),
            rng,
            u: Gf2_128Accumulator::zero(),
            v: Gf2_128Accumulator::zero(),
            poly: ProverPoly::default(),
        }
    }

    /// Consumes the next tape entry for a private input `bit` and returns its
    /// authenticated wire.
    ///
    /// Inputs must be supplied in the same order as during the witness pass.
    ///
    /// # Panics
    ///
    /// Panics if the MAC tape has been exhausted.
    pub fn input(&mut self, bit: bool) -> Gf2_128 {
        let i = self.cursor;
        let mut mac = *self.macs.get(i).expect("mac tape exhausted during input");
        set_lsb(&mut mac, bit);
        self.cursor = i + 1;
        mac
    }

    /// Returns the authenticated wire for a public input `bit`.
    ///
    /// Public inputs consume no tape entry, since their value is known to both
    /// parties; the wire is a fixed constant determined by `bit`.
    pub fn input_public(&self, bit: bool) -> Gf2_128 {
        if bit { MAC_ONE } else { MAC_ZERO }
    }

    /// Completes the accumulate phase, yielding a [`ProverOutput`].
    ///
    /// The caller masks `(u, v)` with the VOPE correlation
    /// ([`vope_receiver`](crate::vope_receiver)) and the polynomial check
    /// coefficients ([`ProverPoly::coefficients`]) with the degree-`d_max`
    /// VOPE coefficients before sending the proof.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] if the number of consumed tape entries does not match
    /// the tape length, indicating the circuits drew fewer inputs and AND
    /// gates than the tape provides.
    pub fn finish(self) -> Result<ProverOutput> {
        if self.cursor != self.macs.len() {
            return Err(Error::tape_unconsumed(self.cursor, self.macs.len()));
        }
        Ok(ProverOutput {
            u: self.u.reduce(),
            v: self.v.reduce(),
            poly: self.poly,
            assertions: *self.assertions.finalize().as_bytes(),
        })
    }
}

impl<R: RngCore> Context for Accumulate<'_, R> {
    type Error = Error;
    type Wire = Gf2_128;
    type Field = Gf2;

    fn add(&mut self, a: Gf2_128, b: Gf2_128) -> Gf2_128 {
        a + b
    }

    fn sub(&mut self, a: Gf2_128, b: Gf2_128) -> Gf2_128 {
        a - b
    }

    fn mul(&mut self, a: Gf2_128, b: Gf2_128) -> Gf2_128 {
        let x = GetBit::<Lsb0>::get_bit(&a, 0);
        let y = GetBit::<Lsb0>::get_bit(&b, 0);
        let i = self.cursor;
        let mut mac = *self
            .macs
            .get(i)
            .expect("mac tape exhausted: circuit has more AND gates than the tape");
        set_lsb(&mut mac, x & y);
        self.cursor = i + 1;

        let chi = draw_chi(&mut self.rng);

        // `a_10 = b if lsb(a) else 0`, `a_11 = a if lsb(b) else 0`,
        // expressed as `a · w` with `w ∈ {0, u128::MAX}` so there
        // is no data-dependent branch.
        let w_x = (x as u128).wrapping_neg();
        let w_y = (y as u128).wrapping_neg();
        let body_v = Gf2_128::new(b.to_inner() & w_x) + Gf2_128::new(a.to_inner() & w_y) + mac;

        self.u.add_product(a * b, chi);
        self.v.add_product(body_v, chi);

        mac
    }

    fn constant(&mut self, v: Gf2) -> Gf2_128 {
        self.input_public(v.0)
    }

    fn assert_const(&mut self, v: Gf2_128, expected: Gf2) -> Result<()> {
        let got = GetBit::<Lsb0>::get_bit(&v, 0);
        if got != expected.0 {
            return Err(Error::assert());
        }

        self.assertions.update(&v.to_inner().to_le_bytes());

        Ok(())
    }
}

impl<R: RngCore> PolyContext for Accumulate<'_, R> {
    type Coeffs = ProverCoeffs<Gf2>;

    fn lift(&self, wire: Gf2_128) -> Expr<ProverCoeffs<Gf2>, U1> {
        // The wire's LSB carries its committed bit, so the top is read off it.
        Expr::<ProverCoeffs<Gf2>, U1>::lift_wire(wire, lsb(wire))
    }

    fn lift_const(&self, value: Gf2) -> Expr<ProverCoeffs<Gf2>, U0> {
        Expr::<ProverCoeffs<Gf2>, U0>::constant(value)
    }

    fn materialize<N>(&mut self, expr: Expr<ProverCoeffs<Gf2>, N>) -> Gf2_128
    where
        N: Degree + Max<U1>,
        Maximum<N, U1>: Degree,
    {
        let wire = self.input(expr.value().0);
        // Pin the fresh wire to the expression: `expr - wire == 0`. The
        // constraint's top coefficient is `expr.value + lsb(wire) = 0` by
        // construction, so it folds without a witness check.
        let constraint = expr - self.lift(wire);
        let chi = draw_chi(&mut self.rng);
        self.poly.fold_expr(&constraint, chi);
        wire
    }

    fn assert_zero<N: Degree>(&mut self, expr: Expr<ProverCoeffs<Gf2>, N>) -> Result<()> {
        let Self { poly, rng, .. } = self;
        poly.assert_expr(&expr, || draw_chi(&mut *rng))
    }
}
