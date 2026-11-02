//! Dyadic numbers `m·2^e` and closed intervals of them: the rigorous arithmetic every enclosure
//! in this crate is computed in. No operation here rounds silently — an exact result stays exact,
//! and every rounding names its direction (`floor_to`, `ceil_to`, `round_out`).

use crate::numeric::{BigInt, Rational};
use std::cmp::Ordering;

pub(crate) fn one() -> BigInt {
    BigInt::from_i64(1)
}

/// `⌊m / 2^k⌋` for any sign.
pub(crate) fn floor_shr(m: &BigInt, k: u64) -> BigInt {
    if m.is_negative() {
        let up = m.negated().add(&one().shl_bits(k)).sub(&one());
        up.shr_bits(k).expect("nonnegative").negated()
    } else {
        m.shr_bits(k).expect("nonnegative")
    }
}

/// `⌈m / 2^k⌉` for any sign.
pub(crate) fn ceil_shr(m: &BigInt, k: u64) -> BigInt {
    floor_shr(&m.negated(), k).negated()
}

/// `⌊a / b⌋` for `b > 0`.
pub(crate) fn floor_div(a: &BigInt, b: &BigInt) -> BigInt {
    let (q, r) = a.div_rem(b).expect("b is nonzero");
    if r.is_negative() {
        q.sub(&one())
    } else {
        q
    }
}

/// `⌈a / b⌉` for `b > 0`.
pub(crate) fn ceil_div(a: &BigInt, b: &BigInt) -> BigInt {
    floor_div(&a.negated(), b).negated()
}

/// `m·2^e`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Dyadic {
    pub(crate) m: BigInt,
    pub(crate) e: i64,
}

impl Dyadic {
    pub(crate) fn zero() -> Dyadic {
        Dyadic {
            m: BigInt::zero(),
            e: 0,
        }
    }

    pub(crate) fn int(n: i64) -> Dyadic {
        Dyadic {
            m: BigInt::from_i64(n),
            e: 0,
        }
    }

    pub(crate) fn new(m: BigInt, e: i64) -> Dyadic {
        Dyadic { m, e }
    }

    pub(crate) fn align(&self, o: &Dyadic) -> (BigInt, BigInt, i64) {
        let e = self.e.min(o.e);
        (
            self.m.shl_bits((self.e - e) as u64),
            o.m.shl_bits((o.e - e) as u64),
            e,
        )
    }

    pub(crate) fn add(&self, o: &Dyadic) -> Dyadic {
        let (a, b, e) = self.align(o);
        Dyadic::new(a.add(&b), e)
    }

    pub(crate) fn sub(&self, o: &Dyadic) -> Dyadic {
        let (a, b, e) = self.align(o);
        Dyadic::new(a.sub(&b), e)
    }

    pub(crate) fn mul(&self, o: &Dyadic) -> Dyadic {
        Dyadic::new(self.m.mul(&o.m), self.e + o.e)
    }

    pub(crate) fn neg(&self) -> Dyadic {
        Dyadic::new(self.m.negated(), self.e)
    }

    pub(crate) fn shift(&self, k: i64) -> Dyadic {
        Dyadic::new(self.m.clone(), self.e + k)
    }

    pub(crate) fn cmp(&self, o: &Dyadic) -> Ordering {
        let (a, b, _) = self.align(o);
        a.cmp(&b)
    }

    pub(crate) fn is_negative(&self) -> bool {
        self.m.is_negative()
    }

    pub(crate) fn is_positive(&self) -> bool {
        self.m.is_positive()
    }

    pub(crate) fn min(self, o: Dyadic) -> Dyadic {
        if self.cmp(&o) == Ordering::Greater {
            o
        } else {
            self
        }
    }

    pub(crate) fn max(self, o: Dyadic) -> Dyadic {
        if self.cmp(&o) == Ordering::Less {
            o
        } else {
            self
        }
    }

    /// Round down onto the `2^-p` grid.
    pub(crate) fn floor_to(&self, p: i64) -> Dyadic {
        if self.e >= -p {
            return self.clone();
        }
        Dyadic::new(floor_shr(&self.m, (-p - self.e) as u64), -p)
    }

    /// Round up onto the `2^-p` grid.
    pub(crate) fn ceil_to(&self, p: i64) -> Dyadic {
        if self.e >= -p {
            return self.clone();
        }
        Dyadic::new(ceil_shr(&self.m, (-p - self.e) as u64), -p)
    }

