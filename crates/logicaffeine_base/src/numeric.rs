//! The numeric tower's foundation: a hand-rolled arbitrary-precision integer.
//!
//! Logos carries the *type* of a number across every boundary (interpreter, VM,
//! wire), so an integer never silently becomes an IEEE-754 double the way a JSON
//! number does — there is no 2^53 cliff here. [`BigInt`] is the exact, unbounded
//! integer all of that rests on; [`Rational`]/[`Decimal`]/[`Complex`] build on it.
//!
//! Representation is sign + little-endian base-2^64 magnitude (limbs, no
//! trailing zeros; zero is the empty magnitude). A single-limb magnitude is
//! stored inline — no heap allocation for anything that fits 64 bits, and the
//! add/mul/div_rem fast paths compute those cases in machine registers.
//! Arithmetic here is *correct first* — schoolbook add/sub/mul and
//! bit-at-a-time long division — which is the exact-determinism floor;
//! Karatsuba multiplication and Knuth-D division are the FAST follow-up that
//! must reproduce these results bit-for-bit.

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
    mag: Mag,
}

/// The magnitude of a [`BigInt`]: a single inline limb (the overwhelmingly
/// common case — no heap allocation) or heap limbs for multi-limb values.
///
/// INVARIANT: `Heap` holds ≥ 2 normalized limbs (no trailing zeros); every
/// 0- or 1-limb magnitude is `Inline` (zero is `Inline(0)`). The variant is
/// therefore canonical per value, so the derived `PartialEq`/`Hash` agree
/// with numeric equality.
#[derive(Clone, PartialEq, Eq, Hash)]
enum Mag {
    Inline(u64),
    Heap(Vec<u64>),
}

impl Mag {
    /// Canonicalize a limb vector (trim trailing zeros, inline when it fits).
    fn from_vec(mut v: Vec<u64>) -> Mag {
        while v.last() == Some(&0) {
            v.pop();
        }
        match v.len() {
            0 => Mag::Inline(0),
            1 => Mag::Inline(v[0]),
            _ => Mag::Heap(v),
        }
    }

    /// The normalized little-endian limbs (empty ⇔ zero).
    fn limbs(&self) -> &[u64] {
        match self {
            Mag::Inline(0) => &[],
            Mag::Inline(x) => std::slice::from_ref(x),
            Mag::Heap(v) => v,
        }
    }

