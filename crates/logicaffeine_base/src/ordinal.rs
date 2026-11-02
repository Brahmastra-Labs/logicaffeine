//! Ordinals below ε₀ in Cantor normal form — the stage counter of transfinite execution.
//!
//! Every ordinal `α < ε₀` is uniquely `ω^{β₁}·c₁ + ω^{β₂}·c₂ + … + ω^{βₖ}·cₖ` with exponents
//! `β₁ > β₂ > … > βₖ` (themselves ordinals below ε₀, in the same form) and positive natural
//! coefficients. An [`Ordinal`] stores exactly that list, so equality is structural and the order
//! is lexicographic on (exponent, coefficient) pairs, a longer list winning a common prefix.
//!
//! The arithmetic is the ordinal arithmetic, not the arithmetic of naturals, and its asymmetries
//! are the point:
//!
//! - **addition** absorbs every term of the left summand below the right summand's leading
//!   exponent: `1 + ω = ω`, yet `ω + 1 > ω`;
//! - **multiplication** distributes on the left only: `α·(β + γ) = α·β + α·γ`, while
//!   `(1 + 1)·ω = ω ≠ ω + ω`; a term `ω^β·c` of the right factor contributes `ω^{α₁ + β}·c` when
//!   `β > 0` and `α·c` (the leading coefficient scaled, the rest kept) when `β = 0`;
//! - **exponentiation** `α^β = ∏ (α^{ω^{βᵢ}})^{cᵢ}`, where for `β ≥ 1` a finite base `n ≥ 2` gives
//!   `n^{ω^β} = ω^{ω^{β'}}` (`β = 1 + β'`) and an infinite base gives `α^{ω^β} = ω^{α₁·ω^β}` —
//!   so `2^ω = ω` and `(ω + 1)^ω = ω^ω`.

use crate::numeric::BigInt;
use std::cmp::Ordering;
use std::fmt;

/// An ordinal below ε₀. See the module documentation.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Ordinal {
    /// `(exponent, coefficient)` with strictly decreasing exponents and positive coefficients.
    terms: Vec<(Ordinal, BigInt)>,
}

impl Ordinal {
    pub fn zero() -> Ordinal {
        Ordinal { terms: Vec::new() }
    }

    pub fn one() -> Ordinal {
        Self::from_u64(1)
    }

    pub fn from_u64(n: u64) -> Ordinal {
        Self::from_natural(BigInt::from_u64(n))
    }

    /// The finite ordinal `n`. Panics on a negative `n`.
    pub fn from_natural(n: BigInt) -> Ordinal {
        assert!(!n.is_negative(), "an ordinal is never negative");
        if n.is_zero() {
            return Self::zero();
        }
        Ordinal {
            terms: vec![(Self::zero(), n)],
        }
    }

    pub fn omega() -> Ordinal {
        Self::omega_pow(Self::one())
    }

    /// `ω^e`.
    pub fn omega_pow(e: Ordinal) -> Ordinal {
        Ordinal {
            terms: vec![(e, BigInt::from_i64(1))],
        }
    }

    pub fn is_zero(&self) -> bool {
        self.terms.is_empty()
    }

    pub fn is_finite(&self) -> bool {
        self.terms.iter().all(|(e, _)| e.is_zero())
    }

    pub fn to_natural(&self) -> Option<BigInt> {
        match self.terms.as_slice() {
            [] => Some(BigInt::zero()),
            [(e, c)] if e.is_zero() => Some(c.clone()),
            _ => None,
        }
    }

    /// `α + 1`.
    pub fn successor(&self) -> Ordinal {
        self.add(&Self::one())
    }

    /// Ends in a finite term: `α = β + 1` for some `β`.
    pub fn is_successor(&self) -> bool {
        self.terms.last().is_some_and(|(e, _)| e.is_zero())
    }

    /// Nonzero and not a successor: the supremum of everything below it.
    pub fn is_limit(&self) -> bool {
        self.terms.last().is_some_and(|(e, _)| !e.is_zero())
    }

    /// `β` with `β + 1 = self`, for a successor.
    pub fn predecessor(&self) -> Option<Ordinal> {
        if !self.is_successor() {
            return None;
        }
        let mut terms = self.terms.clone();
        let (_, c) = terms.last_mut().expect("a successor has a last term");
        *c = c.sub(&BigInt::from_i64(1));
        if c.is_zero() {
            terms.pop();
        }
        Some(Ordinal { terms })
    }

    /// The exponent of the leading term; `None` for zero.
    fn leading_exponent(&self) -> Option<&Ordinal> {
        self.terms.first().map(|(e, _)| e)
    }

    pub fn add(&self, o: &Ordinal) -> Ordinal {
        let Some(lead) = o.leading_exponent() else {
            return self.clone();
        };
        let mut terms: Vec<(Ordinal, BigInt)> = self
            .terms
            .iter()
            .take_while(|(e, _)| e >= lead)
            .cloned()
            .collect();
        let mut rest = o.terms.iter();
        if let Some((e, c)) = terms.last_mut() {
            if e == lead {
                let (_, oc) = rest.next().expect("o has a leading term");
                *c = c.add(oc);
            }
        }
        terms.extend(rest.cloned());
        Ordinal { terms }
    }

