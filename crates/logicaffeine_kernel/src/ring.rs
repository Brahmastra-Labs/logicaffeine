//! Ring Tactic: Polynomial Equality by Normalization
//!
//! This module implements the `ring` decision procedure, which proves polynomial
//! equalities by converting terms to canonical polynomial form and comparing them.
//!
//! # Algorithm
//!
//! The ring tactic works in three steps:
//! 1. **Reification**: Convert Syntax terms to internal polynomial representation
//! 2. **Normalization**: Expand and combine like terms into canonical form
//! 3. **Comparison**: Check if normalized forms are structurally equal
//!
//! # Supported Operations
//!
//! - Addition (`add`)
//! - Subtraction (`sub`)
//! - Multiplication (`mul`)
//!
//! Division and modulo are not polynomial operations and are rejected.
//!
//! # Canonical Form
//!
//! Polynomials are stored as a map from monomials to coefficients.
//! Monomials are maps from variable indices to exponents.
//! BTreeMap ensures deterministic ordering for canonical comparison.
//!
//! # Exactness
//!
//! Coefficients and exponents are arbitrary-precision ([`BigInt`]): the
//! verdict feeds trusted reflection reductions, so the arithmetic must be
//! exact at every magnitude — a wrapped coefficient or exponent would equate
//! unequal polynomials.

use std::collections::BTreeMap;

use logicaffeine_base::numeric::BigInt;

use crate::reify::{extract_binary_app, extract_slit, extract_sname, extract_svar, VarInterner};
use crate::term::Term;

/// A monomial is a product of variables with their powers.
///
/// Example: x^2 * y^3 is represented as {0: 2, 1: 3}
/// The constant monomial (1) is represented as an empty map.
///
/// Uses BTreeMap for deterministic ordering (canonical form).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Monomial {
    /// Maps variable index to its exponent.
    /// Variables with exponent 0 are omitted.
    powers: BTreeMap<i64, BigInt>,
}

impl Monomial {
    /// The constant monomial (1)
    pub fn one() -> Self {
        Monomial {
            powers: BTreeMap::new(),
        }
    }

    /// A single variable: x_i^1
    pub fn var(index: i64) -> Self {
        let mut powers = BTreeMap::new();
        powers.insert(index, BigInt::from_i64(1));
        Monomial { powers }
    }

    /// Multiply two monomials by adding their exponents.
    ///
    /// For monomials m1 = x^a * y^b and m2 = x^c * z^d,
    /// the product is x^(a+c) * y^b * z^d.
    pub fn mul(&self, other: &Monomial) -> Monomial {
        let mut result = self.powers.clone();
        for (var, exp) in &other.powers {
            let entry = result.entry(*var).or_insert_with(BigInt::zero);
            *entry = entry.add(exp);
        }
        Monomial { powers: result }
    }
}

/// A polynomial is a sum of monomials with integer coefficients.
///
/// Example: 2*x^2 + 3*x*y - 5 is {x^2: 2, x*y: 3, 1: -5}
///
/// Uses BTreeMap for deterministic ordering (canonical form).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Polynomial {
    /// Maps monomials to their coefficients.
    /// Terms with coefficient 0 are omitted.
    terms: BTreeMap<Monomial, BigInt>,
}

impl Polynomial {
    /// The additive identity (zero polynomial).
    ///
    /// Represented as an empty map of terms.
    pub fn zero() -> Self {
        Polynomial {
            terms: BTreeMap::new(),
        }
    }

    /// Create a constant polynomial from an integer.
    ///
    /// Returns the zero polynomial if `c` is 0.
    pub fn constant(c: impl Into<BigInt>) -> Self {
        let c = c.into();
        if c.is_zero() {
            return Self::zero();
        }
        let mut terms = BTreeMap::new();
        terms.insert(Monomial::one(), c);
        Polynomial { terms }
    }

    /// A single variable: x_i
    pub fn var(index: i64) -> Self {
        let mut terms = BTreeMap::new();
        terms.insert(Monomial::var(index), BigInt::from_i64(1));
        Polynomial { terms }
    }

    /// Add two polynomials
    pub fn add(&self, other: &Polynomial) -> Polynomial {
        let mut result = self.terms.clone();
        for (mono, coeff) in &other.terms {
            let entry = result.entry(mono.clone()).or_insert_with(BigInt::zero);
            *entry = entry.add(coeff);
            if entry.is_zero() {
                result.remove(mono);
            }
        }
        Polynomial { terms: result }
    }

    /// Negate a polynomial
    pub fn neg(&self) -> Polynomial {
        let mut result = BTreeMap::new();
        for (mono, coeff) in &self.terms {
            result.insert(mono.clone(), coeff.negated());
        }
        Polynomial { terms: result }
    }

    /// Subtract two polynomials
    pub fn sub(&self, other: &Polynomial) -> Polynomial {
        self.add(&other.neg())
    }

    /// Multiply two polynomials
    pub fn mul(&self, other: &Polynomial) -> Polynomial {
        let mut result = Polynomial::zero();
        for (m1, c1) in &self.terms {
            for (m2, c2) in &other.terms {
                let mono = m1.mul(m2);
                let coeff = c1.mul(c2);
                let entry = result.terms.entry(mono).or_insert_with(BigInt::zero);
                *entry = entry.add(&coeff);
            }
        }
        // Clean up zero coefficients
        result.terms.retain(|_, c| !c.is_zero());
        result
    }