    /// The single limb when the magnitude fits one (`Some(0)` for zero),
    /// `None` for multi-limb values.
    fn as_single(&self) -> Option<u64> {
        match self {
            Mag::Inline(x) => Some(*x),
            Mag::Heap(_) => None,
        }
    }
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
        BigInt { sign: Sign::Zero, mag: Mag::Inline(0) }
    }

    /// Build from a sign flag and a (not-necessarily-normalized) magnitude,
    /// re-establishing the canonical form (trim zeros; empty magnitude ⇒ Zero).
    fn from_sign_mag(neg: bool, mag: Vec<u64>) -> Self {
        let mag = Mag::from_vec(mag);
        let sign = if mag.limbs().is_empty() {
            Sign::Zero
        } else if neg {
            Sign::Neg
        } else {
            Sign::Pos
        };
        BigInt { sign, mag }
    }

    /// Build from a sign flag and a ≤128-bit magnitude — the allocation-free
    /// constructor the single-limb fast paths return through.
    fn from_sign_mag_u128(neg: bool, m: u128) -> Self {
        if m == 0 {
            return Self::zero();
        }
        let lo = m as u64;
        let hi = (m >> 64) as u64;
        let mag = if hi == 0 { Mag::Inline(lo) } else { Mag::Heap(vec![lo, hi]) };
        BigInt { sign: if neg { Sign::Neg } else { Sign::Pos }, mag }
    }

    /// Exact widening from a machine integer.
    pub fn from_i64(x: i64) -> Self {
        Self::from_sign_mag_u128(x < 0, (x as i128).unsigned_abs())
    }

    /// Exact widening from an unsigned machine integer.
    pub fn from_u64(x: u64) -> Self {
        Self::from_sign_mag_u128(false, x as u128)
    }

    /// Narrow back to an `i64` iff the value fits — the basis of the "downsize when
    /// it provably fits" path the runtime uses to stay on the fast i64 repr.
    pub fn to_i64(&self) -> Option<i64> {
        let m = self.mag.as_single()? as i128;
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

    /// Parity (the low bit of the magnitude — sign does not change it). Zero is even.
    pub fn is_odd(&self) -> bool {
        self.mag.limbs().first().is_some_and(|limb| limb & 1 == 1)
    }

    /// Nearest `f64` (lossy by nature — float semantics) for mixed int/float math.
    pub fn to_f64(&self) -> f64 {
        const TWO64: f64 = 18_446_744_073_709_551_616.0; // 2^64
        let mut acc = 0.0f64;
        for &limb in self.mag.limbs().iter().rev() {
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

    pub fn is_positive(&self) -> bool {
        self.sign == Sign::Pos
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
        // Single-limb fast path: both magnitudes fit a limb, so the signed sum
        // fits an i128 — no limb vectors, no allocation.
        if let (Some(a), Some(b)) = (self.mag.as_single(), other.mag.as_single()) {
            let av = if self.sign == Sign::Neg { -(a as i128) } else { a as i128 };
            let bv = if other.sign == Sign::Neg { -(b as i128) } else { b as i128 };
            let sum = av + bv;
            return Self::from_sign_mag_u128(sum < 0, sum.unsigned_abs());
        }
        match (self.sign, other.sign) {
            (Sign::Zero, _) => other.clone(),
            (_, Sign::Zero) => self.clone(),
            // Same sign: add magnitudes, keep the sign.
            (a, b) if a == b => {
                Self::from_sign_mag(a == Sign::Neg, mag_add(self.mag.limbs(), other.mag.limbs()))
            }
            // Opposite signs: subtract the smaller magnitude from the larger; the
            // result takes the sign of the larger.
            _ => match mag_cmp(self.mag.limbs(), other.mag.limbs()) {
                Ordering::Equal => Self::zero(),
                Ordering::Greater => Self::from_sign_mag(
                    self.sign == Sign::Neg,
                    mag_sub(self.mag.limbs(), other.mag.limbs()),
                ),
                Ordering::Less => Self::from_sign_mag(
                    other.sign == Sign::Neg,
                    mag_sub(other.mag.limbs(), self.mag.limbs()),
                ),
            },
        }
    }

    /// `self - other`.
    pub fn sub(&self, other: &Self) -> Self {
        self.add(&other.negated())
    }

    /// `self * other`.
    pub fn mul(&self, other: &Self) -> Self {
        let neg = (self.sign == Sign::Neg) ^ (other.sign == Sign::Neg);
        // Single-limb fast path: the product fits a u128.
        if let (Some(a), Some(b)) = (self.mag.as_single(), other.mag.as_single()) {
            return Self::from_sign_mag_u128(neg, (a as u128) * (b as u128));
        }
        if self.is_zero() || other.is_zero() {
            return Self::zero();
        }
        Self::from_sign_mag(neg, mag_mul(self.mag.limbs(), other.mag.limbs()))
    }

    /// Truncated division toward zero: returns `(quotient, remainder)` with
    /// `self = q*other + r` and the remainder carrying the dividend's sign — exactly
    /// matching Rust/`i64` `/` and `%`, so the wide type is a drop-in for the narrow.
    /// `None` when `other` is zero.
    pub fn div_rem(&self, other: &Self) -> Option<(Self, Self)> {
        if other.is_zero() {
            return None;
        }
        let q_neg = (self.sign == Sign::Neg) ^ (other.sign == Sign::Neg);
        // Single-limb fast path: machine division gives the truncated pair
        // directly (the remainder takes the dividend's sign).
        if let (Some(a), Some(b)) = (self.mag.as_single(), other.mag.as_single()) {
            return Some((
                Self::from_sign_mag_u128(q_neg, (a / b) as u128),
                Self::from_sign_mag_u128(self.sign == Sign::Neg, (a % b) as u128),
            ));
        }
        if self.is_zero() {
            return Some((Self::zero(), Self::zero()));
        }
        let (qm, rm) = mag_divrem(self.mag.limbs(), other.mag.limbs());
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
        let limbs = self.mag.limbs();
        let mut bytes = Vec::with_capacity(limbs.len() * 8);
        for &limb in limbs {
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
                Sign::Pos => mag_cmp(self.mag.limbs(), other.mag.limbs()),
                Sign::Neg => mag_cmp(other.mag.limbs(), self.mag.limbs()),
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
            chunks.push(r.mag.limbs().first().copied().unwrap_or(0));
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

    pub fn is_positive(&self) -> bool {
        self.num.is_positive()
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

// =====================================================================
// Decimal — exact base-10 fixed-point on top of BigInt
// =====================================================================

/// How a [`Decimal`] resolves the digits it must drop when rounding to a smaller
/// scale. `HalfEven` (banker's rounding) is the money/finance default: the unbiased
/// tie-break that does not drift a long column of sums upward.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RoundingMode {
    /// Toward zero (truncate).
    Down,
    /// Away from zero.
    Up,
    /// Toward −∞.
    Floor,
    /// Toward +∞.
    Ceiling,
    /// Nearest; ties away from zero.
    HalfUp,
    /// Nearest; ties toward zero.
    HalfDown,
    /// Nearest; ties to the even neighbour (banker's rounding).
    HalfEven,
}

/// `10^k` as a [`BigInt`].
fn ten_pow(k: u32) -> BigInt {
    BigInt::from_u64(10).pow(k)
}

/// Round a rational to the nearest integer under `mode`. The single rounding
/// primitive: [`Decimal`] scale changes and division both funnel through here, so
/// every mode behaves identically everywhere.
fn round_rational_to_bigint(x: &Rational, mode: RoundingMode) -> BigInt {
    let num = x.numerator();
    let den = x.denominator(); // INVARIANT (Rational): den > 0.
    let (q, r) = num.div_rem(den).expect("rational denominator is nonzero");
    if r.is_zero() {
        return q; // exact — no rounding to do.
    }
    let neg = num.is_negative();
    // Compare 2·|r| against den to place the fraction relative to the half-way point.
    let twice = r.abs().mul(&BigInt::from_i64(2));
    let half = twice.cmp(den);
    let away = match mode {
        RoundingMode::Down => false,
        RoundingMode::Up => true,
        RoundingMode::Floor => neg,
        RoundingMode::Ceiling => !neg,
        RoundingMode::HalfUp => half != Ordering::Less,
        RoundingMode::HalfDown => half == Ordering::Greater,
        RoundingMode::HalfEven => half == Ordering::Greater || (half == Ordering::Equal && q.is_odd()),
    };
    if away {
        // `q` is truncated toward zero, so rounding away adds the value's sign.
        if neg { q.sub(&BigInt::from_i64(1)) } else { q.add(&BigInt::from_i64(1)) }
    } else {
        q
    }
}

/// An exact base-10 fixed-point number: an integer `coefficient` divided by a power
/// of ten. The value is `coefficient · 10^(−scale)`, so `19.99` is `coefficient =
/// 1999`, `scale = 2`. Money lives here — `0.1 + 0.2` is exactly `0.3`, never the
/// `0.30000000000000004` of a binary float, and a long ledger of sums never drifts.
///
/// `+`, `−`, `×` are exact (multiplication adds the scales); `÷` and [`Decimal::rescale`]
/// round under an explicit [`RoundingMode`] because base-10 division need not
/// terminate. Equality and ordering are by **value** (`1.0 == 1.00 == 1`); `scale`
/// is only a display hint. The coefficient is a [`BigInt`], so the value is unbounded
/// and exact — the i128 fast path is the documented performance follow-up.
#[derive(Clone)]
pub struct Decimal {
    coeff: BigInt,
    /// Number of base-10 fractional digits (value = `coeff / 10^scale`).
    scale: u32,
}

impl Decimal {
    /// The integer `coeff` at scale 0.
    pub fn from_bigint(coeff: BigInt) -> Decimal {
        Decimal { coeff, scale: 0 }
    }

    pub fn from_i64(x: i64) -> Decimal {
        Decimal::from_bigint(BigInt::from_i64(x))
    }

    /// Construct directly from a coefficient and scale (value = `coeff / 10^scale`).
    pub fn from_coeff_scale(coeff: BigInt, scale: u32) -> Decimal {
        Decimal { coeff, scale }
    }

    pub fn zero() -> Decimal {
        Decimal::from_i64(0)
    }

    pub fn one() -> Decimal {
        Decimal::from_i64(1)
    }

    pub fn scale(&self) -> u32 {
        self.scale
    }

    pub fn coefficient(&self) -> &BigInt {
        &self.coeff
    }

    pub fn is_zero(&self) -> bool {
        self.coeff.is_zero()
    }

    pub fn is_negative(&self) -> bool {
        self.coeff.is_negative()
    }

    pub fn negated(&self) -> Decimal {
        Decimal { coeff: self.coeff.negated(), scale: self.scale }
    }

    pub fn abs(&self) -> Decimal {
        Decimal { coeff: self.coeff.abs(), scale: self.scale }
    }

    /// Exact view as a [`Rational`] (`coeff / 10^scale`) — lossless, the bridge that
    /// keeps `Decimal` inside the exact tower.
    pub fn to_rational(&self) -> Rational {
        Rational::new(self.coeff.clone(), ten_pow(self.scale)).expect("10^scale is nonzero")
    }

    /// The fixed-point value nearest `value` at the given `scale`, rounded under `mode`.
    pub fn from_rational(value: &Rational, scale: u32, mode: RoundingMode) -> Decimal {
        let scaled = value.mul(&Rational::from_bigint(ten_pow(scale)));
        Decimal { coeff: round_rational_to_bigint(&scaled, mode), scale }
    }

    /// Restate at a different scale. Widening appends zeros (exact); narrowing drops
    /// digits and rounds under `mode`.
    pub fn rescale(&self, scale: u32, mode: RoundingMode) -> Decimal {
        if scale >= self.scale {
            Decimal { coeff: self.coeff.mul(&ten_pow(scale - self.scale)), scale }
        } else {
            Decimal::from_rational(&self.to_rational(), scale, mode)
        }
    }

    /// Bring two values to a common scale (the larger) so coefficients line up.
    fn aligned(&self, other: &Decimal) -> (BigInt, BigInt, u32) {
        let s = self.scale.max(other.scale);
        let a = self.coeff.mul(&ten_pow(s - self.scale));
        let b = other.coeff.mul(&ten_pow(s - other.scale));
        (a, b, s)
    }

    pub fn add(&self, other: &Decimal) -> Decimal {
        let (a, b, s) = self.aligned(other);
        Decimal { coeff: a.add(&b), scale: s }
    }

    pub fn sub(&self, other: &Decimal) -> Decimal {
        let (a, b, s) = self.aligned(other);
        Decimal { coeff: a.sub(&b), scale: s }
    }

    /// Exact product: coefficients multiply, scales add.
    pub fn mul(&self, other: &Decimal) -> Decimal {
        Decimal { coeff: self.coeff.mul(&other.coeff), scale: self.scale + other.scale }
    }

    /// Quotient rounded to `scale` under `mode`. `None` when `other == 0` — base-10
    /// division need not terminate, so the caller names the precision it wants.
    pub fn div(&self, other: &Decimal, scale: u32, mode: RoundingMode) -> Option<Decimal> {
        let q = self.to_rational().div(&other.to_rational())?;
        Some(Decimal::from_rational(&q, scale, mode))
    }

    /// Minimal-scale form (trailing zeros stripped) — the canonical value used for
    /// `Eq`/`Hash` so `1.0`, `1.00`, and `1` are one key.
    fn normalized(&self) -> (BigInt, u32) {
        if self.coeff.is_zero() {
            return (BigInt::zero(), 0);
        }
        let ten = BigInt::from_u64(10);
        let mut c = self.coeff.clone();
        let mut s = self.scale;
        while s > 0 {
            let (q, r) = c.div_rem(&ten).expect("ten is nonzero");
            if !r.is_zero() {
                break;
            }
            c = q;
            s -= 1;
        }
        (c, s)
    }

    /// Sign + little-endian coefficient bytes + scale — the wire form (the inverse of
    /// [`Decimal::from_le_bytes`]). Reuses [`BigInt`]'s exact byte serialization.
    pub fn to_le_bytes(&self) -> (bool, Vec<u8>, u32) {
        let (neg, bytes) = self.coeff.to_le_bytes();
        (neg, bytes, self.scale)
    }

    pub fn from_le_bytes(negative: bool, bytes: &[u8], scale: u32) -> Decimal {
        Decimal { coeff: BigInt::from_le_bytes(negative, bytes), scale }
    }

    /// Parse plain decimal notation: optional sign, optional integer part, optional
    /// `.fraction`. `"19.99"`, `"-0.005"`, `".5"`, `"5."`, `"42"` all parse; the scale
    /// is the count of fractional digits. `None` on a non-digit or an empty value.
    pub fn parse(s: &str) -> Option<Decimal> {
        let s = s.trim();
        let (neg, body) = match s.as_bytes().first() {
            Some(b'-') => (true, &s[1..]),
            Some(b'+') => (false, &s[1..]),
            _ => (false, s),
        };
        if body.is_empty() {
            return None;
        }
        let (int_part, frac_part) = match body.split_once('.') {
            Some((i, f)) => (i, f),
            None => (body, ""),
        };
        if int_part.is_empty() && frac_part.is_empty() {
            return None; // a lone "." is not a number.
        }
        if !int_part.bytes().all(|c| c.is_ascii_digit()) || !frac_part.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let digits = format!("{int_part}{frac_part}");
        let mag = if digits.is_empty() { BigInt::zero() } else { BigInt::parse_decimal(&digits)? };
        let coeff = if neg { mag.negated() } else { mag };
        Some(Decimal { coeff, scale: frac_part.len() as u32 })
    }
}

impl PartialEq for Decimal {
    fn eq(&self, other: &Self) -> bool {
        self.normalized() == other.normalized()
    }
}

impl Eq for Decimal {}

impl std::hash::Hash for Decimal {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.normalized().hash(state);
    }
}

impl Ord for Decimal {
    fn cmp(&self, other: &Self) -> Ordering {
        // Compare by value (cross-scale, no rounding) via the exact rational view.
        self.to_rational().cmp(&other.to_rational())
    }
}

impl PartialOrd for Decimal {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.scale == 0 {
            return write!(f, "{}", self.coeff);
        }
        let neg = self.coeff.is_negative();
        let mut digits = self.coeff.abs().to_string();
        let scale = self.scale as usize;
        if digits.len() <= scale {
            // Pad so there is at least one integer digit (e.g. "5" at scale 3 → "0005").
            digits = format!("{}{}", "0".repeat(scale + 1 - digits.len()), digits);
        }
        let point = digits.len() - scale;
        if neg {
            write!(f, "-")?;
        }
        write!(f, "{}.{}", &digits[..point], &digits[point..])
    }
}

impl fmt::Debug for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Decimal({self})")
    }
}

impl From<i64> for Decimal {
    fn from(x: i64) -> Self {
        Decimal::from_i64(x)
    }
}

impl From<BigInt> for Decimal {
    fn from(x: BigInt) -> Self {
        Decimal::from_bigint(x)
    }
}

// =====================================================================
// Complex — exact Gaussian rationals on top of Rational
// =====================================================================

/// An exact complex number `re + im·i`, each part a [`Rational`]. Because the parts
/// are exact, `i·i == −1` and `(1+i)(1−i) == 2` hold with no floating error, and the
/// Gaussian rationals are closed under `+ − × ÷` (every nonzero value has an exact
/// inverse). The magnitude `√(re²+im²)` is irrational in general — request it as an
/// `f64` view via [`Complex::abs_f64`] rather than forcing the exact value inexact.
///
/// Complex numbers are NOT ordered, so there is deliberately no `Ord`/`PartialOrd`.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Complex {
    re: Rational,
    im: Rational,
}

