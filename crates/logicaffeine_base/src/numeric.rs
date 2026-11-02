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

/// The magnitude length (in 64-bit limbs) at/above which Karatsuba beats schoolbook:
/// its three half-size products plus the add/sub glue only pay off once the smaller
/// operand is large enough. Below it, schoolbook's tight inner loop wins.
const KARATSUBA_THRESHOLD: usize = 32;

/// `a * b` on magnitudes: schoolbook for small operands, Karatsuba for large.
fn mag_mul(a: &[u64], b: &[u64]) -> Vec<u64> {
    if a.is_empty() || b.is_empty() {
        return Vec::new();
    }
    if a.len().min(b.len()) < KARATSUBA_THRESHOLD {
        return mag_mul_schoolbook(a, b);
    }
    mag_mul_karatsuba(a, b)
}

/// Schoolbook `a * b` — O(n·m). The exact correctness oracle Karatsuba is fuzzed against.
fn mag_mul_schoolbook(a: &[u64], b: &[u64]) -> Vec<u64> {
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

/// Prepend `n` zero limbs — multiply by `2^(64·n)`, i.e. shift the magnitude up by `n` limbs.
fn mag_shl_limbs(a: &[u64], n: usize) -> Vec<u64> {
    if a.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(a.len() + n);
    out.resize(n, 0);
    out.extend_from_slice(a);
    out
}

/// Karatsuba `a * b`: split each operand at `m` limbs (`x = x_hi·B^m + x_lo`, `B = 2^64`)
/// and combine three half-size products —
///   `z0 = a_lo·b_lo`, `z2 = a_hi·b_hi`, `z1 = (a_lo+a_hi)·(b_lo+b_hi) − z0 − z2` —
/// into `a·b = z2·B^{2m} + z1·B^m + z0`. `z1` is the (non-negative) sum of the cross
/// products, so the subtractions never underflow. Recurses through `mag_mul`, so every
/// sub-product below `KARATSUBA_THRESHOLD` falls back to schoolbook.
fn mag_mul_karatsuba(a: &[u64], b: &[u64]) -> Vec<u64> {
    let m = a.len().min(b.len()) / 2; // ≥ KARATSUBA_THRESHOLD/2 ≥ 1, and ≤ both lengths
    let (a_lo, a_hi) = a.split_at(m);
    let (b_lo, b_hi) = b.split_at(m);

    let z0 = mag_mul(a_lo, b_lo);
    let z2 = mag_mul(a_hi, b_hi);
    let a_sum = mag_add(a_lo, a_hi);
    let b_sum = mag_add(b_lo, b_hi);
    let z1 = mag_sub(&mag_sub(&mag_mul(&a_sum, &b_sum), &z0), &z2);

    let mut out = z0;
    out = mag_add(&out, &mag_shl_limbs(&z1, m));
    out = mag_add(&out, &mag_shl_limbs(&z2, 2 * m));
    normalize(out)
}

/// `r << 1 | bit` on a magnitude (shift the whole number up by one bit).
#[cfg(test)]
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

/// Bit-at-a-time long division — the exact oracle Algorithm D is fuzzed against.
#[cfg(test)]
fn mag_divrem_bitwise(a: &[u64], b: &[u64]) -> (Vec<u64>, Vec<u64>) {
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

/// `a << bits` on a magnitude.
fn mag_shl_bits(a: &[u64], bits: u64) -> Vec<u64> {
    if a.is_empty() {
        return Vec::new();
    }
    let (words, sh) = ((bits / 64) as usize, (bits % 64) as u32);
    let mut out = Vec::with_capacity(a.len() + words + 1);
    out.resize(words, 0);
    if sh == 0 {
        out.extend_from_slice(a);
    } else {
        let mut carry = 0u64;
        for &limb in a {
            out.push((limb << sh) | carry);
            carry = limb >> (64 - sh);
        }
        if carry != 0 {
            out.push(carry);
        }
    }
    out
}

/// `x · 2^k` for a finite `x`: exact while the result is finite, infinite once it is not. Multiplying
/// by a power of two changes only the exponent, so there is nothing to round until the exponent
/// overflows; the scale is applied in steps of 2^1000 because a single `f64` holds at most 2^1023.
fn scale_by_power_of_two(mut x: f64, mut k: u64) -> f64 {
    const STEP: u64 = 1000;
    let power = |e: u64| f64::from_bits((1023 + e) << 52);
    while k > STEP && x.is_finite() {
        x *= power(STEP);
        k -= STEP;
    }
    if x.is_finite() {
        x * power(k)
    } else {
        x
    }
}

/// `a >> bits` on a magnitude (floor).
fn mag_shr_bits(a: &[u64], bits: u64) -> Vec<u64> {
    let words = bits / 64;
    if words >= a.len() as u64 {
        return Vec::new();
    }
    let a = &a[words as usize..];
    let sh = (bits % 64) as u32;
    if sh == 0 {
        return a.to_vec();
    }
    let mut out = Vec::with_capacity(a.len());
    for i in 0..a.len() {
        let hi = a.get(i + 1).map_or(0, |h| h << (64 - sh));
        out.push((a[i] >> sh) | hi);
    }
    normalize(out)
}

/// Long division on magnitudes: returns `(quotient, remainder)` with `a = q*b + r` and
/// `0 <= r < b`, by Knuth's Algorithm D (TAOCP vol. 2, §4.3.1): normalise so the divisor's top
/// limb has its high bit set, estimate each quotient limb from the top two limbs of the running
/// remainder, correct the estimate at most twice, and add back on the rare overshoot.
fn mag_divrem(a: &[u64], b: &[u64]) -> (Vec<u64>, Vec<u64>) {
    debug_assert!(!b.is_empty(), "division by zero magnitude");
    if mag_cmp(a, b) == Ordering::Less {
        return (Vec::new(), a.to_vec());
    }
    if b.len() == 1 {
        let d = b[0] as u128;
        let mut q = vec![0u64; a.len()];
        let mut rem: u128 = 0;
        for i in (0..a.len()).rev() {
            let cur = (rem << 64) | a[i] as u128;
            q[i] = (cur / d) as u64;
            rem = cur % d;
        }
        return (normalize(q), normalize(vec![rem as u64]));
    }
    let shift = b[b.len() - 1].leading_zeros() as u64;
    let v = mag_shl_bits(b, shift);
    let mut u = mag_shl_bits(a, shift);
    u.resize(a.len() + 1, 0);
    let n = v.len();
    let m = u.len() - n - 1;
    let mut q = vec![0u64; m + 1];
    let (vtop, vnext) = (v[n - 1] as u128, v[n - 2] as u128);
    const B: u128 = 1 << 64;
    for j in (0..=m).rev() {
        let num = ((u[j + n] as u128) << 64) | u[j + n - 1] as u128;
        let mut qhat = num / vtop;
        let mut rhat = num % vtop;
        while qhat >= B || qhat * vnext > ((rhat << 64) | u[j + n - 2] as u128) {
            qhat -= 1;
            rhat += vtop;
            if rhat >= B {
                break;
            }
        }
        let mut borrow: i128 = 0;
        let mut carry: u128 = 0;
        for i in 0..n {
            let p = qhat * v[i] as u128 + carry;
            carry = p >> 64;
            let t = u[i + j] as i128 - (p as u64) as i128 + borrow;
            u[i + j] = t as u64;
            borrow = t >> 64;
        }
        let t = u[j + n] as i128 - carry as i128 + borrow;
        u[j + n] = t as u64;
        if t < 0 {
            qhat -= 1;
            let mut c: u128 = 0;
            for i in 0..n {
                let s = u[i + j] as u128 + v[i] as u128 + c;
                u[i + j] = s as u64;
                c = s >> 64;
            }
            u[j + n] = u[j + n].wrapping_add(c as u64);
        }
        q[j] = qhat as u64;
    }
    u.truncate(n);
    (normalize(q), mag_shr_bits(&normalize(u), shift))
}

impl BigInt {
    /// The additive identity.
    pub fn zero() -> Self {
        BigInt {
            sign: Sign::Zero,
            mag: Mag::Inline(0),
        }
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
        let mag = if hi == 0 {
            Mag::Inline(lo)
        } else {
            Mag::Heap(vec![lo, hi])
        };
        BigInt {
            sign: if neg { Sign::Neg } else { Sign::Pos },
            mag,
        }
    }

    /// Exact widening from a machine integer.
    pub fn from_i64(x: i64) -> Self {
        Self::from_sign_mag_u128(x < 0, (x as i128).unsigned_abs())
    }

    /// Exact widening from an unsigned machine integer.
    pub fn from_u64(x: u64) -> Self {
        Self::from_sign_mag_u128(false, x as u128)
    }

    /// Heap payload bytes duplicated when this integer is cloned.
    ///
    /// Inline fields and the vector header belong to the containing value.
    /// This borrowed query neither copies limbs nor formats the integer.
    pub fn clone_payload_bytes(&self) -> usize {
        match &self.mag {
            Mag::Inline(_) => 0,
            Mag::Heap(limbs) => limbs.len() * std::mem::size_of::<u64>(),
        }
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

    /// The value as a `u64` when it is non-negative and fits.
    pub fn to_u64(&self) -> Option<u64> {
        match self.sign {
            Sign::Neg => None,
            _ => self.mag.as_single(),
        }
    }

    /// The low 64 bits, defined for every integer.
    pub fn wrapping_u64(&self) -> u64 {
        let low = self.mag.limbs().first().copied().unwrap_or(0);
        if self.sign == Sign::Neg {
            low.wrapping_neg()
        } else {
            low
        }
    }

    /// Number of significant bits of the magnitude (`0` for zero).
    pub fn bit_len(&self) -> u64 {
        let l = self.mag.limbs();
        match l.last() {
            None => 0,
            Some(top) => (l.len() as u64 - 1) * 64 + (64 - top.leading_zeros() as u64),
        }
    }

    /// `gcd(|self|, |other|)`, with `gcd(a, 0) = |a|`.
    pub fn gcd(&self, other: &Self) -> Self {
        if let (Some(a), Some(b)) = (self.mag.as_single(), other.mag.as_single()) {
            let (mut a, mut b) = (a, b);
            while b != 0 {
                let t = a % b;
                a = b;
                b = t;
            }
            return BigInt::from_u64(a);
        }
        bigint_gcd(self, other)
    }

    /// `self · 2^bits` (exact for either sign).
    pub fn shl_bits(&self, bits: u64) -> Self {
        Self::from_sign_mag(self.sign == Sign::Neg, mag_shl_bits(self.mag.limbs(), bits))
    }

    /// `⌊self / 2^bits⌋` for a non-negative value; `None` for a negative one.
    pub fn shr_bits(&self, bits: u64) -> Option<Self> {
        if self.sign == Sign::Neg {
            return None;
        }
        Some(Self::from_sign_mag(
            false,
            mag_shr_bits(self.mag.limbs(), bits),
        ))
    }

    fn natural_bitwise(&self, other: &Self, op: impl Fn(u64, u64) -> u64) -> Option<Self> {
        if self.sign == Sign::Neg || other.sign == Sign::Neg {
            return None;
        }
        let (a, b) = (self.mag.limbs(), other.mag.limbs());
        let out = (0..a.len().max(b.len()))
            .map(|i| op(*a.get(i).unwrap_or(&0), *b.get(i).unwrap_or(&0)))
            .collect();
        Some(Self::from_sign_mag(false, out))
    }

    /// Bitwise AND of two non-negative values; `None` if either is negative.
    pub fn bitand_natural(&self, other: &Self) -> Option<Self> {
        self.natural_bitwise(other, |x, y| x & y)
    }

    /// Bitwise OR of two non-negative values; `None` if either is negative.
    pub fn bitor_natural(&self, other: &Self) -> Option<Self> {
        self.natural_bitwise(other, |x, y| x | y)
    }

    /// Bitwise XOR of two non-negative values; `None` if either is negative.
    pub fn bitxor_natural(&self, other: &Self) -> Option<Self> {
        self.natural_bitwise(other, |x, y| x ^ y)
    }

    pub fn is_zero(&self) -> bool {
        self.sign == Sign::Zero
    }

    /// Parity (the low bit of the magnitude — sign does not change it). Zero is even.
    pub fn is_odd(&self) -> bool {
        self.mag.limbs().first().is_some_and(|limb| limb & 1 == 1)
    }

    /// The binary64 nearest to this integer, ties to even, ±∞ once the rounding reaches 2^1024: the
    /// one conversion every tier uses for an Int flowing into a Real (a widened argument, `n + 0.5`).
    ///
    /// One rounding, never one per limb: the top 64 significant bits are taken exactly, every bit
    /// below them is folded into bit 0 as a sticky bit (eleven places under the 53-bit rounding
    /// position, so it can break a tie but never create one), and that `u64` is converted with the
    /// hardware's round-to-nearest-even. Scaling back by a power of two is exact until it overflows,
    /// which is exactly where the exact value rounds to infinity too.
    pub fn to_f64(&self) -> f64 {
        let limbs = self.mag.limbs();
        let bits = self.bit_len();
        let magnitude = if bits <= 64 {
            limbs.first().copied().unwrap_or(0) as f64
        } else {
            let dropped = bits - 64;
            let (word, shift) = ((dropped / 64) as usize, (dropped % 64) as u32);
            let top = if shift == 0 {
                limbs[word]
            } else {
                (limbs[word] >> shift) | (limbs[word + 1] << (64 - shift))
            };
            let sticky = limbs[..word].iter().any(|&l| l != 0)
                || (shift != 0 && limbs[word] << (64 - shift) != 0);
            scale_by_power_of_two((top | sticky as u64) as f64, dropped)
        };
        if self.sign == Sign::Neg {
            -magnitude
        } else {
            magnitude
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
        BigInt {
            sign: if self.sign == Sign::Zero {
                Sign::Zero
            } else {
                Sign::Pos
            },
            mag: self.mag.clone(),
        }
    }

    /// Additive inverse.
    pub fn negated(&self) -> Self {
        let sign = match self.sign {
            Sign::Neg => Sign::Pos,
            Sign::Zero => Sign::Zero,
            Sign::Pos => Sign::Neg,
        };
        BigInt {
            sign,
            mag: self.mag.clone(),
        }
    }

    /// `self + other`.
    pub fn add(&self, other: &Self) -> Self {
        // Single-limb fast path: both magnitudes fit a limb, so the signed sum
        // fits an i128 — no limb vectors, no allocation.
        if let (Some(a), Some(b)) = (self.mag.as_single(), other.mag.as_single()) {
            let av = if self.sign == Sign::Neg {
                -(a as i128)
            } else {
                a as i128
            };
            let bv = if other.sign == Sign::Neg {
                -(b as i128)
            } else {
                b as i128
            };
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

    /// In-place multiply by a machine-integer scalar, REUSING the heap magnitude buffer
    /// (it grows by at most one limb, never reallocates from scratch). The exact-value
    /// counterpart of `self.mul(&BigInt::from_i64(rhs))` — same result, but the growing
    /// accumulator's `Vec` is mutated rather than reallocated each call, which is the hot
    /// loop `Set p to p * i` (factorial). Canonical form is preserved: multiplying a
    /// normalized magnitude by a nonzero scalar never introduces a trailing zero limb (a
    /// zeroed low half always carries out), and a `Heap` value stays ≥ 2 limbs.
    pub fn mul_assign_i64(&mut self, rhs: i64) {
        if rhs == 0 || self.is_zero() {
            *self = Self::zero();
            return;
        }
        if rhs < 0 {
            self.sign = match self.sign {
                Sign::Neg => Sign::Pos,
                Sign::Pos => Sign::Neg,
                Sign::Zero => Sign::Zero,
            };
        }
        let m = (rhs as i128).unsigned_abs() as u64;
        match &mut self.mag {
            Mag::Inline(x) => {
                let prod = (*x as u128) * (m as u128);
                self.mag = if (prod >> 64) == 0 {
                    Mag::Inline(prod as u64)
                } else {
                    Mag::Heap(vec![prod as u64, (prod >> 64) as u64])
                };
            }
            Mag::Heap(v) => {
                let mut carry: u64 = 0;
                for limb in v.iter_mut() {
                    let prod = (*limb as u128) * (m as u128) + (carry as u128);
                    *limb = prod as u64;
                    carry = (prod >> 64) as u64;
                }
                if carry != 0 {
                    v.push(carry);
                }
            }
        }
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
            return Some(Rational {
                num: BigInt::zero(),
                den: BigInt::from_i64(1),
            });
        }
        let g = bigint_gcd(&num, &den);
        let (num, _) = num.div_rem(&g).expect("gcd divides the numerator exactly");
        let (den, _) = den
            .div_rem(&g)
            .expect("gcd divides the denominator exactly");
        Some(Rational { num, den })
    }

    /// The integer `n` as the fraction `n/1`.
    pub fn from_bigint(n: BigInt) -> Rational {
        Rational {
            num: n,
            den: BigInt::from_i64(1),
        }
    }

    pub fn from_i64(x: i64) -> Rational {
        Rational::from_bigint(BigInt::from_i64(x))
    }

    /// `n/d` from machine integers (convenience; `None` if `d == 0`).
    pub fn from_ratio_i64(n: i64, d: i64) -> Option<Rational> {
        Rational::new(BigInt::from_i64(n), BigInt::from_i64(d))
    }

    pub fn zero() -> Rational {
        Rational {
            num: BigInt::zero(),
            den: BigInt::from_i64(1),
        }
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
        let (mag, _) = numerator
            .div_rem(&denominator)
            .expect("denominator is nonzero");
        if self.num.is_negative() {
            mag.negated()
        } else {
            mag
        }
    }

    pub fn negated(&self) -> Rational {
        Rational {
            num: self.num.negated(),
            den: self.den.clone(),
        }
    }

    pub fn abs(&self) -> Rational {
        Rational {
            num: self.num.abs(),
            den: self.den.clone(),
        }
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
        RoundingMode::HalfEven => {
            half == Ordering::Greater || (half == Ordering::Equal && q.is_odd())
        }
    };
    if away {
        // `q` is truncated toward zero, so rounding away adds the value's sign.
        if neg {
            q.sub(&BigInt::from_i64(1))
        } else {
            q.add(&BigInt::from_i64(1))
        }
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
        Decimal {
            coeff: self.coeff.negated(),
            scale: self.scale,
        }
    }

    pub fn abs(&self) -> Decimal {
        Decimal {
            coeff: self.coeff.abs(),
            scale: self.scale,
        }
    }

    /// Exact view as a [`Rational`] (`coeff / 10^scale`) — lossless, the bridge that
    /// keeps `Decimal` inside the exact tower.
    pub fn to_rational(&self) -> Rational {
        Rational::new(self.coeff.clone(), ten_pow(self.scale)).expect("10^scale is nonzero")
    }

    /// The fixed-point value nearest `value` at the given `scale`, rounded under `mode`.
    pub fn from_rational(value: &Rational, scale: u32, mode: RoundingMode) -> Decimal {
        let scaled = value.mul(&Rational::from_bigint(ten_pow(scale)));
        Decimal {
            coeff: round_rational_to_bigint(&scaled, mode),
            scale,
        }
    }

    /// Restate at a different scale. Widening appends zeros (exact); narrowing drops
    /// digits and rounds under `mode`.
    pub fn rescale(&self, scale: u32, mode: RoundingMode) -> Decimal {
        if scale >= self.scale {
            Decimal {
                coeff: self.coeff.mul(&ten_pow(scale - self.scale)),
                scale,
            }
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
        Decimal {
            coeff: a.add(&b),
            scale: s,
        }
    }

    pub fn sub(&self, other: &Decimal) -> Decimal {
        let (a, b, s) = self.aligned(other);
        Decimal {
            coeff: a.sub(&b),
            scale: s,
        }
    }

    /// Exact product: coefficients multiply, scales add.
    pub fn mul(&self, other: &Decimal) -> Decimal {
        Decimal {
            coeff: self.coeff.mul(&other.coeff),
            scale: self.scale + other.scale,
        }
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
        Decimal {
            coeff: BigInt::from_le_bytes(negative, bytes),
            scale,
        }
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
        if !int_part.bytes().all(|c| c.is_ascii_digit())
            || !frac_part.bytes().all(|c| c.is_ascii_digit())
        {
            return None;
        }
        let digits = format!("{int_part}{frac_part}");
        let mag = if digits.is_empty() {
            BigInt::zero()
        } else {
            BigInt::parse_decimal(&digits)?
        };
        let coeff = if neg { mag.negated() } else { mag };
        Some(Decimal {
            coeff,
            scale: frac_part.len() as u32,
        })
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
        Complex {
            re,
            im: Rational::zero(),
        }
    }

    pub fn from_i64(x: i64) -> Complex {
        Complex::from_rational(Rational::from_i64(x))
    }

    pub fn zero() -> Complex {
        Complex {
            re: Rational::zero(),
            im: Rational::zero(),
        }
    }

    pub fn one() -> Complex {
        Complex::from_i64(1)
    }

    /// The imaginary unit `i` (`0 + 1i`).
    pub fn i() -> Complex {
        Complex {
            re: Rational::zero(),
            im: Rational::one(),
        }
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
        Complex {
            re: self.re.clone(),
            im: self.im.negated(),
        }
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
        Complex {
            re: self.re.negated(),
            im: self.im.negated(),
        }
    }

    pub fn add(&self, other: &Complex) -> Complex {
        Complex {
            re: self.re.add(&other.re),
            im: self.im.add(&other.im),
        }
    }

    pub fn sub(&self, other: &Complex) -> Complex {
        Complex {
            re: self.re.sub(&other.re),
            im: self.im.sub(&other.im),
        }
    }

    /// `(a+bi)(c+di) = (ac − bd) + (ad + bc)i`.
    pub fn mul(&self, other: &Complex) -> Complex {
        let (a, b, c, d) = (&self.re, &self.im, &other.re, &other.im);
        Complex {
            re: a.mul(c).sub(&b.mul(d)),
            im: a.mul(d).add(&b.mul(c)),
        }
    }

    /// `1/self` via the conjugate over the squared magnitude — `None` only for zero.
    pub fn recip(&self) -> Option<Complex> {
        let n = self.norm_sqr();
        if n.is_zero() {
            return None;
        }
        let inv = n.recip().expect("norm is nonzero here");
        Some(Complex {
            re: self.re.mul(&inv),
            im: self.im.negated().mul(&inv),
        })
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
        let re = if real_str.is_empty() {
            Rational::zero()
        } else {
            Rational::parse(real_str)?
        };
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
                if unit {
                    write!(f, "-i")
                } else {
                    write!(f, "-{mag}i")
                }
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

// =====================================================================
// ComplexF — floating-point complex (LAPACK's `Z`), the NUMERICAL dual
// of the exact `Complex`. Its parts are `f64`, so — unlike the exact
// Gaussian rationals — it has a `sqrt` (`√(a+bi)` from the real `sqrt`),
// which is what the sqrt-based complex decompositions (Cholesky, QR, eig,
// SVD) need. Inexact by nature (it rounds); use the exact `Complex` when
// you want zero error. Boxed as `Rc<ComplexF>` in `RuntimeValue` so the
// enum stays 16 bytes (the nanbox budget), exactly like `Complex`.
// =====================================================================

/// A floating-point complex number `re + im·i`, each part an `f64`. NOT ordered
/// and NOT `Eq`/`Hash` (it carries `f64`s); equality is exact bit-for-value.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ComplexF {
    re: f64,
    im: f64,
}

impl ComplexF {
    pub fn new(re: f64, im: f64) -> ComplexF {
        ComplexF { re, im }
    }
    /// A real number as `re + 0i`.
    pub fn from_f64(re: f64) -> ComplexF {
        ComplexF { re, im: 0.0 }
    }
    pub fn re(&self) -> f64 {
        self.re
    }
    pub fn im(&self) -> f64 {
        self.im
    }
    pub fn is_zero(&self) -> bool {
        self.re == 0.0 && self.im == 0.0
    }
    /// True when the imaginary part is zero (a plain real).
    pub fn is_real(&self) -> bool {
        self.im == 0.0
    }
    /// `re − im·i`.
    pub fn conjugate(&self) -> ComplexF {
        ComplexF {
            re: self.re,
            im: -self.im,
        }
    }
    /// `re² + im²` — the squared magnitude.
    pub fn norm_sqr(&self) -> f64 {
        self.re * self.re + self.im * self.im
    }
    /// The magnitude `√(re²+im²)`.
    pub fn abs(&self) -> f64 {
        self.norm_sqr().sqrt()
    }
    pub fn negated(&self) -> ComplexF {
        ComplexF {
            re: -self.re,
            im: -self.im,
        }
    }
    pub fn add(&self, other: &ComplexF) -> ComplexF {
        ComplexF {
            re: self.re + other.re,
            im: self.im + other.im,
        }
    }
    pub fn sub(&self, other: &ComplexF) -> ComplexF {
        ComplexF {
            re: self.re - other.re,
            im: self.im - other.im,
        }
    }
    /// `(a+bi)(c+di) = (ac − bd) + (ad + bc)i`.
    pub fn mul(&self, other: &ComplexF) -> ComplexF {
        let (a, b, c, d) = (self.re, self.im, other.re, other.im);
        ComplexF {
            re: a * c - b * d,
            im: a * d + b * c,
        }
    }
    /// `1/self` via the conjugate over the squared magnitude — `None` only for zero.
    pub fn recip(&self) -> Option<ComplexF> {
        let n = self.norm_sqr();
        if n == 0.0 {
            return None;
        }
        Some(ComplexF {
            re: self.re / n,
            im: -self.im / n,
        })
    }
    /// `self / other` — `None` when `other == 0`.
    pub fn div(&self, other: &ComplexF) -> Option<ComplexF> {
        Some(self.mul(&other.recip()?))
    }
    /// The PRINCIPAL square root `√(a+bi)`, from the real `sqrt` only (no transcendentals):
    /// `√z = √((|z|+a)/2) + sign(b)·√((|z|−a)/2)·i`. This is the operation the exact `Complex`
    /// cannot do (its magnitude is irrational) — the reason a floating-point complex exists.
    pub fn sqrt(&self) -> ComplexF {
        let m = self.abs();
        if m == 0.0 {
            return ComplexF { re: 0.0, im: 0.0 };
        }
        let re = ((m + self.re) * 0.5).max(0.0).sqrt();
        let im0 = ((m - self.re) * 0.5).max(0.0).sqrt();
        ComplexF {
            re,
            im: if self.im < 0.0 { -im0 } else { im0 },
        }
    }
    /// `self` raised to an integer power (binary exponentiation); `None` only for zero to a
    /// negative power. `0^0 == 1`.
    pub fn pow(&self, exp: i32) -> Option<ComplexF> {
        if exp < 0 {
            return self.recip()?.pow(-exp);
        }
        let mut result = ComplexF::from_f64(1.0);
        let mut base = *self;
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
}

impl fmt::Display for ComplexF {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Basic view; the interpreter renders the user-facing form through `fmt_f64` (shortest
        // round-trip) so `2.0 + 1.0i` shows as `2+i`, matching the exact `Complex`.
        if self.im == 0.0 {
            return write!(f, "{}", self.re);
        }
        let sign = if self.im < 0.0 { "-" } else { "+" };
        write!(f, "{}{}{}i", self.re, sign, self.im.abs())
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

/// The f64 nearest to `r` (ties to even), rounded once from the exact value: the binary exponent
/// comes from the bit lengths, the quotient is scaled to 53 significant bits (or onto the `2^-1074`
/// grid below the normal range), and the remainder decides the rounding. Past the largest finite
/// float it is the signed infinity. Unlike [`Rational::to_f64`], which divides two rounded doubles,
/// this never rounds twice.
pub fn rational_to_f64_nearest(r: &Rational) -> f64 {
    if r.is_zero() {
        return 0.0;
    }
    let neg = r.is_negative();
    let n = r.numerator().abs();
    let d = r.denominator().clone();
    let e0 = n.bit_len() as i64 - d.bit_len() as i64;
    let at_least = if e0 >= 0 {
        n >= d.shl_bits(e0 as u64)
    } else {
        n.shl_bits((-e0) as u64) >= d
    };
    let e = if at_least { e0 } else { e0 - 1 };
    if e > 1023 {
        return if neg { f64::NEG_INFINITY } else { f64::INFINITY };
    }
    let shift = if e >= -1022 { 52 - e } else { 1074 };
    let (num, den) = if shift >= 0 {
        (n.shl_bits(shift as u64), d)
    } else {
        (n, d.shl_bits((-shift) as u64))
    };
    let (mut q, rem) = num.div_rem(&den).expect("the denominator is nonzero");
    let round_up = match rem.add(&rem).cmp(&den) {
        Ordering::Greater => true,
        Ordering::Less => false,
        Ordering::Equal => q.is_odd(),
    };
    if round_up {
        q = q.add(&BigInt::from_i64(1));
    }
    let significand = q.to_u64().expect("at most 2^53 after rounding") as f64;
    let magnitude = scale_by_pow2(significand, -shift);
    if neg {
        -magnitude
    } else {
        magnitude
    }
}

/// `2^j` for a normal exponent `j ∈ [-1022, 1023]`.
fn pow2_normal(j: i64) -> f64 {
    f64::from_bits(((j + 1023) as u64) << 52)
}

/// `x · 2^k`, exact whenever the product is representable: the scale is split into factors that
/// are each a normal power of two, so no intermediate overflows or underflows.
fn scale_by_pow2(x: f64, k: i64) -> f64 {
    if k < -1022 {
        x * pow2_normal(-1022) * pow2_normal(k + 1022)
    } else if k > 1023 {
        x * pow2_normal(1023) * pow2_normal(k - 1023)
    } else {
        x * pow2_normal(k)
    }
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
    if r >= NUMERIC_HASH_P {
        r - NUMERIC_HASH_P
    } else {
        r
    }
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
    if neg && h != 0 {
        NUMERIC_HASH_P - h
    } else {
        h
    }
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

/// The language's `min` of two reals: a NaN gives way to the other operand, and of two equal values the
/// one with the negative sign, so `min(0, -0.0)` is `-0.0` (IEEE 754-2019 `minimumNumber`). One answer on
/// every tier and under every optimizer, where `f64::min` leaves the sign of two equal zeros open.
pub fn float_min(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        b
    } else if b.is_nan() || a < b || (a == b && a.is_sign_negative()) {
        a
    } else {
        b
    }
}

/// The language's `max` of two reals: a NaN gives way to the other operand, and of two equal values the
/// one with the positive sign, so `max(0, -0.0)` is `0.0` (IEEE 754-2019 `maximumNumber`).
pub fn float_max(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        b
    } else if b.is_nan() || a > b || (a == b && a.is_sign_positive()) {
        a
    } else {
        b
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_base/numeric/tests.rs"]
mod tests;