    pub fn mul(&self, o: &Ordinal) -> Ordinal {
        let Some(lead) = self.leading_exponent() else {
            return Self::zero();
        };
        let mut acc = Self::zero();
        for (beta, c) in &o.terms {
            let part = if beta.is_zero() {
                let mut terms = self.terms.clone();
                terms[0].1 = terms[0].1.mul(c);
                Ordinal { terms }
            } else {
                Ordinal {
                    terms: vec![(lead.add(beta), c.clone())],
                }
            };
            acc = acc.add(&part);
        }
        acc
    }

    /// `self^e` (with `0^0 = 1`).
    pub fn pow(&self, e: &Ordinal) -> Ordinal {
        if e.is_zero() {
            return Self::one();
        }
        if self.is_zero() {
            return Self::zero();
        }
        if *self == Self::one() {
            return Self::one();
        }
        let mut acc = Self::one();
        for (beta, c) in &e.terms {
            let base = if beta.is_zero() {
                self.clone()
            } else {
                self.pow_omega_power(beta)
            };
            acc = acc.mul(&base.pow_natural(c));
        }
        acc
    }

    /// `self^{ω^β}` for `β ≥ 1` and `self ≥ 2`.
    fn pow_omega_power(&self, beta: &Ordinal) -> Ordinal {
        if self.is_finite() {
            let rest = match beta.to_natural() {
                Some(n) => Self::from_natural(n.sub(&BigInt::from_i64(1))),
                None => beta.clone(),
            };
            Self::omega_pow(Self::omega_pow(rest))
        } else {
            let lead = self.leading_exponent().expect("an infinite ordinal is nonzero");
            Self::omega_pow(lead.mul(&Self::omega_pow(beta.clone())))
        }
    }

    fn pow_natural(&self, n: &BigInt) -> Ordinal {
        let mut acc = Self::one();
        let mut base = self.clone();
        let mut k = n.clone();
        let two = BigInt::from_i64(2);
        while !k.is_zero() {
            let (q, r) = k.div_rem(&two).expect("2 is nonzero");
            if !r.is_zero() {
                acc = acc.mul(&base);
            }
            k = q;
            if !k.is_zero() {
                base = base.mul(&base);
            }
        }
        acc
    }
}

impl Ord for Ordinal {
    fn cmp(&self, o: &Self) -> Ordering {
        for (a, b) in self.terms.iter().zip(o.terms.iter()) {
            match a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)) {
                Ordering::Equal => continue,
                other => return other,
            }
        }
        self.terms.len().cmp(&o.terms.len())
    }
}

impl PartialOrd for Ordinal {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl fmt::Display for Ordinal {
    /// `ω^2·3 + ω + 5`, `ω^(ω + 1)`, `ω^ω^ω`: an exponent is parenthesised unless it is a
    /// natural or a single bare power of ω.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_zero() {
            return write!(f, "0");
        }
        let one = BigInt::from_i64(1);
        for (i, (e, c)) in self.terms.iter().enumerate() {
            if i > 0 {
                write!(f, " + ")?;
            }
            if e.is_zero() {
                write!(f, "{c}")?;
                continue;
            }
            if *e == Ordinal::one() {
                write!(f, "ω")?;
            } else if e.is_finite() || (e.terms.len() == 1 && e.terms[0].1 == one) {
                write!(f, "ω^{e}")?;
            } else {
                write!(f, "ω^({e})")?;
            }
            if *c != one {
                write!(f, "·{c}")?;
            }
        }
        Ok(())
    }
}

impl crate::exact_codec::ExactCodec for Ordinal {
    /// The Cantor normal form: the number of terms, then each exponent (recursively) and its
    /// coefficient.
    fn write(&self, out: &mut Vec<u8>) {
        crate::exact_codec::write_len(self.terms.len(), out);
        for (e, c) in &self.terms {
            e.write(out);
            c.write(out);
        }
    }

    fn read(buf: &[u8], pos: &mut usize) -> Option<Self> {
        read_ordinal(buf, pos, 0)
    }
}

/// An ordinal at nesting `depth`, refused unless its exponents strictly decrease and its
/// coefficients are positive.
fn read_ordinal(buf: &[u8], pos: &mut usize, depth: usize) -> Option<Ordinal> {
    use crate::exact_codec::ExactCodec;
    if depth > crate::exact_codec::MAX_WIRE_DEPTH {
        return None;
    }
    let n = crate::exact_codec::read_len(buf, pos, crate::exact_codec::MAX_WIRE_LEN)?;
    let mut terms: Vec<(Ordinal, BigInt)> = Vec::with_capacity(n.min(64));
    for _ in 0..n {
        let e = read_ordinal(buf, pos, depth + 1)?;
        let c = BigInt::read(buf, pos)?;
        if !c.is_positive() || terms.last().is_some_and(|(prev, _)| *prev <= e) {
            return None;
        }
        terms.push((e, c));
    }
    Some(Ordinal { terms })
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_base/ordinal/tests.rs"]
mod tests;