    /// `|self| < 2^bits`.
    pub(crate) fn mag_bits(&self) -> i64 {
        if self.m.is_zero() {
            return i64::MIN / 4;
        }
        self.m.bit_len() as i64 + self.e
    }

    /// The integer `⌊self·2^p⌋`.
    pub(crate) fn floor_scaled(&self, p: i64) -> BigInt {
        let k = self.e + p;
        if k >= 0 {
            self.m.shl_bits(k as u64)
        } else {
            floor_shr(&self.m, (-k) as u64)
        }
    }

    /// The integer `⌈self·2^p⌉`.
    pub(crate) fn ceil_scaled(&self, p: i64) -> BigInt {
        let k = self.e + p;
        if k >= 0 {
            self.m.shl_bits(k as u64)
        } else {
            ceil_shr(&self.m, (-k) as u64)
        }
    }

    pub(crate) fn to_rational(&self) -> Rational {
        if self.e >= 0 {
            Rational::from_bigint(self.m.shl_bits(self.e as u64))
        } else {
            Rational::new(self.m.clone(), one().shl_bits((-self.e) as u64)).expect("nonzero")
        }
    }

    pub(crate) fn from_rational_floor(r: &Rational, p: i64) -> Dyadic {
        let num = if p >= 0 {
            r.numerator().shl_bits(p as u64)
        } else {
            r.numerator().clone()
        };
        let den = if p >= 0 {
            r.denominator().clone()
        } else {
            r.denominator().shl_bits((-p) as u64)
        };
        Dyadic::new(floor_div(&num, &den), -p)
    }

    pub(crate) fn from_rational_ceil(r: &Rational, p: i64) -> Dyadic {
        Dyadic::from_rational_floor(&r.negated(), p).neg()
    }
}

/// A closed dyadic interval.
#[derive(Clone, Debug)]
pub(crate) struct Iv {
    pub(crate) lo: Dyadic,
    pub(crate) hi: Dyadic,
}

impl Iv {
    pub(crate) fn point(d: Dyadic) -> Iv {
        Iv {
            lo: d.clone(),
            hi: d,
        }
    }

    pub(crate) fn width(&self) -> Dyadic {
        self.hi.sub(&self.lo)
    }

    pub(crate) fn round_out(&self, p: i64) -> Iv {
        Iv {
            lo: self.lo.floor_to(p),
            hi: self.hi.ceil_to(p),
        }
    }

    pub(crate) fn add(&self, o: &Iv) -> Iv {
        Iv {
            lo: self.lo.add(&o.lo),
            hi: self.hi.add(&o.hi),
        }
    }

    pub(crate) fn neg(&self) -> Iv {
        Iv {
            lo: self.hi.neg(),
            hi: self.lo.neg(),
        }
    }

    pub(crate) fn mul(&self, o: &Iv) -> Iv {
        let ps = [
            self.lo.mul(&o.lo),
            self.lo.mul(&o.hi),
            self.hi.mul(&o.lo),
            self.hi.mul(&o.hi),
        ];
        let mut lo = ps[0].clone();
        let mut hi = ps[0].clone();
        for p in &ps[1..] {
            lo = lo.min(p.clone());
            hi = hi.max(p.clone());
        }
        Iv { lo, hi }
    }

    pub(crate) fn sq(&self) -> Iv {
        if !self.lo.is_negative() {
            Iv {
                lo: self.lo.mul(&self.lo),
                hi: self.hi.mul(&self.hi),
            }
        } else if !self.hi.is_positive() {
            Iv {
                lo: self.hi.mul(&self.hi),
                hi: self.lo.mul(&self.lo),
            }
        } else {
            let a = self.lo.mul(&self.lo);
            let b = self.hi.mul(&self.hi);
            Iv {
                lo: Dyadic::zero(),
                hi: a.max(b),
            }
        }
    }

    pub(crate) fn widen(&self, r: &Dyadic) -> Iv {
        Iv {
            lo: self.lo.sub(r),
            hi: self.hi.add(r),
        }
    }

    pub(crate) fn contains_zero(&self) -> bool {
        !self.lo.is_positive() && !self.hi.is_negative()
    }

    /// `|x| < 2^bits` for every `x` in the interval.
    pub(crate) fn mag_bits(&self) -> i64 {
        self.lo.mag_bits().max(self.hi.mag_bits())
    }

    pub(crate) fn clamp_unit(&self) -> Iv {
        Iv {
            lo: self.lo.clone().max(Dyadic::int(-1)),
            hi: self.hi.clone().min(Dyadic::int(1)),
        }
    }
}