impl Complex {
    pub fn new(re: Rational, im: Rational) -> Complex {
        Complex { re, im }
    }

    /// A real number as `re + 0i`.
    pub fn from_rational(re: Rational) -> Complex {
        Complex { re, im: Rational::zero() }
    }

    pub fn from_i64(x: i64) -> Complex {
        Complex::from_rational(Rational::from_i64(x))
    }

    pub fn zero() -> Complex {
        Complex { re: Rational::zero(), im: Rational::zero() }
    }

    pub fn one() -> Complex {
        Complex::from_i64(1)
    }

    /// The imaginary unit `i` (`0 + 1i`).
    pub fn i() -> Complex {
        Complex { re: Rational::zero(), im: Rational::one() }
    }

    pub fn re(&self) -> &Rational {
        &self.re
    }

    pub fn im(&self) -> &Rational {
        &self.im
    }

    pub fn is_zero(&self) -> bool {
        self.re.is_zero() && self.im.is_zero()
    }

    /// True when the imaginary part is zero (the value is a plain real).
    pub fn is_real(&self) -> bool {
        self.im.is_zero()
    }

    /// `re − im·i`.
    pub fn conjugate(&self) -> Complex {
        Complex { re: self.re.clone(), im: self.im.negated() }
    }

    /// `re² + im²` — the squared magnitude, exact (a nonnegative [`Rational`]).
    pub fn norm_sqr(&self) -> Rational {
        self.re.mul(&self.re).add(&self.im.mul(&self.im))
    }

    /// The magnitude `√(re²+im²)` as an `f64` (lossy by nature — the exact value is
    /// generally irrational). The components stay exact; this is a view.
    pub fn abs_f64(&self) -> f64 {
        self.norm_sqr().to_f64().sqrt()
    }

    pub fn negated(&self) -> Complex {
        Complex { re: self.re.negated(), im: self.im.negated() }
    }

    pub fn add(&self, other: &Complex) -> Complex {
        Complex { re: self.re.add(&other.re), im: self.im.add(&other.im) }
    }

    pub fn sub(&self, other: &Complex) -> Complex {
        Complex { re: self.re.sub(&other.re), im: self.im.sub(&other.im) }
    }

    /// `(a+bi)(c+di) = (ac − bd) + (ad + bc)i`.
    pub fn mul(&self, other: &Complex) -> Complex {
        let (a, b, c, d) = (&self.re, &self.im, &other.re, &other.im);
        Complex { re: a.mul(c).sub(&b.mul(d)), im: a.mul(d).add(&b.mul(c)) }
    }

    /// `1/self` via the conjugate over the squared magnitude — `None` only for zero.
    pub fn recip(&self) -> Option<Complex> {
        let n = self.norm_sqr();
        if n.is_zero() {
            return None;
        }
        let inv = n.recip().expect("norm is nonzero here");
        Some(Complex { re: self.re.mul(&inv), im: self.im.negated().mul(&inv) })
    }

    /// `self / other` — `None` when `other == 0`.
    pub fn div(&self, other: &Complex) -> Option<Complex> {
        Some(self.mul(&other.recip()?))
    }

    /// `self` raised to an integer power. Negative exponents take the reciprocal
    /// first; `None` only for zero raised to a negative power. `0^0 == 1`.
    pub fn pow(&self, exp: i32) -> Option<Complex> {
        if exp < 0 {
            return self.recip()?.pow(-exp);
        }
        let mut result = Complex::one();
        let mut base = self.clone();
        let mut e = exp as u32;
        while e > 0 {
            if e & 1 == 1 {
                result = result.mul(&base);
            }
            e >>= 1;
            if e > 0 {
                base = base.mul(&base);
            }
        }
        Some(result)
    }

    /// Parse `"3+4i"`, `"3-4i"`, `"4i"`, `"-i"`, `"i"`, or a bare real `"3"` /`"1/2"`.
    /// Round-trips the [`Display`](fmt::Display) form. `None` on malformed input.
    pub fn parse(s: &str) -> Option<Complex> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        let bytes = s.as_bytes();
        if bytes[bytes.len() - 1] != b'i' {
            // No imaginary part: a plain real.
            return Some(Complex::from_rational(Rational::parse(s)?));
        }
        let body = &s[..s.len() - 1]; // strip trailing 'i'
        // Split into real and imaginary at the last sign that is not the leading one.
        let split = body
            .bytes()
            .enumerate()
            .rev()
            .find(|&(i, c)| i > 0 && (c == b'+' || c == b'-'))
            .map(|(i, _)| i);
        let (real_str, imag_str) = match split {
            Some(i) => (&body[..i], &body[i..]),
            None => ("", body),
        };
        let re = if real_str.is_empty() { Rational::zero() } else { Rational::parse(real_str)? };
        let im = match imag_str {
            "" | "+" => Rational::one(),
            "-" => Rational::from_i64(-1),
            other => Rational::parse(other)?,
        };
        Some(Complex { re, im })
    }
}

impl fmt::Display for Complex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.im.is_zero() {
            return write!(f, "{}", self.re);
        }
        let mag = self.im.abs();
        let unit = mag == Rational::one(); // omit the coefficient when it is 1
        if self.re.is_zero() {
            // Pure imaginary.
            return if self.im.is_negative() {
                if unit { write!(f, "-i") } else { write!(f, "-{mag}i") }
            } else if unit {
                write!(f, "i")
            } else {
                write!(f, "{mag}i")
            };
        }
        let sign = if self.im.is_negative() { "-" } else { "+" };
        if unit {
            write!(f, "{}{}i", self.re, sign)
        } else {
            write!(f, "{}{}{}i", self.re, sign, mag)
        }
    }
}

impl fmt::Debug for Complex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Complex({self})")
    }
}

impl From<i64> for Complex {
    fn from(x: i64) -> Self {
        Complex::from_i64(x)
    }
}

impl From<Rational> for Complex {
    fn from(x: Rational) -> Self {
        Complex::from_rational(x)
    }
}

// =====================================================================
// Modular — the ring ℤ/nℤ over an arbitrary modulus, on top of BigInt
// =====================================================================

