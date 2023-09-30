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

#[derive(Clone, Copy)]
/// Carryless multiplication
pub struct Clmul {
    inner: Inner,
    token: mul_intrinsics::InitToken,
}

#[derive(Clone, Copy)]
union Inner {
    intrinsics: intrinsics::Clmul,
    soft: soft::Clmul,
}

impl core::fmt::Debug for Clmul {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        unsafe {
            if self.token.get() {
                self.inner.intrinsics.fmt(f)
            } else {
                self.inner.soft.fmt(f)
            }
        }
    }
}

impl Clmul {
    pub fn new(h: &[u8; 16]) -> Self {
        let (token, has_intrinsics) = mul_intrinsics::init_get();

        let inner = if cfg!(feature = "force-soft") {
            Inner {
                soft: soft::Clmul::new(h),
            }
        } else if has_intrinsics {
            Inner {
                intrinsics: intrinsics::Clmul::new(h),
            }
        } else {
            Inner {
                soft: soft::Clmul::new(h),
            }
        };

        Self { inner, token }
    }

    /// Performs carryless multiplication
    pub fn clmul(self, x: Self) -> (Self, Self) {
        unsafe {
            let (in0, in1) = if self.token.get() {
                let s_intr = self.inner.intrinsics;
                let x_intr = x.inner.intrinsics;

                let (r0, r1) = s_intr.clmul(x_intr);
                (Inner { intrinsics: r0 }, Inner { intrinsics: r1 })
            } else {
                let s_soft = self.inner.soft;
                let x_soft = x.inner.soft;

                let (r0, r1) = s_soft.clmul(x_soft);
                (Inner { soft: r0 }, Inner { soft: r1 })
            };

            (
                Self {
                    inner: in0,
                    token: self.token,
                },
                Self {
                    inner: in1,
                    token: x.token,
                },
            )
        }
    }

    /// Performs carryless multiplication. Same as clmul() but reusing the
    /// operands to return the result. This gives a ~6x speed up compared
    /// to clmul() where we create new objects containing the result.
    /// The high bits will be placed in `self`, the low bits - in `x`.
    pub fn clmul_reuse(&mut self, x: &mut Self) {
        unsafe {
            if self.token.get() {
                let s_intr = self.inner.intrinsics;
                let x_intr = x.inner.intrinsics;

                let (r0, r1) = s_intr.clmul(x_intr);
                self.inner.intrinsics = r0;
                x.inner.intrinsics = r1;
            } else {
                let s_soft = self.inner.soft;
                let x_soft = x.inner.soft;

                let (r0, r1) = s_soft.clmul(x_soft);
                self.inner.soft = r0;
                x.inner.soft = r1;
            }
        }
    }

    /// Reduces the polynomial represented in bits modulo the GCM polynomial x^128 + x^7 + x^2 + x + 1.
    /// x and y are resp. upper and lower bits of the polynomial.
    pub fn reduce_gcm(x: Self, y: Self) -> Self {
        unsafe {
            if x.token.get() {
                let x_intr = x.inner.intrinsics;
                let y_intr = y.inner.intrinsics;

                let r = intrinsics::Clmul::reduce_gcm(x_intr, y_intr);
                Self {
                    inner: Inner { intrinsics: r },
                    token: x.token,
                }
            } else {
                let x_soft = x.inner.soft;
                let y_soft = y.inner.soft;

                let r = soft::Clmul::reduce_gcm(x_soft, y_soft);
                Self {
                    inner: Inner { soft: r },
                    token: x.token,
                }
            }
        }
    }
}

impl From<Clmul> for [u8; 16] {
    #[inline]
    fn from(m: Clmul) -> [u8; 16] {
        unsafe {
            if m.token.get() {
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
            let inner = if self.token.get() {
                let a = self.inner.intrinsics;
                let b = other.inner.intrinsics;
                Inner { intrinsics: a ^ b }
            } else {
                let a = self.inner.soft;
                let b = other.inner.soft;
                Inner { soft: a ^ b }
            };

            Self {
                inner,
                token: self.token,
            }
        }
    }
}

impl BitXorAssign for Clmul {
    #[inline]
    fn bitxor_assign(&mut self, other: Self) {
        unsafe {
            if self.token.get() {
                let a = self.inner.intrinsics;
                let b = other.inner.intrinsics;
                self.inner.intrinsics = a ^ b;
            } else {
                let a = self.inner.soft;
                let b = other.inner.soft;
                self.inner.soft = a ^ b;
            }
        }
    }
}

impl PartialEq for Clmul {
    fn eq(&self, other: &Self) -> bool {
        unsafe {
            if self.token.get() {
                self.inner.intrinsics == other.inner.intrinsics
            } else {
                self.inner.soft == other.inner.soft
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
