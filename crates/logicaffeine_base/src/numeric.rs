//! The numeric tower's foundation: a hand-rolled arbitrary-precision integer.
//!
//! Logos carries the *type* of a number across every boundary (interpreter, VM,
//! wire), so an integer never silently becomes an IEEE-754 double the way a JSON
//! number does — there is no 2^53 cliff here. [`BigInt`] is the exact, unbounded
//! integer all of that rests on; [`Rational`]/[`Decimal`]/[`Complex`] build on it.
//!
//! Representation is sign + little-endian base-2^64 magnitude (`Vec<u64>` limbs,
//! no trailing zeros; zero is the empty magnitude). Arithmetic here is *correct
//! first* — schoolbook add/sub/mul and bit-at-a-time long division — which is the
//! exact-determinism floor; Karatsuba multiplication and Knuth-D division are the
//! FAST follow-up that must reproduce these results bit-for-bit.

use std::cmp::Ordering;
use std::fmt;

/// The sign of a [`BigInt`]. Zero has its own sign so the magnitude invariant
/// (no trailing zero limbs; the zero magnitude is empty) stays canonical.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Sign {
    Neg,
    Zero,
    Pos,
}

/// An exact, arbitrary-precision integer.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct BigInt {
    sign: Sign,
    /// Little-endian base-2^64 limbs, normalized (no trailing zeros). Empty ⇔ zero.
    mag: Vec<u64>,
}

// ---- magnitude primitives (operate on normalized little-endian `&[u64]`) ----

/// Drop trailing zero limbs so a magnitude has a unique representation.
fn normalize(mut mag: Vec<u64>) -> Vec<u64> {
    while mag.last() == Some(&0) {
        mag.pop();
    }
    mag
}

/// Compare two normalized magnitudes.
fn mag_cmp(a: &[u64], b: &[u64]) -> Ordering {
    match a.len().cmp(&b.len()) {
        Ordering::Equal => {
            for i in (0..a.len()).rev() {
                match a[i].cmp(&b[i]) {
                    Ordering::Equal => {}
                    other => return other,
                }
            }
            Ordering::Equal
        }
        other => other,
    }
}

/// `a + b` on magnitudes (full carry).
fn mag_add(a: &[u64], b: &[u64]) -> Vec<u64> {
    let mut out = Vec::with_capacity(a.len().max(b.len()) + 1);
    let mut carry = 0u128;
    for i in 0..a.len().max(b.len()) {
        let av = *a.get(i).unwrap_or(&0) as u128;
        let bv = *b.get(i).unwrap_or(&0) as u128;
        let sum = av + bv + carry;
        out.push(sum as u64);
        carry = sum >> 64;
    }
    if carry != 0 {
        out.push(carry as u64);
    }
    normalize(out)
}

/// `a - b` on magnitudes; requires `a >= b` (full borrow).
fn mag_sub(a: &[u64], b: &[u64]) -> Vec<u64> {
    debug_assert!(mag_cmp(a, b) != Ordering::Less, "mag_sub underflow");
    let mut out = Vec::with_capacity(a.len());
    let mut borrow = 0i128;
    for i in 0..a.len() {
        let av = a[i] as i128;
        let bv = *b.get(i).unwrap_or(&0) as i128;
        let mut diff = av - bv - borrow;
        if diff < 0 {
            diff += 1i128 << 64;
            borrow = 1;
        } else {
            borrow = 0;
        }
        out.push(diff as u64);
    }
    debug_assert_eq!(borrow, 0, "mag_sub left a borrow");
    normalize(out)
}

/// Schoolbook `a * b` on magnitudes (Karatsuba is the FAST follow-up).
fn mag_mul(a: &[u64], b: &[u64]) -> Vec<u64> {
    if a.is_empty() || b.is_empty() {
        return Vec::new();
    }
    let mut out = vec![0u64; a.len() + b.len()];
    for (i, &av) in a.iter().enumerate() {
        let mut carry = 0u128;
        for (j, &bv) in b.iter().enumerate() {
            let cur = out[i + j] as u128 + (av as u128) * (bv as u128) + carry;
            out[i + j] = cur as u64;
            carry = cur >> 64;
        }
        out[i + b.len()] += carry as u64;
    }
    normalize(out)
}

/// `r << 1 | bit` on a magnitude (shift the whole number up by one bit).
fn mag_shl1(a: &[u64], bit: u64) -> Vec<u64> {
    let mut out = Vec::with_capacity(a.len() + 1);
    let mut carry = bit & 1;
    for &limb in a {
        out.push((limb << 1) | carry);
        carry = limb >> 63;
    }
    if carry != 0 {
        out.push(carry);
    }
    normalize(out)
}

/// Long division on magnitudes: returns `(quotient, remainder)` with
/// `a = q*b + r` and `0 <= r < b`. Bit-at-a-time — correct and simple (the
/// exact-determinism oracle); Knuth Algorithm D is the FAST replacement.
fn mag_divrem(a: &[u64], b: &[u64]) -> (Vec<u64>, Vec<u64>) {
    debug_assert!(!b.is_empty(), "division by zero magnitude");
    if mag_cmp(a, b) == Ordering::Less {
        return (Vec::new(), a.to_vec());
    }
    let nbits = a.len() * 64;
    let mut q = vec![0u64; a.len()];
    let mut r: Vec<u64> = Vec::new();
    for i in (0..nbits).rev() {
        let bit = (a[i / 64] >> (i % 64)) & 1;
        r = mag_shl1(&r, bit);
        if mag_cmp(&r, b) != Ordering::Less {
            r = mag_sub(&r, b);
            q[i / 64] |= 1u64 << (i % 64);
        }
    }
    (normalize(q), normalize(r))
}