/// Extended Euclid on `BigInt`: returns `(g, x, y)` with `a·x + b·y = g`, `g = gcd(a, b)`.
/// The basis for the modular inverse (`g == 1 ⇒ x` is `a⁻¹ mod b`).
fn bigint_egcd(a: &BigInt, b: &BigInt) -> (BigInt, BigInt, BigInt) {
    let (mut old_r, mut r) = (a.clone(), b.clone());
    let (mut old_s, mut s) = (BigInt::from_i64(1), BigInt::zero());
    let (mut old_t, mut t) = (BigInt::zero(), BigInt::from_i64(1));
    while !r.is_zero() {
        let (q, _) = old_r.div_rem(&r).expect("r is nonzero inside the loop");
        let nr = old_r.sub(&q.mul(&r));
        old_r = std::mem::replace(&mut r, nr);
        let ns = old_s.sub(&q.mul(&s));
        old_s = std::mem::replace(&mut s, ns);
        let nt = old_t.sub(&q.mul(&t));
        old_t = std::mem::replace(&mut t, nt);
    }
    (old_r, old_s, old_t)
}

/// An element of the ring ℤ/nℤ — an integer taken modulo a fixed `modulus`. This is the
/// arbitrary-modulus generalisation of [`crate::Word8`]/…/[`crate::Word64`] (whose modulus is a power of
/// two): arithmetic WRAPS into `[0, modulus)`, the cyclic group where overflow is the point,
/// not a bug. The number-theory / crypto substrate — modular exponentiation and the
/// extended-Euclid inverse live here, so RSA-style and finite-field code is exact.
///
/// INVARIANT: `0 ≤ value < modulus` and `modulus ≥ 1` (built via [`Modular::new`], which
/// reduces into range), so equal residues share one representation (`Eq`/`Hash` are structural).
/// NOT ordered (ℤ/nℤ has no canonical total order).
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Modular {
    value: BigInt,
    modulus: BigInt,
}

/// Euclidean reduction into `[0, modulus)` (the remainder is made non-negative).
fn mod_reduce(value: BigInt, modulus: &BigInt) -> BigInt {
    let (_, r) = value.div_rem(modulus).expect("modulus is nonzero");
    if r.is_negative() {
        r.add(modulus)
    } else {
        r
    }
}

impl Modular {
    /// Reduce `value` into the ring ℤ/`modulus`ℤ. `None` for a non-positive modulus.
    pub fn new(value: BigInt, modulus: BigInt) -> Option<Modular> {
        if modulus.is_zero() || modulus.is_negative() {
            return None;
        }
        let value = mod_reduce(value, &modulus);
        Some(Modular { value, modulus })
    }

    /// Convenience over machine integers (`None` if `modulus ≤ 0`).
    pub fn from_i64(value: i64, modulus: i64) -> Option<Modular> {
        Modular::new(BigInt::from_i64(value), BigInt::from_i64(modulus))
    }

    /// The canonical representative in `[0, modulus)`.
    pub fn value(&self) -> &BigInt {
        &self.value
    }

    pub fn modulus(&self) -> &BigInt {
        &self.modulus
    }

    pub fn is_zero(&self) -> bool {
        self.value.is_zero()
    }

    /// `self + other` — `None` when the moduli differ (a ring mismatch, like a Word width mismatch).
    pub fn add(&self, other: &Modular) -> Option<Modular> {
        if self.modulus != other.modulus {
            return None;
        }
        Modular::new(self.value.add(&other.value), self.modulus.clone())
    }

    pub fn sub(&self, other: &Modular) -> Option<Modular> {
        if self.modulus != other.modulus {
            return None;
        }
        Modular::new(self.value.sub(&other.value), self.modulus.clone())
    }

    pub fn mul(&self, other: &Modular) -> Option<Modular> {
        if self.modulus != other.modulus {
            return None;
        }
        Modular::new(self.value.mul(&other.value), self.modulus.clone())
    }

    /// The additive inverse `−self` (i.e. `modulus − value`, reduced).
    pub fn negated(&self) -> Modular {
        Modular::new(self.value.negated(), self.modulus.clone()).expect("modulus stays valid")
    }

    /// `self^exp` by fast modular exponentiation (square-and-multiply). `0^0 == 1`.
    pub fn pow(&self, mut exp: u64) -> Modular {
        let one = Modular::new(BigInt::from_i64(1), self.modulus.clone()).expect("modulus valid");
        let mut result = one;
        let mut base = self.clone();
        while exp > 0 {
            if exp & 1 == 1 {
                result = result.mul(&base).expect("same modulus");
            }
            exp >>= 1;
            if exp > 0 {
                base = base.mul(&base).expect("same modulus");
            }
        }
        result
    }

    /// The multiplicative inverse `self⁻¹` (extended Euclid). `None` unless
    /// `gcd(value, modulus) == 1` (the value must be a unit of the ring).
    pub fn inverse(&self) -> Option<Modular> {
        let (g, x, _) = bigint_egcd(&self.value, &self.modulus);
        if g != BigInt::from_i64(1) {
            return None;
        }
        Modular::new(x, self.modulus.clone())
    }

    /// `self / other = self · other⁻¹` — `None` on a modulus mismatch or a non-invertible divisor.
    pub fn div(&self, other: &Modular) -> Option<Modular> {
        if self.modulus != other.modulus {
            return None;
        }
        self.mul(&other.inverse()?)
    }
}

impl fmt::Display for Modular {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (mod {})", self.value, self.modulus)
    }
}

impl fmt::Debug for Modular {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Modular({} mod {})", self.value, self.modulus)
    }
}

// ─── Exact float interop ────────────────────────────────────────────────────
//
// Cross-type numeric comparison in LOGOS is EXACT — mathematical values, never
// a lossy cast (`i as f64` rounds above 2^53, so a cast-based `==` would call
// `9007199254740993` equal to the float `9007199254740992.0`). Every finite
// f64 is exactly `±m · 2^e`, so the exact answer is always computable through
// the BigInt/Rational tower. CPython uses the same model.

/// The Mersenne prime 2^61 − 1: the modulus of the UNIFIED numeric hash.
/// Every numeric type hashes to its mathematical value mod P, so values that
/// compare equal across types (`1 == 1.0 == 1/1`) hash equal — the hash/
/// equality coherence law that lets mixed-type map keys unify.
pub const NUMERIC_HASH_P: u64 = (1u64 << 61) - 1;

/// Decompose a finite f64 into `(negative, mantissa, exponent)` with
/// value = ±mantissa · 2^exponent (mantissa integral). `None` for NaN/±inf.
fn decompose_f64(f: f64) -> Option<(bool, u64, i32)> {
    if !f.is_finite() {
        return None;
    }
    let bits = f.to_bits();
    let neg = bits >> 63 == 1;
    let biased = ((bits >> 52) & 0x7ff) as i32;
    let frac = bits & ((1u64 << 52) - 1);
    let (m, e) = if biased == 0 {
        (frac, -1074) // subnormal (or zero)
    } else {
        (frac | (1u64 << 52), biased - 1075)
    };
    Some((neg, m, e))
}

/// The EXACT rational value of a finite f64. `None` for NaN/±inf.
pub fn rational_from_f64_exact(f: f64) -> Option<Rational> {
    let (neg, m, e) = decompose_f64(f)?;
    let mag = BigInt::from_u64(m);
    let two = BigInt::from_u64(2);
    let (num, den) = if e >= 0 {
        (mag.mul(&two.pow(e as u32)), BigInt::from_i64(1))
    } else {
        (mag, two.pow((-e) as u32))
    };
    let num = if neg { num.negated() } else { num };
    Rational::new(num, den)
}

/// Exact comparison of an i64 against an f64. `None` iff `f` is NaN.
pub fn cmp_i64_f64_exact(i: i64, f: f64) -> Option<Ordering> {
    if f.is_nan() {
        return None;
    }
    if f == f64::INFINITY {
        return Some(Ordering::Less);
    }
    if f == f64::NEG_INFINITY {
        return Some(Ordering::Greater);
    }
    // Fast path: |i| ≤ 2^53 is exactly representable, so the f64 compare IS
    // exact (the common case — small ints against floats).
    if i.unsigned_abs() <= (1u64 << 53) {
        return (i as f64).partial_cmp(&f);
    }
    let fr = rational_from_f64_exact(f).expect("finite f64 is an exact rational");
    Some(Rational::from_i64(i).cmp(&fr))
}

/// Exact comparison of a BigInt against an f64. `None` iff `f` is NaN.
pub fn cmp_bigint_f64_exact(b: &BigInt, f: f64) -> Option<Ordering> {
    if f.is_nan() {
        return None;
    }
    if f == f64::INFINITY {
        return Some(Ordering::Less);
    }
    if f == f64::NEG_INFINITY {
        return Some(Ordering::Greater);
    }
    let fr = rational_from_f64_exact(f).expect("finite f64 is an exact rational");
    Some(Rational::from_bigint(b.clone()).cmp(&fr))
}

/// Exact comparison of a Rational against an f64. `None` iff `f` is NaN.
pub fn cmp_rational_f64_exact(r: &Rational, f: f64) -> Option<Ordering> {
    if f.is_nan() {
        return None;
    }
    if f == f64::INFINITY {
        return Some(Ordering::Less);
    }
    if f == f64::NEG_INFINITY {
        return Some(Ordering::Greater);
    }
    let fr = rational_from_f64_exact(f).expect("finite f64 is an exact rational");
    Some(r.cmp(&fr))
}

