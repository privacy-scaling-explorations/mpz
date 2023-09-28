//! Autodetection for CPU intrinsics, with fallback to the "soft" backend when
//! they are unavailable.

use cfg_if::cfg_if;
use core::ops::{BitXor, BitXorAssign};

#[allow(clippy::duplicate_mod)]
#[cfg_attr(not(target_pointer_width = "64"), path = "backend/soft32.rs")]
#[cfg_attr(target_pointer_width = "64", path = "backend/soft64.rs")]
mod soft;

impl soft::Clmul {
    /// Reduces the polynomial represented in bits modulo the GCM polynomial x^128 + x^7 + x^2 + x + 1.
    /// x and y are resp. upper and lower bits of the polynomial.
    ///
    /// Page 16 of [Intel® Carry-Less Multiplication Instruction and its Usage for Computing the GCM Mode rev 2.02]
    /// (https://www.intel.com/content/dam/develop/external/us/en/documents/clmul-wp-rev-2-02-2014-04-20.pdf)
    pub fn reduce_gcm(x: Self, y: Self) -> Self {
        fn sep(x: u128) -> (u64, u64) {
            // (high, low)
            ((x >> 64) as u64, x as u64)
        }
        fn join(u: u64, l: u64) -> u128 {
            ((u as u128) << 64) | (l as u128)
        }

        let x: u128 = bytemuck::cast(x);
        let y: u128 = bytemuck::cast(y);

        let (x3, x2) = sep(y);
        let (x1, x0) = sep(x);
        let a = x3 >> 63;
        let b = x3 >> 62;
        let c = x3 >> 57;
        let d = x2 ^ a ^ b ^ c;
        let (e1, e0) = sep(join(x3, d) << 1);
        let (f1, f0) = sep(join(x3, d) << 2);
        let (g1, g0) = sep(join(x3, d) << 7);
        let h1 = x3 ^ e1 ^ f1 ^ g1;
        let h0 = d ^ e0 ^ f0 ^ g0;
        bytemuck::cast(join(x1 ^ h1, x0 ^ h0))
    }
}

cfg_if! {
    if #[cfg(all(target_arch = "aarch64", feature = "armv8"))] {
        #[path = "backend/pmull.rs"]
        mod pmull;
        use pmull as intrinsics;
        cpufeatures::new!(mul_intrinsics, "aes"); // `aes` implies PMULL
    } else if #[cfg(any(target_arch = "x86_64", target_arch = "x86"))] {
        #[path = "backend/clmul.rs"]
        mod clmul_intr;
        use clmul_intr as intrinsics;
        cpufeatures::new!(mul_intrinsics, "pclmulqdq");
    }
}

pub struct Clmul {
    inner: Inner,
    is_intr: bool,
}

union Inner {
    intrinsics: intrinsics::Clmul,
    soft: soft::Clmul,
}

/*
cfg_if! {
    if #[cfg(any(all(target_arch = "aarch64", feature = "armv8"), any(target_arch = "x86_64", target_arch = "x86")))]{
        #[derive(Clone, Copy, Debug)]
        /// Carryless multiplication
        pub struct Clmul {
            intrinsics: Option<intrinsics::Clmul>,
            soft: Option<soft::Clmul>,
        }
    } else {
        #[derive(Clone, Copy, Debug)]
        /// Carryless multiplication
        pub struct Clmul {
            // intrinsics will never be used on a non-supported arch but Rust
            // won't allow to declare it with a None type, so we need to
            // provide some type
            intrinsics: Option<soft::Clmul>,
            soft: Option<soft::Clmul>,
        }
    }
}
*/

// #[derive(Clone, Copy)]
// pub struct Clmul {
//     intrinsics: Option<intrinsics::Clmul>,
//     soft: Option<soft::Clmul>,
// }