impl BigInt {
    /// The additive identity.
    pub fn zero() -> Self {
        BigInt { sign: Sign::Zero, mag: Vec::new() }
    }

    /// Build from a sign flag and a (not-necessarily-normalized) magnitude,
    /// re-establishing the canonical form (trim zeros; empty magnitude ⇒ Zero).
    fn from_sign_mag(neg: bool, mag: Vec<u64>) -> Self {
        let mag = normalize(mag);
        let sign = if mag.is_empty() {
            Sign::Zero
        } else if neg {
            Sign::Neg
        } else {
            Sign::Pos
        };
        BigInt { sign, mag }
    }

    /// Exact widening from a machine integer.
    pub fn from_i64(x: i64) -> Self {
        if x == 0 {
            return Self::zero();
        }
        let m = (x as i128).unsigned_abs() as u64;
        Self::from_sign_mag(x < 0, vec![m])
    }

    /// Exact widening from an unsigned machine integer.
    pub fn from_u64(x: u64) -> Self {
        if x == 0 {
            Self::zero()
        } else {
            Self::from_sign_mag(false, vec![x])
        }
    }

    /// Narrow back to an `i64` iff the value fits — the basis of the "downsize when
    /// it provably fits" path the runtime uses to stay on the fast i64 repr.
    pub fn to_i64(&self) -> Option<i64> {
        if self.mag.len() > 1 {
            return None;
        }
        let m = self.mag.first().copied().unwrap_or(0) as i128;
        let v = if self.sign == Sign::Neg { -m } else { m };
        if (i64::MIN as i128..=i64::MAX as i128).contains(&v) {
            Some(v as i64)
        } else {
            None
        }
    }

    pub fn is_zero(&self) -> bool {
        self.sign == Sign::Zero
    }

    /// Nearest `f64` (lossy by nature — float semantics) for mixed int/float math.
    pub fn to_f64(&self) -> f64 {
        const TWO64: f64 = 18_446_744_073_709_551_616.0; // 2^64
        let mut acc = 0.0f64;
        for &limb in self.mag.iter().rev() {
            acc = acc * TWO64 + limb as f64;
        }
        if self.sign == Sign::Neg {
            -acc
        } else {
            acc
        }
    }

    pub fn is_negative(&self) -> bool {
        self.sign == Sign::Neg
    }

    /// The absolute value.
    pub fn abs(&self) -> Self {
        BigInt { sign: if self.sign == Sign::Zero { Sign::Zero } else { Sign::Pos }, mag: self.mag.clone() }
    }

    /// Additive inverse.
    pub fn negated(&self) -> Self {
        let sign = match self.sign {
            Sign::Neg => Sign::Pos,
            Sign::Zero => Sign::Zero,
            Sign::Pos => Sign::Neg,
        };
        BigInt { sign, mag: self.mag.clone() }
    }

    /// `self + other`.
    pub fn add(&self, other: &Self) -> Self {
        match (self.sign, other.sign) {
            (Sign::Zero, _) => other.clone(),
            (_, Sign::Zero) => self.clone(),
            // Same sign: add magnitudes, keep the sign.
            (a, b) if a == b => Self::from_sign_mag(a == Sign::Neg, mag_add(&self.mag, &other.mag)),
            // Opposite signs: subtract the smaller magnitude from the larger; the
            // result takes the sign of the larger.
            _ => match mag_cmp(&self.mag, &other.mag) {
                Ordering::Equal => Self::zero(),
                Ordering::Greater => Self::from_sign_mag(self.sign == Sign::Neg, mag_sub(&self.mag, &other.mag)),
                Ordering::Less => Self::from_sign_mag(other.sign == Sign::Neg, mag_sub(&other.mag, &self.mag)),
            },
        }
    }

    /// `self - other`.
    pub fn sub(&self, other: &Self) -> Self {
        self.add(&other.negated())
    }

    /// `self * other`.
    pub fn mul(&self, other: &Self) -> Self {
        if self.is_zero() || other.is_zero() {
            return Self::zero();
        }
        let neg = (self.sign == Sign::Neg) ^ (other.sign == Sign::Neg);
        Self::from_sign_mag(neg, mag_mul(&self.mag, &other.mag))
    }

    /// Truncated division toward zero: returns `(quotient, remainder)` with
    /// `self = q*other + r` and the remainder carrying the dividend's sign — exactly
    /// matching Rust/`i64` `/` and `%`, so the wide type is a drop-in for the narrow.
    /// `None` when `other` is zero.
    pub fn div_rem(&self, other: &Self) -> Option<(Self, Self)> {
        if other.is_zero() {
            return None;
        }
        if self.is_zero() {
            return Some((Self::zero(), Self::zero()));
        }
        let (qm, rm) = mag_divrem(&self.mag, &other.mag);
        let q_neg = (self.sign == Sign::Neg) ^ (other.sign == Sign::Neg);
        let q = Self::from_sign_mag(q_neg, qm);
        // The remainder takes the dividend's sign (truncated division).
        let r = Self::from_sign_mag(self.sign == Sign::Neg, rm);
        Some((q, r))
    }

    /// Parse a base-10 integer (optional leading `+`/`-`). `None` on any non-digit.
    pub fn parse_decimal(s: &str) -> Option<Self> {
        let bytes = s.as_bytes();
        let (neg, digits) = match bytes.first() {
            Some(b'-') => (true, &bytes[1..]),
            Some(b'+') => (false, &bytes[1..]),
            _ => (false, bytes),
        };
        if digits.is_empty() {
            return None;
        }
        let ten = BigInt::from_u64(10);
        let mut acc = BigInt::zero();
        for &d in digits {
            if !d.is_ascii_digit() {
                return None;
            }
            acc = acc.mul(&ten).add(&BigInt::from_u64((d - b'0') as u64));
        }
        Some(if neg { acc.negated() } else { acc })
    }