/// Reduce a u64 mod P (one Mersenne fold suffices: `x >> 61 ≤ 7`).
fn mod_p(x: u64) -> u64 {
    let r = (x & NUMERIC_HASH_P) + (x >> 61);
    if r >= NUMERIC_HASH_P { r - NUMERIC_HASH_P } else { r }
}

fn mul_mod_p(a: u64, b: u64) -> u64 {
    ((a as u128 * b as u128) % (NUMERIC_HASH_P as u128)) as u64
}

fn pow_mod_p(mut base: u64, mut exp: u64) -> u64 {
    let mut acc = 1u64;
    base %= NUMERIC_HASH_P;
    while exp > 0 {
        if exp & 1 == 1 {
            acc = mul_mod_p(acc, base);
        }
        base = mul_mod_p(base, base);
        exp >>= 1;
    }
    acc
}

/// Apply the sign to a magnitude hash: negatives map to `P − h` (0 fixed),
/// shared by every numeric hasher so equal values agree.
fn signed_hash(neg: bool, h: u64) -> u64 {
    if neg && h != 0 { NUMERIC_HASH_P - h } else { h }
}

/// The unified numeric hash of an i64 (its value mod P, sign-adjusted).
pub fn numeric_hash_i64(n: i64) -> u64 {
    signed_hash(n < 0, mod_p(n.unsigned_abs()))
}

/// The unified numeric hash of a BigInt. Limbs fold most-significant first:
/// 2^64 ≡ 8 (mod 2^61 − 1).
pub fn numeric_hash_bigint(b: &BigInt) -> u64 {
    let mut acc: u64 = 0;
    for &limb in b.mag.limbs().iter().rev() {
        let t = (acc as u128) * 8 + (mod_p(limb) as u128);
        acc = (t % (NUMERIC_HASH_P as u128)) as u64;
    }
    signed_hash(b.is_negative(), acc)
}

/// The unified numeric hash of an f64: `m · 2^e mod P` via modular
/// exponentiation (2^61 ≡ 1, so the exponent reduces mod 61). NaN and the
/// infinities take fixed sentinels outside the finite-value pattern.
pub fn numeric_hash_f64(f: f64) -> u64 {
    if f.is_nan() {
        return 0x6e616e; // "nan"
    }
    if f == f64::INFINITY {
        return 314159;
    }
    if f == f64::NEG_INFINITY {
        return NUMERIC_HASH_P - 314159;
    }
    let (neg, m, e) = decompose_f64(f).expect("finite");
    let h = mul_mod_p(mod_p(m), pow_mod_p(2, e.rem_euclid(61) as u64));
    signed_hash(neg, h)
}

