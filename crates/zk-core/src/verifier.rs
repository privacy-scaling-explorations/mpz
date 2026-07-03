use blake3::Hasher;
use mpz_circuits::Context;
use mpz_fields::{
    Accumulator,
    gf2::Gf2,
    gf2_128::{Gf2_128, Gf2_128Accumulator},
};
use rand_core::RngCore;

use typenum::{Max, Maximum, U0, U1, Unsigned};

use crate::{
    Error, MAC_ONE, MAC_ZERO, Result, VerifierOutput,
    poly::{Degree, Expr, PolyContext, VerifierCoeffs, VerifierPoly},
    util::{draw_chi, set_lsb},
};

/// The verifier side of the zero-knowledge protocol.
///
/// A `Verifier` holds the global MAC key `delta`, the key and adjustment tapes,
/// and the installed challenge stream. It walks the circuits once, implementing
/// [`Context`] directly: every multiplication and assertion is folded into the
/// running check state as the circuits are evaluated. [`finish`](Self::finish)
/// yields a [`VerifierOutput`].
///
/// The caller masks `w` with the VOPE correlation
/// ([`vope_sender`](crate::vope_sender)) and accepts the prover's proof iff
/// `w == u + delta * v` and the assertion hashes match.
#[derive(Debug)]
pub struct Verifier<'a, R> {
    keys: &'a [Gf2_128],
    adjust: &'a [bool],
    delta: Gf2_128,
    key_one: Gf2_128,
    cursor: usize,
    assertions: Hasher,
    rng: R,
    xy: Gf2_128Accumulator,
    z: Gf2_128Accumulator,
    poly: VerifierPoly,
}

impl<'a, R> Verifier<'a, R> {
    /// Creates a new verifier with the global MAC key `delta`, drawing
    /// challenge weights from `rng`.
    ///
    /// `keys` is the tape of verifier keys, one entry per input bit and per
    /// AND gate, consumed in evaluation order. `adjust` is the corresponding
    /// tape of adjustment bits received from the prover as the commitment;
    /// each entry selects whether the matching key is offset by `delta`.
    ///
    /// When folding a sub-range of a trace, pass the sub-range's tape slices
    /// and seek `rng` to the sub-range's gate offset: the `w` outputs of the
    /// sub-ranges sum to the full trace's `w`. Each multiplication and each
    /// polynomial constraint ([`PolyContext::assert_zero`] of degree ≥ 1, or
    /// [`PolyContext::materialize`]) consumes 16 bytes of the stream.
    ///
    /// The polynomial check ([`VerifierPoly::check`]) needs the powers of
    /// `delta` ([`DeltaPowers`](crate::poly::DeltaPowers)); the accumulate pass
    /// itself does not, so the caller precomputes them once and supplies them
    /// at check time.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] if `keys` and `adjust` differ in length.
    pub fn new(delta: Gf2_128, keys: &'a [Gf2_128], adjust: &'a [bool], rng: R) -> Result<Self> {
        if keys.len() != adjust.len() {
            return Err(Error::tape_len("adjust", keys.len(), adjust.len()));
        }
        Ok(Self {
            keys,
            adjust,
            delta,
            key_one: MAC_ONE + delta,
            cursor: 0,
            assertions: Hasher::default(),
            rng,
            xy: Gf2_128Accumulator::zero(),
            z: Gf2_128Accumulator::zero(),
            poly: VerifierPoly::default(),
        })
    }

    /// Consumes the next input from the tapes and returns its verifier key.
    ///
    /// The key is offset by `delta` when the corresponding adjustment bit is
    /// set.
    ///
    /// # Panics
    ///
    /// Panics if the key or adjustment tape has been exhausted.
    pub fn input(&mut self) -> Gf2_128 {
        let i = self.cursor;
        let raw = *self.keys.get(i).expect("key tape exhausted during input");
        let adj = *self
            .adjust
            .get(i)
            .expect("adjust tape exhausted during input");
        let mut key = if adj { raw + self.delta } else { raw };
        set_lsb(&mut key, false);
        self.cursor = i + 1;
        key
    }

    /// Returns the verifier key for a public input wire carrying `bit`.
    ///
    /// Public inputs consume no tape entries, since their value is known to
    /// both parties.
    pub fn input_public(&self, bit: bool) -> Gf2_128 {
        if bit { self.key_one } else { MAC_ZERO }
    }

    /// Completes the accumulate phase, yielding a [`VerifierOutput`].
    ///
    /// The caller masks `w` with the VOPE correlation
    /// ([`vope_sender`](crate::vope_sender)) and accepts the prover's proof
    /// iff `w == u + delta * v`, the assertion hashes match, and the
    /// polynomial check passes ([`VerifierPoly::check`]).
    ///
    /// # Errors
    ///
    /// Returns [`Error`] if the number of consumed tape entries does not match
    /// the tape length, indicating the circuits drew fewer inputs and AND
    /// gates than the tape provides.
    pub fn finish(self) -> Result<VerifierOutput> {
        if self.cursor != self.adjust.len() {
            return Err(Error::tape_unconsumed(self.cursor, self.adjust.len()));
        }
        let w = self.xy.reduce() + self.delta * self.z.reduce();
        Ok(VerifierOutput {
            w,
            poly: self.poly,
            assertions: *self.assertions.finalize().as_bytes(),
        })
    }
}

impl<R: RngCore> Context for Verifier<'_, R> {
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
        let i = self.cursor;
        let mut key = *self
            .keys
            .get(i)
            .expect("key tape exhausted: circuit has more AND gates than the tape");
        let adj = *self
            .adjust
            .get(i)
            .expect("adjust tape exhausted: circuit has more AND gates than the tape");

        if adj {
            key = key + self.delta;
        }
        set_lsb(&mut key, false);
        self.cursor = i + 1;

        let chi = draw_chi(&mut self.rng);

        self.xy.add_product(a * b, chi);
        self.z.add_product(key, chi);

        key
    }

    fn constant(&mut self, v: Gf2) -> Gf2_128 {
        self.input_public(v.0)
    }

    fn assert_const(&mut self, v: Gf2_128, expected: Gf2) -> Result<()> {
        let mac = if expected.0 { v + self.delta } else { v };
        self.assertions.update(&mac.to_inner().to_le_bytes());

        Ok(())
    }
}

impl<R: RngCore> PolyContext for Verifier<'_, R> {
    type Coeffs = VerifierCoeffs;

    fn lift(&self, wire: Gf2_128) -> Expr<VerifierCoeffs, U1> {
        Expr::<VerifierCoeffs, U1>::lift_key(wire, self.delta)
    }

    fn lift_const(&self, value: Gf2) -> Expr<VerifierCoeffs, U0> {
        Expr::<VerifierCoeffs, U0>::constant(value, self.delta)
    }

    fn materialize<N>(&mut self, expr: Expr<VerifierCoeffs, N>) -> Gf2_128
    where
        N: Degree + Max<U1>,
        Maximum<N, U1>: Degree,
    {
        let wire = self.input();
        // Pin the fresh wire to the expression: `expr - wire == 0`.
        let constraint = expr - self.lift(wire);
        let chi = draw_chi(&mut self.rng);
        self.poly
            .fold_expr(&constraint, Maximum::<N, U1>::USIZE, chi);
        wire
    }

    fn assert_zero<N: Degree>(&mut self, expr: Expr<VerifierCoeffs, N>) -> Result<()> {
        let Self { poly, rng, .. } = self;
        poly.assert_expr(&expr, || draw_chi(&mut *rng))
    }
}