    /// `self` raised to a non-negative integer power, by exponentiation-by-squaring
    /// (`0^0 == 1`). Exact and unbounded — `2.pow(63)` is the value `i64` cannot hold.
    pub fn pow(&self, mut exp: u32) -> BigInt {
        let mut result = BigInt::from_u64(1);
        let mut base = self.clone();
        while exp > 0 {
            if exp & 1 == 1 {
                result = result.mul(&base);
            }
            exp >>= 1;
            if exp > 0 {
                base = base.mul(&base);
            }
        }
        result
    }

    /// Sign + little-endian magnitude bytes — a compact, exact serialization (the
    /// inverse of [`BigInt::from_le_bytes`]). The wire ships these instead of a
    /// decimal string, so there is no base conversion and no precision question.
    pub fn to_le_bytes(&self) -> (bool, Vec<u8>) {
        let mut bytes = Vec::with_capacity(self.mag.len() * 8);
        for &limb in &self.mag {
            bytes.extend_from_slice(&limb.to_le_bytes());
        }
        (self.sign == Sign::Neg, bytes)
    }

    /// Reconstruct from a sign flag and little-endian magnitude bytes (length need
    /// not be a multiple of 8; trailing zero limbs are normalized away).
    pub fn from_le_bytes(negative: bool, bytes: &[u8]) -> Self {
        let mut mag = Vec::with_capacity(bytes.len().div_ceil(8));
        for chunk in bytes.chunks(8) {
            let mut limb = [0u8; 8];
            limb[..chunk.len()].copy_from_slice(chunk);
            mag.push(u64::from_le_bytes(limb));
        }
        Self::from_sign_mag(negative, mag)
    }
}

impl Ord for BigInt {
    fn cmp(&self, other: &Self) -> Ordering {
        // Order by sign first, then by magnitude (reversed when both negative).
        let rank = |s: Sign| match s {
            Sign::Neg => -1i8,
            Sign::Zero => 0,
            Sign::Pos => 1,
        };
        match rank(self.sign).cmp(&rank(other.sign)) {
            Ordering::Equal => match self.sign {
                Sign::Zero => Ordering::Equal,
                Sign::Pos => mag_cmp(&self.mag, &other.mag),
                Sign::Neg => mag_cmp(&other.mag, &self.mag),
            },
            other => other,
        }
    }
}

impl PartialOrd for BigInt {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for BigInt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_zero() {
            return write!(f, "0");
        }
        // Emit base-10 by repeatedly dividing by 10^19 (the largest power of ten that
        // fits in a u64), collecting 19-digit chunks least-significant first.
        const TEN19: u64 = 10_000_000_000_000_000_000;
        let ten19 = BigInt::from_u64(TEN19);
        let mut chunks: Vec<u64> = Vec::new();
        let mut cur = self.abs();
        while !cur.is_zero() {
            let (q, r) = cur.div_rem(&ten19).expect("10^19 is nonzero");
            // r < 10^19 < 2^64, so it is one limb at most — and may exceed i64::MAX,
            // hence read the limb directly rather than via `to_i64`.
            chunks.push(r.mag.first().copied().unwrap_or(0));
            cur = q;
        }
        if self.is_negative() {
            write!(f, "-")?;
        }
        // Most-significant chunk has no leading zeros; the rest are zero-padded to 19.
        write!(f, "{}", chunks.last().unwrap())?;
        for &chunk in chunks.iter().rev().skip(1) {
            write!(f, "{chunk:019}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for BigInt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BigInt({self})")
    }
}

impl From<i64> for BigInt {
    fn from(x: i64) -> Self {
        BigInt::from_i64(x)
    }
}

// =====================================================================
// Rational — exact fractions on top of BigInt
// =====================================================================

/// `gcd(|a|, |b|)` by the Euclidean algorithm (`gcd(a, 0) == |a|`).
fn bigint_gcd(a: &BigInt, b: &BigInt) -> BigInt {
    let mut a = a.abs();
    let mut b = b.abs();
    while !b.is_zero() {
        let (_q, r) = a.div_rem(&b).expect("b is nonzero inside the loop");
        a = b;
        b = r;
    }
    a
}

/// An exact rational number: a fraction kept in lowest terms with a strictly
/// positive denominator. Built on [`BigInt`], so it never rounds the way a JSON
/// / `f64` "number" does — `1/3` stays exactly `1/3`, not `0.3333…`, and a
/// numerator past 2^53 survives instead of collapsing onto a double.
///
/// Representation is correct-first: BigInt numerator and denominator, reduced on
/// every construction. The i64-fast-path (storing small num/den inline to skip
/// the BigInt allocation) is the documented performance follow-up — exactly as
/// Karatsuba is for [`BigInt`] — and must reproduce these values bit-for-bit.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Rational {
    /// Carries the sign of the whole value.
    num: BigInt,
    /// INVARIANT: `den > 0`, `gcd(|num|, den) == 1`, and `den == 1` whenever
    /// `num == 0` — so equal values share one representation (Eq/Hash/Ord are
    /// structural).
    den: BigInt,
}