impl Clmul {
    pub fn new(h: &[u8; 16]) -> Self {
        cfg_if! {
            if #[cfg(feature = "force-soft")] {
                Self {
                    inner: Inner { soft: soft::Clmul::new(h) },
                    is_intr: false,
                }
            } else if #[cfg(any(all(target_arch = "aarch64", feature = "armv8"), any(target_arch = "x86_64", target_arch = "x86")))]{
                if mul_intrinsics::get() {
                    Self {
                        inner: Inner { intrinsics: intrinsics::Clmul::new(h) },
                        is_intr: true,
                    }
                } else {
                    // supported arch was found but intrinsics are not available
                    Self {
                        inner: Inner { soft: soft::Clmul::new(h) },
                        is_intr: false,
                    }
                }
            } else {
                // "force-soft" feature was not enabled but neither was
                //  supported arch found. Falling back to soft backend.
                Self {
                    inner: Inner { soft: soft::Clmul::new(h) },
                    is_intr: false,
                }
            }
        }
    }

    /// Performs carryless multiplication
    pub fn clmul(self, x: Self) -> (Self, Self) {
        unsafe {
            match (self.is_intr, x.is_intr) {
                (true, true) => {
                    let s_intr = self.inner.intrinsics;
                    let x_intr = x.inner.intrinsics;

                    let (r0, r1) = s_intr.clmul(x_intr);
                    (
                        Self {
                            inner: Inner { intrinsics: r0 },
                            is_intr: true,
                        },
                        Self {
                            inner: Inner { intrinsics: r1 },
                            is_intr: true,
                        },
                    )
                }
                (false, false) => {
                    let s_soft = self.inner.soft;
                    let x_soft = x.inner.soft;

                    let (r0, r1) = s_soft.clmul(x_soft);
                    (
                        Self {
                            inner: Inner { soft: r0 },
                            is_intr: false,
                        },
                        Self {
                            inner: Inner { soft: r1 },
                            is_intr: false,
                        },
                    )
                }
                _ => unreachable!(),
            }
        }
    }

    /// Performs carryless multiplication. Same as clmul() but reusing the
    /// operands to return the result. This gives a ~6x speed up compared
    /// to clmul() where we create new objects containing the result.
    /// The high bits will be placed in `self`, the low bits - in `x`.
    pub fn clmul_reuse(&mut self, x: &mut Self) {
        unsafe {
            match (self.is_intr, x.is_intr) {
                (true, true) => {
                    let s_intr = self.inner.intrinsics;
                    let x_intr = x.inner.intrinsics;

                    let (r0, r1) = s_intr.clmul(x_intr);
                    self.inner.intrinsics = r0;
                    x.inner.intrinsics = r1;
                }
                (false, false) => {
                    let s_soft = self.inner.soft;
                    let x_soft = x.inner.soft;

                    let (r0, r1) = s_soft.clmul(x_soft);
                    self.inner.soft = r0;
                    x.inner.soft = r1;
                }
                _ => unreachable!(),
            }
        }
    }

    /// Reduces the polynomial represented in bits modulo the GCM polynomial x^128 + x^7 + x^2 + x + 1.
    /// x and y are resp. upper and lower bits of the polynomial.
    pub fn reduce_gcm(x: Self, y: Self) -> Self {
        unsafe {
            match (x.is_intr, y.is_intr) {
                (true, true) => {
                    let x_intr = x.inner.intrinsics;
                    let y_intr = y.inner.intrinsics;

                    cfg_if! {
                        if #[cfg(any(all(target_arch = "aarch64", feature = "armv8"), any(target_arch = "x86_64", target_arch = "x86")))]{
                            let r = intrinsics::Clmul::reduce_gcm(x_intr, y_intr);
                        } else {
                            let r = soft::Clmul::reduce_gcm(x_intr, y_intr);
                        }
                    }
                    Self {
                        inner: Inner { intrinsics: r },
                        is_intr: true,
                    }
                }
                (false, false) => {
                    let x_soft = x.inner.soft;
                    let y_soft = y.inner.soft;

                    let r = soft::Clmul::reduce_gcm(x_soft, y_soft);
                    Self {
                        inner: Inner { soft: r },
                        is_intr: false,
                    }
                }
                _ => unreachable!(),
            }
        }
    }
}

impl From<Clmul> for [u8; 16] {
    #[inline]
    fn from(m: Clmul) -> [u8; 16] {
        unsafe {
            if m.is_intr {
                m.inner.intrinsics.into()
            } else {
                m.inner.soft.into()
            }
        }
    }
}

impl BitXor for Clmul {
    type Output = Self;

    #[inline]
    fn bitxor(self, other: Self) -> Self::Output {
        unsafe {
            match (self.is_intr, other.is_intr) {
                (true, true) => {
                    let a = self.inner.intrinsics;
                    let b = other.inner.intrinsics;
                    Self {
                        inner: Inner { intrinsics: a ^ b },
                        is_intr: true,
                    }
                }
                (false, false) => {
                    let a = self.inner.soft;
                    let b = other.inner.soft;
                    Self {
                        inner: Inner { soft: a ^ b },
                        is_intr: false,
                    }
                }
                _ => unreachable!(),
            }
        }
    }
}

impl BitXorAssign for Clmul {
    #[inline]
    fn bitxor_assign(&mut self, other: Self) {
        unsafe {
            match (self.is_intr, other.is_intr) {
                (true, true) => {
                    let a = self.inner.intrinsics;
                    let b = other.inner.intrinsics;
                    self.inner.intrinsics = a ^ b;
                }
                (false, false) => {
                    let a = self.inner.soft;
                    let b = other.inner.soft;
                    self.inner.soft = a ^ b;
                }
                _ => unreachable!(),
            }
        }
    }
}

impl PartialEq for Clmul {
    fn eq(&self, other: &Self) -> bool {
        unsafe {
            match (self.is_intr, other.is_intr) {
                (true, true) => self.inner.intrinsics == other.inner.intrinsics,
                (false, false) => self.inner.soft == other.inner.soft,
                _ => unreachable!(),
            }
        }
    }
}

#[test]
fn reduce_test() {
    use rand::Rng;
    use rand_chacha::{rand_core::SeedableRng, ChaCha12Rng};

    let mut rng = ChaCha12Rng::from_seed([0; 32]);
    let x: [u8; 16] = rng.gen();
    let y: [u8; 16] = rng.gen();

    let xx = soft::Clmul::new(&x);
    let yy = soft::Clmul::new(&y);

    let zz = soft::Clmul::reduce_gcm(xx, yy);
    let zz: [u8; 16] = zz.into();

    let xxx = Clmul::new(&x);
    let yyy = Clmul::new(&y);

    let zzz = Clmul::reduce_gcm(xxx, yyy);
    let zzz: [u8; 16] = zzz.into();

    assert_eq!(zz, zzz);
}
