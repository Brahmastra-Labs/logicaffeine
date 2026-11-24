//! Division by a constant as a multiplication: the signed magic reciprocal.
//!
//! For a divisor `d ≥ 2` there is a 64-bit `magic` and a `shift` such that, for every `i64` dividend `x`,
//! truncating `x / d` is the high half of `magic · x`, plus `x` when `magic` is negative, shifted right
//! (arithmetically) by `shift`, plus one when `x` is negative. A machine that multiplies in a few cycles and divides in
//! dozens computes the same quotient a division would, for every dividend, with no division at all; the remainder is
//! `x − q·d`. The construction is Granlund and Montgomery's (PLDI 1994) in the form of Warren's *Hacker's Delight*
//! (§10-1), the one compilers use for a literal divisor.
//!
//! A divisor known only at run time has the same reciprocal, computed once: the JIT specializes a loop whose divisor
//! does not change on the value it observed, guarded by that value, and divides by multiplying.
//!
//! [`SignedReciprocal::new`] returns only a reciprocal [`SignedReciprocal::counterexample`] decides exact: the
//! construction is checked against its error bound in exact arithmetic for the divisor at hand, so no division the
//! JIT specializes rests on the construction being right in general.

use crate::numeric::BigInt;

/// The magic reciprocal of a divisor of at least two.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SignedReciprocal {
    magic: i64,
    shift: u32,
    add: bool,
    divisor: i64,
}

impl SignedReciprocal {
    /// The reciprocal of `divisor`, decided exact by [`Self::counterexample`], or `None` below two (one and the
    /// negatives are not specialized; zero has none).
    pub fn new(divisor: i64) -> Option<Self> {
        if divisor < 2 {
            return None;
        }
        let two63: u64 = 1 << 63;
        let ad = divisor as u64;
        // `anc`: the largest value of the form k·d − 1 below 2⁶³, the absolute value of the critical dividend.
        let anc = two63 - 1 - two63 % ad;
        let mut p: u32 = 63;
        let (mut q1, mut r1) = (two63 / anc, two63 - (two63 / anc) * anc);
        let (mut q2, mut r2) = (two63 / ad, two63 - (two63 / ad) * ad);
        loop {
            p += 1;
            q1 = q1.wrapping_mul(2);
            r1 = r1.wrapping_mul(2);
            if r1 >= anc {
                q1 = q1.wrapping_add(1);
                r1 = r1.wrapping_sub(anc);
            }
            q2 = q2.wrapping_mul(2);
            r2 = r2.wrapping_mul(2);
            if r2 >= ad {
                q2 = q2.wrapping_add(1);
                r2 = r2.wrapping_sub(ad);
            }
            let delta = ad - r2;
            if !(q1 < delta || (q1 == delta && r1 == 0)) {
                break;
            }
        }
        let magic = q2.wrapping_add(1) as i64;
        let reciprocal = Self {
            magic,
            shift: p - 64,
            add: magic < 0,
            divisor,
        };
        reciprocal.counterexample().is_none().then_some(reciprocal)
    }

    /// A dividend this reciprocal gets wrong, or `None` when it computes `x / divisor` for every `i64` `x`, decided
    /// exactly.
    ///
    /// The quotient is `⌊m·x / 2^L⌋`, plus one for a negative `x`, where `L = 64 + shift` and `m` is the magic, read
    /// as unsigned when the add step is taken. With `e = m·d − 2^L` and `x = q·d + r`, `m·x / 2^L` is
    /// `q + (r·2^L + x·e) / (d·2^L)`, so the quotient of `0 ≤ x < 2^63` is right exactly when
    /// `0 ≤ r·2^L + x·e < d·2^L`, and of `x = −y` (`1 ≤ y ≤ 2^63`) exactly when `0 < r·2^L + y·e ≤ d·2^L`. The lower
    /// bounds hold for every dividend once `e > 0`, which `y = d` needs. The upper bound is tightest at the critical
    /// dividend `c`, the largest in range of residue `d − 1`, where it reads `c·e < 2^L` (`≤` for the negatives): a
    /// dividend below `c` is bounded by the next one of residue `d − 1`, and one past it is `c + 1 + r` with
    /// `r ≤ d − 2`, whose bound follows from `c ≥ d − 1`. The positive critical dividend is Granlund and Montgomery's;
    /// the negative one has the same magnitude, or `2^63` when `d` divides `2^63 + 1`.
    pub fn counterexample(&self) -> Option<i64> {
        let two_l = BigInt::from_i64(1).shl_bits(64 + u64::from(self.shift));
        let mut m = BigInt::from_i64(self.magic);
        if self.add {
            m = m.add(&BigInt::from_i64(1).shl_bits(64));
        }
        let e = m.mul(&BigInt::from_i64(self.divisor)).sub(&two_l);
        if e <= BigInt::zero() {
            return Some(-self.divisor);
        }
        let d = self.divisor as u64;
        let critical = (1u64 << 63) - 1 - (1u64 << 63) % d;
        if BigInt::from_u64(critical).mul(&e) >= two_l {
            return Some(critical as i64);
        }
        let negative = critical + d;
        (negative == 1 << 63 && BigInt::from_u64(negative).mul(&e) > two_l).then_some(i64::MIN)
    }

    /// The multiplier.
    pub fn magic(&self) -> i64 {
        self.magic
    }

    /// The arithmetic right shift after the multiplication.
    pub fn shift(&self) -> u32 {
        self.shift
    }

    /// Whether the dividend is added to the high product before the shift: exactly when the magic is negative.
    pub fn add(&self) -> bool {
        self.add
    }

    /// The divisor.
    pub fn divisor(&self) -> i64 {
        self.divisor
    }

    /// `x / divisor`, truncating toward zero.
    pub fn quotient(&self, x: i64) -> i64 {
        let mut q = ((i128::from(self.magic) * i128::from(x)) >> 64) as i64;
        if self.add {
            q = q.wrapping_add(x);
        }
        q >>= self.shift;
        q.wrapping_add(((x as u64) >> 63) as i64)
    }

    /// `x % divisor`, with the sign of the dividend.
    pub fn remainder(&self, x: i64) -> i64 {
        x.wrapping_sub(self.quotient(x).wrapping_mul(self.divisor))
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_base/reciprocal/tests.rs"]
mod tests;