impl Rational {
    /// The canonicalizing constructor: reduce `num/den` to lowest terms with a
    /// positive denominator. Returns `None` for a zero denominator (the one
    /// undefined fraction).
    pub fn new(num: BigInt, den: BigInt) -> Option<Rational> {
        if den.is_zero() {
            return None;
        }
        let (mut num, mut den) = (num, den);
        if den.is_negative() {
            num = num.negated();
            den = den.negated();
        }
        if num.is_zero() {
            return Some(Rational { num: BigInt::zero(), den: BigInt::from_i64(1) });
        }
        let g = bigint_gcd(&num, &den);
        let (num, _) = num.div_rem(&g).expect("gcd divides the numerator exactly");
        let (den, _) = den.div_rem(&g).expect("gcd divides the denominator exactly");
        Some(Rational { num, den })
    }

    /// The integer `n` as the fraction `n/1`.
    pub fn from_bigint(n: BigInt) -> Rational {
        Rational { num: n, den: BigInt::from_i64(1) }
    }

    pub fn from_i64(x: i64) -> Rational {
        Rational::from_bigint(BigInt::from_i64(x))
    }

    /// `n/d` from machine integers (convenience; `None` if `d == 0`).
    pub fn from_ratio_i64(n: i64, d: i64) -> Option<Rational> {
        Rational::new(BigInt::from_i64(n), BigInt::from_i64(d))
    }

    pub fn zero() -> Rational {
        Rational { num: BigInt::zero(), den: BigInt::from_i64(1) }
    }

    pub fn one() -> Rational {
        Rational::from_i64(1)
    }

    pub fn numerator(&self) -> &BigInt {
        &self.num
    }

    pub fn denominator(&self) -> &BigInt {
        &self.den
    }

    /// True when the value is a whole number (`den == 1`).
    pub fn is_integer(&self) -> bool {
        self.den == BigInt::from_i64(1)
    }

    pub fn is_zero(&self) -> bool {
        self.num.is_zero()
    }

    pub fn is_negative(&self) -> bool {
        self.num.is_negative()
    }

    /// The integer value when this is whole, else `None` (the provable narrow
    /// back to [`BigInt`]).
    pub fn to_bigint(&self) -> Option<BigInt> {
        if self.is_integer() {
            Some(self.num.clone())
        } else {
            None
        }
    }

    /// The `i64` value when this is a whole number that fits, else `None`.
    pub fn to_i64(&self) -> Option<i64> {
        if self.is_integer() {
            self.num.to_i64()
        } else {
            None
        }
    }

    /// Nearest `f64` (for display / interop only — lossy for large terms, exact
    /// for small ones). The *value* stays exact; this is a view.
    pub fn to_f64(&self) -> f64 {
        self.num.to_f64() / self.den.to_f64()
    }

    /// The greatest integer `≤ self` (round toward −∞). `floor(7/2) == 3`,
    /// `floor(-7/2) == -4`. The companion of explicit floor division.
    pub fn floor(&self) -> BigInt {
        let (q, r) = self.num.div_rem(&self.den).expect("denominator is nonzero");
        if self.num.is_negative() && !r.is_zero() {
            q.sub(&BigInt::from_i64(1))
        } else {
            q
        }
    }

    /// The least integer `≥ self` (round toward +∞). `ceil(7/2) == 4`,
    /// `ceil(-7/2) == -3`.
    pub fn ceil(&self) -> BigInt {
        let (q, r) = self.num.div_rem(&self.den).expect("denominator is nonzero");
        if !self.num.is_negative() && !r.is_zero() {
            q.add(&BigInt::from_i64(1))
        } else {
            q
        }
    }

    /// The nearest integer, ties rounded AWAY from zero (matching `f64::round`):
    /// `round(x) = sign(x) · ⌊|x| + 1/2⌋ = sign(x) · ((2|num| + den) ÷ 2den)`.
    pub fn round(&self) -> BigInt {
        let two = BigInt::from_i64(2);
        let numerator = self.num.abs().mul(&two).add(&self.den);
        let denominator = self.den.mul(&two);
        let (mag, _) = numerator.div_rem(&denominator).expect("denominator is nonzero");
        if self.num.is_negative() {
            mag.negated()
        } else {
            mag
        }
    }

    pub fn negated(&self) -> Rational {
        Rational { num: self.num.negated(), den: self.den.clone() }
    }

    pub fn abs(&self) -> Rational {
        Rational { num: self.num.abs(), den: self.den.clone() }
    }

    /// `1/self` — `None` when `self == 0`.
    pub fn recip(&self) -> Option<Rational> {
        Rational::new(self.den.clone(), self.num.clone())
    }

    pub fn add(&self, other: &Rational) -> Rational {
        // a/b + c/d = (a·d + c·b)/(b·d); b,d > 0 ⇒ b·d > 0 ⇒ `new` succeeds.
        let num = self.num.mul(&other.den).add(&other.num.mul(&self.den));
        let den = self.den.mul(&other.den);
        Rational::new(num, den).expect("product of positive denominators is nonzero")
    }

    pub fn sub(&self, other: &Rational) -> Rational {
        let num = self.num.mul(&other.den).sub(&other.num.mul(&self.den));
        let den = self.den.mul(&other.den);
        Rational::new(num, den).expect("product of positive denominators is nonzero")
    }

    pub fn mul(&self, other: &Rational) -> Rational {
        let num = self.num.mul(&other.num);
        let den = self.den.mul(&other.den);
        Rational::new(num, den).expect("product of positive denominators is nonzero")
    }

    /// `self / other` — `None` when `other == 0`.
    pub fn div(&self, other: &Rational) -> Option<Rational> {
        // (a/b)/(c/d) = (a·d)/(b·c); `new` rejects a zero denominator (c == 0).
        let num = self.num.mul(&other.den);
        let den = self.den.mul(&other.num);
        Rational::new(num, den)
    }