/// The unified numeric hash of a Rational: `num · den⁻¹ mod P` (Fermat
/// inverse; P is prime, and a reduced denominator is never ≡ 0 mod P for
/// representable values).
pub fn numeric_hash_rational(r: &Rational) -> u64 {
    let num = numeric_hash_bigint(&r.numerator().abs());
    let den = numeric_hash_bigint(&r.denominator().abs());
    let inv = pow_mod_p(den, NUMERIC_HASH_P - 2);
    signed_hash(r.is_negative(), mul_mod_p(num, inv))
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

    // -----------------------------------------------------------------
    // Decimal — exact base-10 fixed-point
    // -----------------------------------------------------------------

    fn dec(s: &str) -> Decimal {
        Decimal::parse(s).unwrap_or_else(|| panic!("parse decimal {s:?}"))
    }

    #[test]
    fn decimal_parse_and_display_round_trip() {
        for s in ["0", "19.99", "0.05", "-0.005", "1999", "100.00", "-7", "0.10", "3.14159"] {
            assert_eq!(dec(s).to_string(), s, "round trip {s}");
        }
        // "-0" and "+5" normalize on display.
        assert_eq!(dec("-0").to_string(), "0");
        assert_eq!(dec("+5").to_string(), "5");
        // leading/trailing forms.
        assert_eq!(dec(".5").to_string(), "0.5");
        assert_eq!(dec("5.").to_string(), "5");
        // garbage rejected.
        assert!(Decimal::parse("1.2.3").is_none());
        assert!(Decimal::parse("abc").is_none());
        assert!(Decimal::parse("").is_none());
        assert!(Decimal::parse("-").is_none());
        assert!(Decimal::parse(".").is_none());
    }

    #[test]
    fn decimal_has_no_binary_float_drift() {
        // The headline: 0.1 + 0.2 == 0.3 EXACTLY (the classic f64 trap), and is shown so.
        assert_ne!(0.1_f64 + 0.2, 0.3);
        assert_eq!(dec("0.1").add(&dec("0.2")), dec("0.3"));
        assert_eq!(dec("0.1").add(&dec("0.2")).to_string(), "0.3");
    }

    #[test]
    fn decimal_add_sub_align_scales() {
        assert_eq!(dec("19.99").add(&dec("0.01")).to_string(), "20.00");
        assert_eq!(dec("1.1").add(&dec("2.22")).to_string(), "3.32");
        assert_eq!(dec("5").add(&dec("0.5")).to_string(), "5.5");
        assert_eq!(dec("20.00").sub(&dec("0.01")).to_string(), "19.99");
        assert_eq!(dec("0").sub(&dec("0.005")).to_string(), "-0.005");
    }

    #[test]
    fn decimal_mul_adds_scales_and_is_exact() {
        assert_eq!(dec("1.1").mul(&dec("1.1")).to_string(), "1.21");
        assert_eq!(dec("0.05").mul(&dec("0.05")).to_string(), "0.0025");
        assert_eq!(dec("19.99").mul(&Decimal::from_i64(3)).to_string(), "59.97");
        assert_eq!(dec("-2.5").mul(&dec("4")).to_string(), "-10.0");
    }

    #[test]
    fn decimal_div_rounds_to_scale() {
        // 10/3 = 3.333… → 3.33 at scale 2.
        assert_eq!(dec("10").div(&dec("3"), 2, RoundingMode::HalfEven).unwrap().to_string(), "3.33");
        // 1/8 = 0.125 exact at scale 3.
        assert_eq!(dec("1").div(&dec("8"), 3, RoundingMode::HalfEven).unwrap().to_string(), "0.125");
        // At scale 2 the tie 0.125 splits by mode: HalfEven → 0.12, HalfUp → 0.13.
        assert_eq!(dec("1").div(&dec("8"), 2, RoundingMode::HalfEven).unwrap().to_string(), "0.12");
        assert_eq!(dec("1").div(&dec("8"), 2, RoundingMode::HalfUp).unwrap().to_string(), "0.13");
        // Divide by zero is None, never a panic.
        assert!(dec("1").div(&dec("0"), 2, RoundingMode::HalfEven).is_none());
    }

    #[test]
    fn decimal_rescale_quantizes_with_rounding() {
        assert_eq!(dec("19.999").rescale(2, RoundingMode::HalfEven).to_string(), "20.00");
        // Banker's rounding: 2.5 → 2, 3.5 → 4, -2.5 → -2 (ties to even).
        assert_eq!(dec("2.5").rescale(0, RoundingMode::HalfEven).to_string(), "2");
        assert_eq!(dec("3.5").rescale(0, RoundingMode::HalfEven).to_string(), "4");
        assert_eq!(dec("-2.5").rescale(0, RoundingMode::HalfEven).to_string(), "-2");
        // Widening just appends zeros (exact).
        assert_eq!(dec("1.5").rescale(4, RoundingMode::HalfEven).to_string(), "1.5000");
    }

    #[test]
    fn decimal_rounding_modes_on_a_tie_and_a_non_tie() {
        // 2.5 (an exact tie) under every mode.
        let tie = dec("2.5");
        for (mode, want) in [
            (RoundingMode::Down, "2"),
            (RoundingMode::Up, "3"),
            (RoundingMode::Floor, "2"),
            (RoundingMode::Ceiling, "3"),
            (RoundingMode::HalfUp, "3"),
            (RoundingMode::HalfDown, "2"),
            (RoundingMode::HalfEven, "2"),
        ] {
            assert_eq!(tie.rescale(0, mode).to_string(), want, "2.5 under {mode:?}");
        }
        // -2.5 (negative tie): Floor → -3, Ceiling → -2, HalfUp → -3.
        let ntie = dec("-2.5");
        assert_eq!(ntie.rescale(0, RoundingMode::Floor).to_string(), "-3");
        assert_eq!(ntie.rescale(0, RoundingMode::Ceiling).to_string(), "-2");
        assert_eq!(ntie.rescale(0, RoundingMode::HalfUp).to_string(), "-3");
        assert_eq!(ntie.rescale(0, RoundingMode::Down).to_string(), "-2");
    }

    #[test]
    fn decimal_equality_is_value_based_and_hash_agrees() {
        use std::collections::HashSet;
        // 1.0 == 1.00 == 1 by VALUE; scale is only a display hint.
        assert_eq!(dec("1.0"), dec("1.00"));
        assert_eq!(dec("1.0"), dec("1"));
        assert_eq!(dec("0.0"), dec("0"));
        // Equal values must hash equal (the Eq/Hash contract).
        let mut set = HashSet::new();
        set.insert(dec("1.00"));
        assert!(set.contains(&dec("1")));
        assert!(set.contains(&dec("1.0")));
        // Different values are not equal.
        assert_ne!(dec("1.0"), dec("1.01"));
    }

    #[test]
    fn decimal_ordering_compares_across_scales_without_rounding() {
        assert!(dec("0.1") < dec("0.11"));
        assert!(dec("1.5") < dec("2"));
        assert!(dec("-0.005") < dec("0"));
        assert!(dec("1.00") == dec("1.0"));
        let mut v = vec![dec("0.5"), dec("0.05"), dec("-0.1"), dec("2"), dec("0.50")];
        v.sort();
        let s: Vec<String> = v.iter().map(|x| x.to_string()).collect();
        // 0.5 and 0.50 are equal; sort is stable so input order among equals is kept.
        assert_eq!(s, ["-0.1", "0.05", "0.5", "0.50", "2"]);
    }

    #[test]
    fn decimal_to_and_from_rational_is_exact() {
        assert_eq!(dec("19.99").to_rational(), Rational::from_ratio_i64(1999, 100).unwrap());
        assert_eq!(dec("0.125").to_rational(), Rational::from_ratio_i64(1, 8).unwrap());
        assert_eq!(dec("-0.005").to_rational(), Rational::from_ratio_i64(-1, 200).unwrap());
        // from_rational rounds to the requested scale.
        let third = Rational::from_ratio_i64(1, 3).unwrap();
        assert_eq!(Decimal::from_rational(&third, 4, RoundingMode::HalfEven).to_string(), "0.3333");
    }

    #[test]
    fn decimal_wire_components_round_trip() {
        for s in ["0", "19.99", "-0.005", "100.00", "123456789.000001", "-7"] {
            let d = dec(s);
            let (neg, bytes, scale) = d.to_le_bytes();
            let back = Decimal::from_le_bytes(neg, &bytes, scale);
            assert_eq!(back, d, "value round-trip {s}");
            assert_eq!(back.to_string(), d.to_string(), "display round-trip {s}");
        }
    }

    #[test]
    fn decimal_money_scenario_is_exact() {
        // 3 items at $19.99, plus 8% tax rounded to cents (banker's).
        let subtotal = dec("19.99").mul(&Decimal::from_i64(3)); // 59.97
        assert_eq!(subtotal.to_string(), "59.97");
        let tax = subtotal.mul(&dec("0.08")).rescale(2, RoundingMode::HalfEven); // 4.7976 → 4.80
        assert_eq!(tax.to_string(), "4.80");
        assert_eq!(subtotal.add(&tax).to_string(), "64.77");
    }

    #[test]
    fn decimal_add_sub_mul_match_the_rational_oracle_under_fuzz() {
        // Differential: build decimals from random coeff/scale and check that +,-,*
        // agree EXACTLY with the Rational tower (the exactness oracle).
        let mut rng = Rng(0xDEC1_2A1B_0000_FACE);
        for _ in 0..3000 {
            let ca = (rng.next() % 2_000_001) as i64 - 1_000_000;
            let cb = (rng.next() % 2_000_001) as i64 - 1_000_000;
            let sa = (rng.next() % 6) as u32;
            let sb = (rng.next() % 6) as u32;
            let a = Decimal::from_coeff_scale(BigInt::from_i64(ca), sa);
            let b = Decimal::from_coeff_scale(BigInt::from_i64(cb), sb);
            let (ra, rb) = (a.to_rational(), b.to_rational());
            assert_eq!(a.add(&b).to_rational(), ra.add(&rb), "add {a} {b}");
            assert_eq!(a.sub(&b).to_rational(), ra.sub(&rb), "sub {a} {b}");
            assert_eq!(a.mul(&b).to_rational(), ra.mul(&rb), "mul {a} {b}");
            // div is rounded: its result must be within one ULP of the requested scale.
            if !b.is_zero() {
                let scale = 8u32;
                let q = a.div(&b, scale, RoundingMode::HalfEven).unwrap();
                let exact = ra.div(&rb).unwrap();
                let ulp = Rational::new(BigInt::from_i64(1), BigInt::from_u64(10).pow(scale)).unwrap();
                let err = q.to_rational().sub(&exact).abs();
                // |rounded - exact| <= ulp/2 for nearest-rounding.
                assert!(err.mul(&Rational::from_i64(2)) <= ulp, "div within half-ulp {a}/{b}");
            }
        }
    }

    // -----------------------------------------------------------------
    // Complex — exact Gaussian rationals
    // -----------------------------------------------------------------

    fn c(re: i64, im: i64) -> Complex {
        Complex::new(Rational::from_i64(re), Rational::from_i64(im))
    }

    fn cq(rn: i64, rd: i64, in_: i64, id: i64) -> Complex {
        Complex::new(Rational::from_ratio_i64(rn, rd).unwrap(), Rational::from_ratio_i64(in_, id).unwrap())
    }

    #[test]
    fn complex_i_squared_is_minus_one() {
        // The headline: i·i == −1, EXACTLY.
        assert_eq!(Complex::i().mul(&Complex::i()), c(-1, 0));
        assert_eq!(Complex::i().mul(&Complex::i()), Complex::from_i64(-1));
    }

    #[test]
    fn complex_construction_and_accessors() {
        let z = c(3, 4);
        assert_eq!(z.re(), &Rational::from_i64(3));
        assert_eq!(z.im(), &Rational::from_i64(4));
        assert!(!z.is_real());
        assert!(c(5, 0).is_real());
        assert!(Complex::zero().is_zero());
        assert!(!Complex::i().is_zero());
        assert_eq!(Complex::from_i64(7), c(7, 0));
    }

    #[test]
    fn complex_add_sub_mul_are_exact() {
        assert_eq!(c(2, 3).add(&c(1, -1)), c(3, 2));
        assert_eq!(c(2, 3).sub(&c(1, -1)), c(1, 4));
        // (1+i)(1−i) = 1 − i² = 2 — the classic exact identity.
        assert_eq!(c(1, 1).mul(&c(1, -1)), c(2, 0));
        // (2+3i)(4+5i) = 8 + 10i + 12i + 15i² = (8−15) + 22i = −7 + 22i.
        assert_eq!(c(2, 3).mul(&c(4, 5)), c(-7, 22));
    }

    #[test]
    fn complex_div_recip_and_zero_guard() {
        // z/z == 1 for any nonzero z.
        assert_eq!(c(2, 3).div(&c(2, 3)).unwrap(), Complex::one());
        // 1/i == −i.
        assert_eq!(Complex::i().recip().unwrap(), c(0, -1));
        // (3+4i)/(1+2i) = (3+4i)(1−2i)/5 = (11−2i)/5.
        assert_eq!(c(3, 4).div(&c(1, 2)).unwrap(), cq(11, 5, -2, 5));
        // Division/reciprocal of zero is None, never a panic.
        assert!(c(1, 1).div(&Complex::zero()).is_none());
        assert!(Complex::zero().recip().is_none());
    }

    #[test]
    fn complex_conjugate_and_norm_are_exact() {
        assert_eq!(c(3, 4).conjugate(), c(3, -4));
        assert_eq!(c(3, 4).norm_sqr(), Rational::from_i64(25));
        // |3+4i| = 5 exactly (a Pythagorean magnitude).
        assert!((c(3, 4).abs_f64() - 5.0).abs() < 1e-12);
        // z · conj(z) == |z|² (a real number).
        let z = c(2, -5);
        assert_eq!(z.mul(&z.conjugate()), Complex::from_rational(z.norm_sqr()));
    }

    #[test]
    fn complex_pow_handles_the_cycle_and_negatives() {
        assert_eq!(Complex::i().pow(2).unwrap(), c(-1, 0));
        assert_eq!(Complex::i().pow(3).unwrap(), c(0, -1));
        assert_eq!(Complex::i().pow(4).unwrap(), c(1, 0));
        assert_eq!(Complex::i().pow(0).unwrap(), Complex::one());
        assert_eq!(Complex::i().pow(-1).unwrap(), c(0, -1)); // 1/i = −i
        // (1+i)^2 = 2i.
        assert_eq!(c(1, 1).pow(2).unwrap(), c(0, 2));
        // 0^0 == 1; 0 to a negative power is undefined.
        assert_eq!(Complex::zero().pow(0).unwrap(), Complex::one());
        assert!(Complex::zero().pow(-1).is_none());
    }

    #[test]
    fn complex_display_round_trips_every_form() {
        for (z, want) in [
            (c(0, 0), "0"),
            (c(3, 0), "3"),
            (c(0, 1), "i"),
            (c(0, -1), "-i"),
            (c(0, 4), "4i"),
            (c(0, -4), "-4i"),
            (c(3, 4), "3+4i"),
            (c(3, -4), "3-4i"),
            (c(3, 1), "3+i"),
            (c(3, -1), "3-i"),
            (c(-2, 5), "-2+5i"),
        ] {
            assert_eq!(z.to_string(), want, "display");
            assert_eq!(Complex::parse(want).unwrap(), z, "parse round-trip of {want}");
        }
        // Fractional parts round-trip too.
        let z = cq(1, 2, -3, 4);
        assert_eq!(z.to_string(), "1/2-3/4i");
        assert_eq!(Complex::parse("1/2-3/4i").unwrap(), z);
        // Garbage is rejected.
        assert!(Complex::parse("").is_none());
        assert!(Complex::parse("3+xi").is_none());
    }

    #[test]
    fn complex_equality_and_hash_are_structural() {
        use std::collections::HashSet;
        assert_eq!(c(3, 4), c(3, 4));
        assert_ne!(c(3, 4), c(3, -4));
        assert_eq!(cq(2, 4, 6, 8), cq(1, 2, 3, 4)); // reduced parts compare equal
        let mut set = HashSet::new();
        set.insert(cq(2, 4, 6, 8));
        assert!(set.contains(&cq(1, 2, 3, 4)));
    }

    #[test]
    fn complex_obeys_the_field_laws_under_fuzz() {
        // Property fuzz: random Gaussian rationals satisfy the field axioms EXACTLY,
        // plus the conjugate/norm homomorphisms.
        let mut rng = Rng(0xC011_9EE5_A11C_E5ED);
        let pick = |rng: &mut Rng| -> Rational {
            let n = (rng.next() % 41) as i64 - 20;
            let d = ((rng.next() % 9) as i64) + 1;
            Rational::from_ratio_i64(n, d).unwrap()
        };
        let pick_c = |rng: &mut Rng| -> Complex { Complex::new(pick(rng), pick(rng)) };
        for _ in 0..2000 {
            let (x, y, z) = (pick_c(&mut rng), pick_c(&mut rng), pick_c(&mut rng));
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
            // conjugate is an involution and a ring homomorphism
            assert_eq!(x.conjugate().conjugate(), x);
            assert_eq!(x.mul(&y).conjugate(), x.conjugate().mul(&y.conjugate()));
            // the norm is multiplicative: |xy|² = |x|²·|y|²
            assert_eq!(x.mul(&y).norm_sqr(), x.norm_sqr().mul(&y.norm_sqr()));
            // multiplicative inverse (when nonzero)
            if !x.is_zero() {
                assert_eq!(x.mul(&x.recip().unwrap()), Complex::one());
                assert_eq!(y.div(&x).unwrap(), y.mul(&x.recip().unwrap()));
            }
        }
    }

    // -----------------------------------------------------------------
    // Modular — the ring ℤ/nℤ
    // -----------------------------------------------------------------

    fn m(v: i64, n: i64) -> Modular {
        Modular::from_i64(v, n).unwrap_or_else(|| panic!("bad modulus {n}"))
    }

    #[test]
    fn modular_reduces_into_range_on_construction() {
        assert_eq!(m(10, 7).value().to_i64(), Some(3));
        assert_eq!(m(7, 7).value().to_i64(), Some(0));
        // Negatives become non-negative residues (Euclidean): −1 mod 7 = 6.
        assert_eq!(m(-1, 7).value().to_i64(), Some(6));
        assert_eq!(m(-10, 7).value().to_i64(), Some(4));
        // Modulus 1 collapses everything to 0 (the trivial ring).
        assert_eq!(m(5, 1).value().to_i64(), Some(0));
        // A non-positive modulus is rejected.
        assert!(Modular::from_i64(3, 0).is_none());
        assert!(Modular::from_i64(3, -7).is_none());
    }

    #[test]
    fn modular_arithmetic_wraps_in_the_ring() {
        // Add/sub/mul stay in [0, n) — overflow IS the point (cyclic group).
        assert_eq!(m(5, 7).add(&m(4, 7)).unwrap(), m(2, 7)); // 9 ≡ 2
        assert_eq!(m(3, 7).sub(&m(5, 7)).unwrap(), m(5, 7)); // −2 ≡ 5
        assert_eq!(m(4, 7).mul(&m(5, 7)).unwrap(), m(6, 7)); // 20 ≡ 6
        // −x is the additive inverse.
        assert_eq!(m(3, 7).negated(), m(4, 7));
        assert!(m(3, 7).add(&m(3, 7).negated()).unwrap().is_zero());
        // A modulus mismatch is refused (a ring mismatch, like a Word width mismatch).
        assert!(m(3, 7).add(&m(3, 5)).is_none());
        assert!(m(3, 7).mul(&m(3, 5)).is_none());
    }

    #[test]
    fn modular_exponentiation_is_fast_and_exact() {
        assert_eq!(m(3, 5).pow(4), m(1, 5)); // 81 ≡ 1 (mod 5)
        assert_eq!(m(2, 7).pow(3), m(1, 7)); // 8 ≡ 1 (mod 7)
        assert_eq!(m(5, 13).pow(0), m(1, 13)); // x^0 = 1
        // A large exponent that fast-exp makes tractable: 7^100 (mod 13).
        let mut acc = m(1, 13);
        for _ in 0..100 {
            acc = acc.mul(&m(7, 13)).unwrap();
        }
        assert_eq!(m(7, 13).pow(100), acc);
    }

    #[test]
    fn modular_inverse_and_division() {
        // 3⁻¹ ≡ 5 (mod 7) since 3·5 = 15 ≡ 1.
        assert_eq!(m(3, 7).inverse().unwrap(), m(5, 7));
        assert_eq!(m(3, 7).mul(&m(3, 7).inverse().unwrap()).unwrap(), m(1, 7));
        // Division is multiplication by the inverse: 1/3 ≡ 5 (mod 7).
        assert_eq!(m(1, 7).div(&m(3, 7)).unwrap(), m(5, 7));
        assert_eq!(m(4, 7).div(&m(2, 7)).unwrap(), m(2, 7));
        // A non-unit (gcd ≠ 1) has NO inverse: 2 is not invertible mod 4.
        assert!(m(2, 4).inverse().is_none());
        assert!(m(1, 4).div(&m(2, 4)).is_none());
        // Division across different moduli is refused.
        assert!(m(1, 7).div(&m(3, 5)).is_none());
    }

    #[test]
    fn modular_generalizes_the_word_wrapping_ring() {
        // ℤ/2³²ℤ IS the Word32 ring: add/mul agree with the fixed-width wrapping ops.
        let modulus = 1i64 << 32; // 2^32 = 4_294_967_296
        let (a, b) = (4_000_000_000u32, 1_000_000_000u32); // a + b overflows u32
        let word_add = (crate::Word32(a) + crate::Word32(b)).0 as i64;
        let mod_add = m(a as i64, modulus).add(&m(b as i64, modulus)).unwrap();
        assert_eq!(mod_add.value().to_i64(), Some(word_add), "ℤ/2³² add == Word32 add");
        let word_mul = (crate::Word32(a) * crate::Word32(b)).0 as i64;
        let mod_mul = m(a as i64, modulus).mul(&m(b as i64, modulus)).unwrap();
        assert_eq!(mod_mul.value().to_i64(), Some(word_mul), "ℤ/2³² mul == Word32 mul");
    }

    #[test]
    fn modular_equality_is_per_ring_and_displays_the_modulus() {
        // Same residue in DIFFERENT rings is not equal (the modulus is part of the value).
        assert_ne!(m(3, 7), m(3, 5));
        assert_eq!(m(10, 7), m(3, 7));
        assert_eq!(m(3, 7).to_string(), "3 (mod 7)");
        assert_eq!(m(-1, 7).to_string(), "6 (mod 7)");
    }

    #[test]
    fn modular_obeys_fermats_little_theorem() {
        // For a prime p and a not divisible by p: a^(p−1) ≡ 1 (mod p).
        for &p in &[2i64, 3, 5, 7, 13, 97, 101] {
            for a in 1..p.min(40) {
                assert_eq!(m(a, p).pow((p - 1) as u64), m(1, p), "{a}^({p}-1) ≡ 1 (mod {p})");
            }
        }
    }

    #[test]
    fn modular_obeys_the_ring_laws_and_inverse_under_fuzz() {
        // Property fuzz: random residues over random moduli satisfy the commutative-ring
        // axioms exactly, and every unit times its inverse is 1.
        let mut rng = Rng(0x_0DDF_ACE5_ABCD_1234);
        for _ in 0..3000 {
            let n = ((rng.next() % 50) as i64) + 2; // modulus in [2, 51]
            let pick = |rng: &mut Rng| (rng.next() % 1_000_000) as i64 - 500_000;
            let (a, b, c) = (m(pick(&mut rng), n), m(pick(&mut rng), n), m(pick(&mut rng), n));
            // Commutativity.
            assert_eq!(a.add(&b).unwrap(), b.add(&a).unwrap());
            assert_eq!(a.mul(&b).unwrap(), b.mul(&a).unwrap());
            // Associativity.
            assert_eq!(a.add(&b).unwrap().add(&c).unwrap(), a.add(&b.add(&c).unwrap()).unwrap());
            assert_eq!(a.mul(&b).unwrap().mul(&c).unwrap(), a.mul(&b.mul(&c).unwrap()).unwrap());
            // Distributivity.
            assert_eq!(
                a.mul(&b.add(&c).unwrap()).unwrap(),
                a.mul(&b).unwrap().add(&a.mul(&c).unwrap()).unwrap()
            );
            // Additive inverse.
            assert!(a.add(&a.negated()).unwrap().is_zero());
            // Multiplicative inverse for units (gcd(a, n) == 1).
            if let Some(inv) = a.inverse() {
                assert_eq!(a.mul(&inv).unwrap(), m(1, n), "a·a⁻¹ = 1 in ℤ/{n}ℤ");
            }
        }
    }

    // ─── Exact float interop ────────────────────────────────────────────

    #[test]
    fn exact_rational_from_f64_values() {
        assert_eq!(rational_from_f64_exact(0.5).unwrap(), Rational::from_ratio_i64(1, 2).unwrap());
        assert_eq!(rational_from_f64_exact(3.0).unwrap(), Rational::from_i64(3));
        assert_eq!(rational_from_f64_exact(-2.25).unwrap(), Rational::from_ratio_i64(-9, 4).unwrap());
        assert_eq!(rational_from_f64_exact(0.0).unwrap(), Rational::zero());
        assert_eq!(rational_from_f64_exact(-0.0).unwrap(), Rational::zero());
        assert!(rational_from_f64_exact(f64::NAN).is_none());
        assert!(rational_from_f64_exact(f64::INFINITY).is_none());
        // 0.1 is NOT 1/10 — it is the nearest double, an exact dyadic rational.
        assert_ne!(rational_from_f64_exact(0.1).unwrap(), Rational::from_ratio_i64(1, 10).unwrap());
    }

    #[test]
    fn exact_i64_f64_cmp_at_the_representability_boundary() {
        let big = 9_007_199_254_740_993_i64; // 2^53 + 1 — not representable
        let f = 9_007_199_254_740_992.0_f64; // 2^53 — what the literal rounds to
        // A lossy `as f64` compare would say Equal; the exact one says Greater.
        assert_eq!(cmp_i64_f64_exact(big, f), Some(Ordering::Greater));
        assert_eq!(cmp_i64_f64_exact(big - 1, f), Some(Ordering::Equal));
        assert_eq!(cmp_i64_f64_exact(3, 3.5), Some(Ordering::Less));
        assert_eq!(cmp_i64_f64_exact(4, 3.5), Some(Ordering::Greater));
        assert_eq!(cmp_i64_f64_exact(1, 1.0), Some(Ordering::Equal));
        assert_eq!(cmp_i64_f64_exact(0, f64::NAN), None);
        assert_eq!(cmp_i64_f64_exact(i64::MAX, f64::INFINITY), Some(Ordering::Less));
        assert_eq!(cmp_i64_f64_exact(i64::MIN, f64::NEG_INFINITY), Some(Ordering::Greater));
    }

    #[test]
    fn unified_numeric_hash_coherence() {
        // hash(Int k) == hash(Float k) for every exactly-representable k.
        for k in [0i64, 1, -1, 2, 42, -42, 1_000_000_007, -(1i64 << 53), 1i64 << 53] {
            assert_eq!(
                numeric_hash_i64(k),
                numeric_hash_f64(k as f64),
                "int/float hash mismatch at {k}"
            );
            assert_eq!(
                numeric_hash_i64(k),
                numeric_hash_bigint(&BigInt::from_i64(k)),
                "int/bigint hash mismatch at {k}"
            );
            assert_eq!(
                numeric_hash_i64(k),
                numeric_hash_rational(&Rational::from_i64(k)),
                "int/rational hash mismatch at {k}"
            );
        }
        // Non-integral coherence: 0.5 == 1/2 exactly, so hashes agree.
        assert_eq!(
            numeric_hash_f64(0.5),
            numeric_hash_rational(&Rational::from_ratio_i64(1, 2).unwrap())
        );
        assert_eq!(
            numeric_hash_f64(-2.25),
            numeric_hash_rational(&Rational::from_ratio_i64(-9, 4).unwrap())
        );
        // -0.0 and 0.0 are equal, so they hash equal.
        assert_eq!(numeric_hash_f64(-0.0), numeric_hash_f64(0.0));
        // And a float hashes as its EXACT value: 0.1's hash equals the hash
        // of its exact dyadic rational, not of 1/10.
        assert_eq!(
            numeric_hash_f64(0.1),
            numeric_hash_rational(&rational_from_f64_exact(0.1).unwrap())
        );
    }

    // ---- the one-limb boundary (2^64): every op must cross it exactly ----

    #[test]
    fn arithmetic_is_exact_across_the_one_limb_boundary() {
        let u64max = BigInt::from_u64(u64::MAX);
        let two64 = u64max.add(&b(1)); // 2^64
        assert_eq!(two64.to_string(), "18446744073709551616");
        // Back below the boundary.
        assert_eq!(two64.sub(&b(1)), u64max);
        // Multiplication crossing: (2^32)² = 2^64; 2^62·4 = 2^64.
        let two32 = b(1i64 << 32);
        assert_eq!(two32.mul(&two32), two64);
        assert_eq!(b(1i64 << 62).mul(&b(4)), two64);
        // Division re-crossing: 2^64 / 2 = 2^63 (one limb again).
        let (q, r) = two64.div_rem(&b(2)).expect("nonzero divisor");
        assert!(r.is_zero());
        assert_eq!(q.to_string(), "9223372036854775808");
        // Negative mirror.
        let neg = two64.negated();
        assert_eq!(neg.add(&two64), BigInt::zero());
        assert_eq!(neg.to_string(), "-18446744073709551616");
    }

    #[test]
    fn ordering_and_hash_are_canonical_across_the_limb_boundary() {
        let two64 = BigInt::from_u64(u64::MAX).add(&b(1));
        let big = two64.mul(&two64); // 2^128
        assert!(b(1) < two64);
        assert!(two64 < big);
        assert!(big.negated() < b(-1));
        assert!(two64.negated() < two64);
        // Equal values reached by different construction routes must be equal
        // AND hash equal — the representation is canonical, not path-dependent.
        let via_mul = b(1i64 << 32).mul(&b(1i64 << 32));
        let via_add = BigInt::from_u64(u64::MAX).add(&b(1));
        assert_eq!(via_mul, via_add);
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let h = |v: &BigInt| {
            let mut s = DefaultHasher::new();
            v.hash(&mut s);
            s.finish()
        };
        assert_eq!(h(&via_mul), h(&via_add));
        // A value shrunk back under the boundary equals its native construction.
        let shrunk = two64.sub(&b(1));
        assert_eq!(shrunk, BigInt::from_u64(u64::MAX));
        assert_eq!(h(&shrunk), h(&BigInt::from_u64(u64::MAX)));
    }

    #[test]
    fn le_bytes_round_trip_across_the_limb_boundary() {
        for s in [
            "0",
            "1",
            "-1",
            "18446744073709551615",
            "18446744073709551616",
            "-18446744073709551616",
            "340282366920938463463374607431768211456",
        ] {
            let v = BigInt::parse_decimal(s).expect("valid decimal");
            let (neg, bytes) = v.to_le_bytes();
            assert_eq!(BigInt::from_le_bytes(neg, &bytes), v, "round trip {s}");
            assert_eq!(v.to_string(), s, "display {s}");
        }
    }

    #[test]
    fn div_rem_identity_holds_for_multi_limb_values() {
        // a = q·b + r with |r| < |b| and r carrying the dividend's sign, at
        // sizes where single-limb arithmetic cannot apply.
        let a = b(1_000_003).pow(7); // ~2^140
        for dividend in [a.clone(), a.negated()] {
            for divisor in [b(97), b(-97), b(1i64 << 40), a.sub(&b(1))] {
                let (q, r) = dividend.div_rem(&divisor).expect("nonzero divisor");
                assert_eq!(dividend, q.mul(&divisor).add(&r));
                assert!(r.abs() < divisor.abs());
                assert!(r.is_zero() || r.is_negative() == dividend.is_negative());
            }
        }
    }
}