    /// Check equality in canonical form.
    /// Since BTreeMap maintains sorted order and we remove zeros,
    /// structural equality is semantic equality.
    pub fn canonical_eq(&self, other: &Polynomial) -> bool {
        self.terms == other.terms
    }
}

/// Error during reification of a term to polynomial form.
#[derive(Debug)]
pub enum ReifyError {
    /// Term contains operations not supported in polynomial arithmetic.
    ///
    /// This includes division, modulo, and unknown function symbols.
    NonPolynomial(String),
    /// Term has an unexpected structure that cannot be parsed.
    MalformedTerm,
}

/// Reify a Syntax term into a polynomial representation.
///
/// This function converts the deep embedding of terms (Syntax) into
/// the internal polynomial representation used for normalization.
///
/// # Supported Term Forms
///
/// - `SLit n` - Integer literal becomes a constant polynomial
/// - `SVar i` - De Bruijn variable becomes a polynomial variable
/// - `SName "x"` - Named global becomes a polynomial variable (interned)
/// - `SApp (SApp (SName "add") a) b` - Addition of two terms
/// - `SApp (SApp (SName "sub") a) b` - Subtraction of two terms
/// - `SApp (SApp (SName "mul") a) b` - Multiplication of two terms
///
/// # Errors
///
/// Returns [`ReifyError::NonPolynomial`] for unsupported operations (div, mod)
/// or unknown function symbols.
///
/// # Named Variables
///
/// Named globals draw their indices from `vars`, so distinct names get
/// distinct variables and repeated names get the same one. Every term
/// reified for one goal must share one interner — comparing polynomials
/// reified under different interners is meaningless.
pub fn reify(term: &Term, vars: &mut VarInterner) -> Result<Polynomial, ReifyError> {
    // SLit n -> constant
    if let Some(n) = extract_slit(term) {
        return Ok(Polynomial::constant(n));
    }

    // SVar i -> variable
    if let Some(i) = extract_svar(term) {
        return Ok(Polynomial::var(i));
    }

    // SName "x" -> treat as variable (global constant)
    if let Some(name) = extract_sname(term) {
        return Ok(Polynomial::var(vars.intern(&name)));
    }

    // SApp (SApp (SName "op") a) b -> binary operation
    if let Some((op, a, b)) = extract_binary_app(term) {
        match op.as_str() {
            "add" => {
                let pa = reify(&a, vars)?;
                let pb = reify(&b, vars)?;
                return Ok(pa.add(&pb));
            }
            "sub" => {
                let pa = reify(&a, vars)?;
                let pb = reify(&b, vars)?;
                return Ok(pa.sub(&pb));
            }
            "mul" => {
                let pa = reify(&a, vars)?;
                let pb = reify(&b, vars)?;
                return Ok(pa.mul(&pb));
            }
            "div" | "mod" => {
                return Err(ReifyError::NonPolynomial(format!(
                    "Operation '{}' is not supported in ring",
                    op
                )));
            }
            _ => {
                return Err(ReifyError::NonPolynomial(format!(
                    "Unknown operation '{}'",
                    op
                )));
            }
        }
    }

    // Cannot reify this term
    Err(ReifyError::NonPolynomial(
        "Unrecognized term structure".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_polynomial_constant() {
        let p = Polynomial::constant(42);
        assert_eq!(p, Polynomial::constant(42));
    }

    #[test]
    fn test_polynomial_add() {
        let x = Polynomial::var(0);
        let y = Polynomial::var(1);
        let sum1 = x.add(&y);
        let sum2 = y.add(&x);
        assert!(sum1.canonical_eq(&sum2), "x+y should equal y+x");
    }

    #[test]
    fn test_polynomial_mul() {
        let x = Polynomial::var(0);
        let y = Polynomial::var(1);
        let prod1 = x.mul(&y);
        let prod2 = y.mul(&x);
        assert!(prod1.canonical_eq(&prod2), "x*y should equal y*x");
    }

    #[test]
    fn test_polynomial_distributivity() {
        let x = Polynomial::var(0);
        let y = Polynomial::var(1);
        let z = Polynomial::var(2);

        // x*(y+z) should equal x*y + x*z
        let lhs = x.mul(&y.add(&z));
        let rhs = x.mul(&y).add(&x.mul(&z));
        assert!(lhs.canonical_eq(&rhs));
    }

    #[test]
    fn test_polynomial_subtraction() {
        let x = Polynomial::var(0);
        let result = x.sub(&x);
        assert!(result.canonical_eq(&Polynomial::zero()));
    }

    #[test]
    fn test_collatz_algebra() {
        // 3(2k+1) + 1 = 6k + 4
        let k = Polynomial::var(0);
        let two = Polynomial::constant(2);
        let three = Polynomial::constant(3);
        let one = Polynomial::constant(1);
        let four = Polynomial::constant(4);
        let six = Polynomial::constant(6);

        // LHS: 3*(2*k + 1) + 1
        let two_k = two.mul(&k);
        let two_k_plus_1 = two_k.add(&one);
        let three_times = three.mul(&two_k_plus_1);
        let lhs = three_times.add(&one);

        // RHS: 6*k + 4
        let six_k = six.mul(&k);
        let rhs = six_k.add(&four);

        assert!(lhs.canonical_eq(&rhs), "3(2k+1)+1 should equal 6k+4");
    }
}