    /// `self^exp`, exact for every integer exponent. Negative exponents take the
    /// reciprocal first; `None` only for `0` raised to a negative power.
    pub fn pow(&self, exp: i32) -> Option<Rational> {
        if exp >= 0 {
            let k = exp as u32;
            Some(
                Rational::new(self.num.pow(k), self.den.pow(k))
                    .expect("denominator^k stays positive"),
            )
        } else {
            if self.num.is_zero() {
                return None;
            }
            let k = exp.unsigned_abs();
            // (a/b)^-k = b^k / a^k; `new` re-fixes the sign and reduces.
            Rational::new(self.den.pow(k), self.num.pow(k))
        }
    }

    /// Parse `"3/4"`, `"-3/4"`, or a bare integer `"5"`. Whitespace around the
    /// parts is tolerated; `None` on malformed input or a zero denominator.
    pub fn parse(s: &str) -> Option<Rational> {
        let s = s.trim();
        if let Some((n, d)) = s.split_once('/') {
            let num = BigInt::parse_decimal(n.trim())?;
            let den = BigInt::parse_decimal(d.trim())?;
            Rational::new(num, den)
        } else {
            Some(Rational::from_bigint(BigInt::parse_decimal(s)?))
        }
    }
}

impl Ord for Rational {
    fn cmp(&self, other: &Self) -> Ordering {
        // a/b vs c/d with b, d > 0: compare a·d vs c·b (no rounding).
        self.num.mul(&other.den).cmp(&other.num.mul(&self.den))
    }
}

impl PartialOrd for Rational {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Rational {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_integer() {
            write!(f, "{}", self.num)
        } else {
            write!(f, "{}/{}", self.num, self.den)
        }
    }
}

impl fmt::Debug for Rational {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Rational({}/{})", self.num, self.den)
    }
}

impl From<i64> for Rational {
    fn from(x: i64) -> Self {
        Rational::from_i64(x)
    }
}

impl From<BigInt> for Rational {
    fn from(x: BigInt) -> Self {
        Rational::from_bigint(x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(x: i64) -> BigInt {
        BigInt::from_i64(x)
    }

    #[test]
    fn from_to_i64_round_trips_the_extremes() {
        for x in [0i64, 1, -1, 42, -42, i64::MAX, i64::MIN, i64::MAX - 1, i64::MIN + 1] {
            assert_eq!(BigInt::from_i64(x).to_i64(), Some(x), "round trip {x}");
        }
    }

    #[test]
    fn to_i64_is_none_just_past_the_boundary() {
        // i64::MAX + 1 and i64::MIN - 1 must NOT fit i64 (this is the whole point —
        // the value survives instead of wrapping, unlike a JSON double).
        let over = b(i64::MAX).add(&b(1));
        let under = b(i64::MIN).sub(&b(1));
        assert_eq!(over.to_i64(), None);
        assert_eq!(under.to_i64(), None);
        // …but the values are still exact and printable.
        assert_eq!(over.to_string(), "9223372036854775808");
        assert_eq!(under.to_string(), "-9223372036854775809");
    }

    #[test]
    fn add_sub_mul_match_i128_on_a_dense_grid() {
        // Differential oracle: for every pair that fits i128, our wide arithmetic must
        // equal the machine's. Includes carry/borrow/sign corners.
        let xs: [i64; 11] =
            [0, 1, -1, 2, -2, 1000, -1000, i32::MAX as i64, i32::MIN as i64, i64::MAX, i64::MIN];
        for &x in &xs {
            for &y in &xs {
                let (bx, by) = (b(x), b(y));
                assert_eq!(bx.add(&by).to_string(), (x as i128 + y as i128).to_string(), "{x}+{y}");
                assert_eq!(bx.sub(&by).to_string(), (x as i128 - y as i128).to_string(), "{x}-{y}");
                assert_eq!(bx.mul(&by).to_string(), (x as i128 * y as i128).to_string(), "{x}*{y}");
            }
        }
    }

    #[test]
    fn div_rem_matches_i64_truncation_including_signs() {
        let xs = [0i64, 1, -1, 7, -7, 100, -100, 9_999_999, -9_999_999, i64::MAX, i64::MIN];
        let ys = [1i64, -1, 2, -2, 3, -3, 7, -7, 1000, -1000];
        for &x in &xs {
            for &y in &ys {
                let (q, r) = b(x).div_rem(&b(y)).expect("nonzero divisor");
                // i64::MIN / -1 overflows i64; compare in i128 there.
                let (eq, er) = ((x as i128) / (y as i128), (x as i128) % (y as i128));
                assert_eq!(q.to_string(), eq.to_string(), "{x}/{y} quotient");
                assert_eq!(r.to_string(), er.to_string(), "{x}%{y} remainder");
                // The defining identity must hold exactly.
                assert_eq!(b(x), q.mul(&b(y)).add(&r), "x = q*y + r for {x},{y}");
            }
        }
    }

    #[test]
    fn division_by_zero_is_none_not_a_panic() {
        assert!(b(5).div_rem(&BigInt::zero()).is_none());
        assert!(BigInt::zero().div_rem(&BigInt::zero()).is_none());
    }

    #[test]
    fn huge_factorial_is_exact() {
        // 50! has 65 digits — far beyond any fixed-width integer or f64.
        let mut acc = BigInt::from_u64(1);
        for k in 1..=50u64 {
            acc = acc.mul(&BigInt::from_u64(k));
        }
        assert_eq!(acc.to_string(), "30414093201713378043612608166064768844377641568960512000000000000");
        // Dividing back down the chain returns exactly 1 (exercises big/big division).
        for k in 1..=50u64 {
            let (q, r) = acc.div_rem(&BigInt::from_u64(k)).unwrap();
            assert!(r.is_zero(), "{k}! divides cleanly");
            acc = q;
        }
        assert_eq!(acc, BigInt::from_u64(1));
    }

    #[test]
    fn parse_and_display_round_trip_big_decimals() {
        for s in [
            "0",
            "-0",
            "7",
            "-7",
            "9223372036854775808",
            "-9223372036854775809",
            "123456789012345678901234567890",
            "-100000000000000000000000000000000000000000",
        ] {
            let parsed = BigInt::parse_decimal(s).expect("parse");
            // "-0" canonicalizes to "0".
            let expected = if s == "-0" { "0" } else { s };
            assert_eq!(parsed.to_string(), expected, "round trip {s}");
        }
        assert!(BigInt::parse_decimal("12x3").is_none());
        assert!(BigInt::parse_decimal("").is_none());
        assert!(BigInt::parse_decimal("-").is_none());
    }

    /// A tiny deterministic RNG (SplitMix64) so the fuzz is reproducible with no
    /// external dependency.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }
        /// A random BigInt of 0..=3 limbs and random sign — spans single- and
        /// multi-limb magnitudes (and zero).
        fn big(&mut self) -> BigInt {
            let limbs = (self.next() % 4) as usize;
            let mut bytes = Vec::new();
            for _ in 0..limbs {
                bytes.extend_from_slice(&self.next().to_le_bytes());
            }
            BigInt::from_le_bytes(self.next() & 1 == 1, &bytes)
        }
    }

    #[test]
    fn fuzz_algebraic_laws_hold_for_random_bigints() {
        let mut r = Rng(0x0BAD_F00D_DEAD_BEEF);
        for _ in 0..3000 {
            let (a, b, c) = (r.big(), r.big(), r.big());
            // Commutativity.
            assert_eq!(a.add(&b), b.add(&a), "add commutes");
            assert_eq!(a.mul(&b), b.mul(&a), "mul commutes");
            // Associativity.
            assert_eq!(a.add(&b).add(&c), a.add(&b.add(&c)), "add associates");
            assert_eq!(a.mul(&b).mul(&c), a.mul(&b.mul(&c)), "mul associates");
            // Distributivity.
            assert_eq!(a.mul(&b.add(&c)), a.mul(&b).add(&a.mul(&c)), "mul distributes over add");
            // Identities and inverses.
            assert_eq!(a.add(&BigInt::zero()), a, "0 is the additive identity");
            assert_eq!(a.mul(&BigInt::from_u64(1)), a, "1 is the multiplicative identity");
            assert!(a.mul(&BigInt::zero()).is_zero(), "x*0 = 0");
            assert_eq!(a.sub(&a), BigInt::zero(), "x - x = 0");
            assert_eq!(a.add(&a.negated()), BigInt::zero(), "x + (-x) = 0");
            assert_eq!(a.negated().negated(), a, "double negation");
            // abs is non-negative; sign of a product.
            assert!(!a.abs().is_negative(), "|x| >= 0");
            assert_eq!(a.negated().mul(&b), a.mul(&b).negated(), "(-a)*b = -(a*b)");
            // Division identity and remainder bound.
            if !b.is_zero() {
                let (q, rem) = a.div_rem(&b).unwrap();
                assert_eq!(q.mul(&b).add(&rem), a, "a = q*b + r exactly");
                assert!(rem.is_zero() || rem.abs() < b.abs(), "|r| < |b|");
            }
            // Serialization and decimal round-trips.
            let (neg, bytes) = a.to_le_bytes();
            assert_eq!(BigInt::from_le_bytes(neg, &bytes), a, "byte round-trip");
            assert_eq!(BigInt::parse_decimal(&a.to_string()).unwrap(), a, "decimal round-trip");
            // Ordering is a total, antisymmetric, transitive relation.
            assert_eq!(a < b, b > a, "antisymmetry");
            if a < b && b < c {
                assert!(a < c, "transitivity");
            }
        }
    }

    #[test]
    fn fuzz_differential_against_i128() {
        // For random i64 operands, our (promoting) arithmetic must equal i128 exactly.
        let mut r = Rng(0xC0FF_EE00_1234_5678);
        for _ in 0..5000 {
            let x = r.next() as i64;
            let y = r.next() as i64;
            assert_eq!(b(x).add(&b(y)).to_string(), (x as i128 + y as i128).to_string(), "{x}+{y}");
            assert_eq!(b(x).sub(&b(y)).to_string(), (x as i128 - y as i128).to_string(), "{x}-{y}");
            assert_eq!(b(x).mul(&b(y)).to_string(), (x as i128 * y as i128).to_string(), "{x}*{y}");
            if y != 0 {
                let (q, rem) = b(x).div_rem(&b(y)).unwrap();
                assert_eq!(q.to_string(), (x as i128 / y as i128).to_string(), "{x}/{y}");
                assert_eq!(rem.to_string(), (x as i128 % y as i128).to_string(), "{x}%{y}");
            }
        }
    }

    #[test]
    fn limb_boundary_edge_cases_are_exact() {
        let two64 = BigInt::parse_decimal("18446744073709551616").unwrap(); // 2^64
        // 2^64 - 1 = u64::MAX (a borrow that empties the high limb).
        assert_eq!(two64.sub(&BigInt::from_u64(1)).to_string(), "18446744073709551615");
        // 2^64 * 2^64 = 2^128 (exact two-limb product).
        let two128 = two64.mul(&two64);
        assert_eq!(two128.to_string(), "340282366920938463463374607431768211456");
        // 2^128 - 1 (a borrow chain across every limb).
        assert_eq!(two128.sub(&BigInt::from_u64(1)).to_string(), "340282366920938463463374607431768211455");
        // u64::MAX + 1 = 2^64 (a carry that grows a limb).
        assert_eq!(BigInt::from_u64(u64::MAX).add(&BigInt::from_u64(1)), two64);
        // Division with a multi-limb divisor: 2^128 / 2^64 = 2^64, remainder 0.
        let (q, rem) = two128.div_rem(&two64).unwrap();
        assert_eq!(q, two64);
        assert!(rem.is_zero());
    }

    #[test]
    fn pow_is_exact_and_unbounded() {
        assert_eq!(b(2).pow(63).to_string(), "9223372036854775808"); // the value i64 can't hold
        assert_eq!(b(2).pow(100).to_string(), "1267650600228229401496703205376");
        assert_eq!(b(10).pow(0).to_string(), "1");
        assert_eq!(b(0).pow(0).to_string(), "1");
        assert_eq!(b(-3).pow(3).to_string(), "-27");
        assert_eq!(b(-2).pow(10).to_string(), "1024");
        // 3^50 cross-checked against repeated multiplication.
        let mut acc = BigInt::from_u64(1);
        for _ in 0..50 {
            acc = acc.mul(&b(3));
        }
        assert_eq!(b(3).pow(50), acc);
    }

    #[test]
    fn ordering_is_total_and_sign_aware() {
        let mut v = vec![b(3), b(-5), BigInt::zero(), b(i64::MAX), b(i64::MIN), b(-5).mul(&b(i64::MAX))];
        v.sort();
        let as_str: Vec<String> = v.iter().map(|x| x.to_string()).collect();
        assert_eq!(
            as_str,
            vec![
                "-46116860184273879035", // -5 * i64::MAX
                "-9223372036854775808",  // i64::MIN
                "-5",
                "0",
                "3",
                "9223372036854775807", // i64::MAX
            ]
        );
    }

    // -----------------------------------------------------------------
    // Rational
    // -----------------------------------------------------------------

    fn r(n: i64, d: i64) -> Rational {
        Rational::from_ratio_i64(n, d).expect("nonzero denominator in test")
    }

    #[test]
    fn rational_reduces_to_lowest_terms_on_construction() {
        assert_eq!(r(6, 8).to_string(), "3/4");
        assert_eq!(r(10, 5).to_string(), "2");
        assert_eq!(r(0, 7).to_string(), "0");
        assert_eq!(r(100, 1000).to_string(), "1/10");
        // The reduced form is canonical, so equal values are structurally equal.
        assert_eq!(r(6, 8), r(3, 4));
        assert_eq!(r(0, 7), r(0, 1));
    }

    #[test]
    fn rational_normalizes_sign_onto_a_positive_denominator() {
        assert_eq!(r(1, -2).to_string(), "-1/2");
        assert_eq!(r(-1, -2).to_string(), "1/2");
        assert_eq!(r(-3, 4), r(3, -4));
        assert!(r(1, -2).is_negative());
        assert!(!r(-1, -2).is_negative());
        // The denominator accessor is always positive.
        assert!(!r(1, -2).denominator().is_negative());
    }

    #[test]
    fn rational_zero_denominator_is_none() {
        assert!(Rational::from_ratio_i64(5, 0).is_none());
        assert!(Rational::new(BigInt::from_i64(1), BigInt::zero()).is_none());
        assert!(r(3, 4).div(&Rational::zero()).is_none());
        assert!(Rational::zero().recip().is_none());
        assert!(Rational::parse("7/0").is_none());
    }

    #[test]
    fn rational_arithmetic_is_exact() {
        assert_eq!(r(1, 3).add(&r(1, 6)).to_string(), "1/2");
        assert_eq!(r(1, 2).sub(&r(1, 3)).to_string(), "1/6");
        assert_eq!(r(2, 3).mul(&r(3, 4)).to_string(), "1/2");
        assert_eq!(r(1, 2).div(&r(3, 4)).unwrap().to_string(), "2/3");
        // a + (-a) == 0, a * (1/a) == 1.
        assert!(r(5, 7).add(&r(5, 7).negated()).is_zero());
        assert_eq!(r(5, 7).mul(&r(5, 7).recip().unwrap()), Rational::one());
    }

    #[test]
    fn one_third_stays_exact_where_a_json_double_would_round() {
        // The whole point: 1/3 is EXACT, not 0.3333…; three of them are exactly 1.
        let third = r(1, 3);
        let sum = third.add(&third).add(&third);
        assert_eq!(sum, Rational::one());
        assert_eq!(third.to_string(), "1/3");
        // The classic f64 trap 0.1 + 0.2 != 0.3 — Rationals don't have it.
        assert_ne!(0.1_f64 + 0.2, 0.3);
        assert_eq!(r(1, 10).add(&r(2, 10)), r(3, 10));
    }

    #[test]
    fn rational_ordering_cross_multiplies_without_rounding() {
        assert!(r(1, 3) < r(1, 2));
        assert!(r(-1, 2) < r(1, 3));
        assert!(r(2, 4) == r(1, 2));
        let mut v = vec![r(1, 2), r(1, 3), r(-1, 4), r(2, 3), Rational::zero(), r(1, 2)];
        v.sort();
        let s: Vec<String> = v.iter().map(|x| x.to_string()).collect();
        assert_eq!(s, ["-1/4", "0", "1/3", "1/2", "1/2", "2/3"]);
    }

    #[test]
    fn rational_terms_overflow_i64_into_bigint() {
        // (1/i64::MAX) + (1/i64::MAX) = 2/i64::MAX — denominator past i64 stays exact.
        let big = Rational::new(BigInt::from_i64(1), BigInt::from_i64(i64::MAX)).unwrap();
        let twice = big.add(&big);
        assert_eq!(twice.numerator().to_string(), "2");
        assert_eq!(twice.denominator().to_string(), i64::MAX.to_string());
        // A product whose numerator escapes i64 is still exact.
        let p = r(i64::MAX, 1).mul(&r(3, 1));
        assert_eq!(p.numerator().to_i64(), None);
        assert_eq!(p.numerator().to_string(), (i64::MAX as i128 * 3).to_string());
    }

    #[test]
    fn rational_pow_handles_negative_and_zero_exponents() {
        assert_eq!(r(2, 3).pow(3).unwrap().to_string(), "8/27");
        assert_eq!(r(2, 3).pow(0).unwrap(), Rational::one());
        assert_eq!(r(2, 3).pow(-2).unwrap().to_string(), "9/4");
        assert_eq!(r(-2, 3).pow(-3).unwrap().to_string(), "-27/8");
        assert!(Rational::zero().pow(-1).is_none());
        assert_eq!(Rational::zero().pow(3).unwrap(), Rational::zero());
    }

    #[test]
    fn rational_parse_round_trips_and_rejects_garbage() {
        assert_eq!(Rational::parse("3/4").unwrap().to_string(), "3/4");
        assert_eq!(Rational::parse("-3/4").unwrap().to_string(), "-3/4");
        assert_eq!(Rational::parse("6/8").unwrap().to_string(), "3/4");
        assert_eq!(Rational::parse("5").unwrap().to_string(), "5");
        assert_eq!(Rational::parse("  7 / 14 ").unwrap().to_string(), "1/2");
        assert!(Rational::parse("abc").is_none());
        assert!(Rational::parse("1/2/3").is_none());
    }

    #[test]
    fn rational_integer_predicate_and_narrowing() {
        assert!(r(10, 2).is_integer());
        assert_eq!(r(10, 2).to_i64(), Some(5));
        assert_eq!(r(10, 2).to_bigint().unwrap().to_string(), "5");
        assert!(!r(3, 4).is_integer());
        assert_eq!(r(3, 4).to_i64(), None);
        assert!(r(3, 4).to_bigint().is_none());
    }

    #[test]
    fn rational_floor_and_ceil_round_toward_neg_and_pos_infinity() {
        assert_eq!(r(7, 2).floor().to_string(), "3");
        assert_eq!(r(7, 2).ceil().to_string(), "4");
        assert_eq!(r(-7, 2).floor().to_string(), "-4");
        assert_eq!(r(-7, 2).ceil().to_string(), "-3");
        // Whole values floor/ceil to themselves.
        assert_eq!(r(6, 2).floor().to_string(), "3");
        assert_eq!(r(6, 2).ceil().to_string(), "3");
        // round ties away from zero, matching f64::round.
        assert_eq!(r(7, 2).round().to_string(), "4");
        assert_eq!(r(-7, 2).round().to_string(), "-4");
        assert_eq!(r(1, 3).round().to_string(), "0");
        assert_eq!(r(2, 3).round().to_string(), "1");
        // Differential vs f64 on a dense grid (small terms are exact in f64).
        for n in -9i64..=9 {
            for d in 1i64..=9 {
                let q = r(n, d);
                assert_eq!(q.floor().to_i64(), Some((n as f64 / d as f64).floor() as i64), "{n}/{d} floor");
                assert_eq!(q.ceil().to_i64(), Some((n as f64 / d as f64).ceil() as i64), "{n}/{d} ceil");
                assert_eq!(q.round().to_i64(), Some((n as f64 / d as f64).round() as i64), "{n}/{d} round");
            }
        }
    }

    #[test]
    fn rational_to_f64_matches_the_division_on_small_terms() {
        for n in -8i64..=8 {
            for d in 1i64..=8 {
                let approx = r(n, d).to_f64();
                assert!((approx - (n as f64 / d as f64)).abs() < 1e-12, "{n}/{d}");
            }
        }
    }

    #[test]
    fn rational_obeys_the_field_laws_over_random_fractions() {
        // Differential/property fuzz: build fractions from random i64 terms and
        // check the field axioms exactly (no rounding, unlike f64).
        let mut rng = Rng(0x5A7F_104E_2C19_8B63);
        for _ in 0..2000 {
            let pick = |rng: &mut Rng| -> i64 { (rng.next() % 4001) as i64 - 2000 };
            let (a, b, c, d, e, g) = (pick(&mut rng), pick(&mut rng), pick(&mut rng), pick(&mut rng), pick(&mut rng), pick(&mut rng));
            let (Some(x), Some(y), Some(z)) =
                (Rational::from_ratio_i64(a, b.max(1)), Rational::from_ratio_i64(c, d.max(1)), Rational::from_ratio_i64(e, g.max(1)))
            else { continue };
            // commutativity
            assert_eq!(x.add(&y), y.add(&x));
            assert_eq!(x.mul(&y), y.mul(&x));
            // associativity
            assert_eq!(x.add(&y).add(&z), x.add(&y.add(&z)));
            assert_eq!(x.mul(&y).mul(&z), x.mul(&y.mul(&z)));
            // distributivity
            assert_eq!(x.mul(&y.add(&z)), x.mul(&y).add(&x.mul(&z)));
            // additive inverse + subtraction agreement
            assert!(x.add(&x.negated()).is_zero());
            assert_eq!(x.sub(&y), x.add(&y.negated()));
            // multiplicative inverse (when nonzero)
            if !x.is_zero() {
                assert_eq!(x.mul(&x.recip().unwrap()), Rational::one());
                assert_eq!(y.div(&x).unwrap(), y.mul(&x.recip().unwrap()));
            }
        }
    }
}
