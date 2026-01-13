//! Ring Tactic: Polynomial Equality by Normalization
//!
//! The ring tactic proves polynomial equalities by:
//! 1. Reifying Syntax terms to internal polynomial representation
//! 2. Normalizing both sides to canonical form (sum of monomials)
//! 3. Checking if normalized forms are structurally equal
//!
//! Uses BTreeMap for automatic canonical ordering.

use std::collections::BTreeMap;

use super::term::{Literal, Term};

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
    powers: BTreeMap<i64, u32>,
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
        powers.insert(index, 1);
        Monomial { powers }
    }

    /// Multiply two monomials
    pub fn mul(&self, other: &Monomial) -> Monomial {
        let mut result = self.powers.clone();
        for (var, exp) in &other.powers {
            *result.entry(*var).or_insert(0) += exp;
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
    terms: BTreeMap<Monomial, i64>,
}

impl Polynomial {
    /// The zero polynomial
    pub fn zero() -> Self {
        Polynomial {
            terms: BTreeMap::new(),
        }
    }

    /// A constant polynomial
    pub fn constant(c: i64) -> Self {
        if c == 0 {
            return Self::zero();
        }
        let mut terms = BTreeMap::new();
        terms.insert(Monomial::one(), c);
        Polynomial { terms }
    }

    /// A single variable: x_i
    pub fn var(index: i64) -> Self {
        let mut terms = BTreeMap::new();
        terms.insert(Monomial::var(index), 1);
        Polynomial { terms }
    }

    /// Add two polynomials
    pub fn add(&self, other: &Polynomial) -> Polynomial {
        let mut result = self.terms.clone();
        for (mono, coeff) in &other.terms {
            let entry = result.entry(mono.clone()).or_insert(0);
            *entry += coeff;
            if *entry == 0 {
                result.remove(mono);
            }
        }
        Polynomial { terms: result }
    }

    /// Negate a polynomial
    pub fn neg(&self) -> Polynomial {
        let mut result = BTreeMap::new();
        for (mono, coeff) in &self.terms {
            result.insert(mono.clone(), -coeff);
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
                let coeff = c1 * c2;
                let entry = result.terms.entry(mono).or_insert(0);
                *entry += coeff;
            }
        }
        // Clean up zero coefficients
        result.terms.retain(|_, c| *c != 0);
        result
    }

    /// Check equality in canonical form.
    /// Since BTreeMap maintains sorted order and we remove zeros,
    /// structural equality is semantic equality.
    pub fn canonical_eq(&self, other: &Polynomial) -> bool {
        self.terms == other.terms
    }
}

/// Error during reification
#[derive(Debug)]
pub enum ReifyError {
    /// Term cannot be reified (e.g., division, unknown function)
    NonPolynomial(String),
    /// Unexpected term structure
    MalformedTerm,
}

/// Reify a Syntax term into a polynomial.
///
/// Handles:
/// - SLit n -> constant polynomial
/// - SVar i -> variable polynomial
/// - SName "x" -> named variable (hashed to unique index)
/// - SApp (SApp (SName "add") a) b -> a + b
/// - SApp (SApp (SName "sub") a) b -> a - b
/// - SApp (SApp (SName "mul") a) b -> a * b
pub fn reify(term: &Term) -> Result<Polynomial, ReifyError> {
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
        // Use negative indices for named globals to distinguish from SVar
        let hash = name_to_var_index(&name);
        return Ok(Polynomial::var(hash));
    }

    // SApp (SApp (SName "op") a) b -> binary operation
    if let Some((op, a, b)) = extract_binary_app(term) {
        match op.as_str() {
            "add" => {
                let pa = reify(&a)?;
                let pb = reify(&b)?;
                return Ok(pa.add(&pb));
            }
            "sub" => {
                let pa = reify(&a)?;
                let pb = reify(&b)?;
                return Ok(pa.sub(&pb));
            }
            "mul" => {
                let pa = reify(&a)?;
                let pb = reify(&b)?;
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

/// Extract integer from SLit n
fn extract_slit(term: &Term) -> Option<i64> {
    // Pattern: App(Global("SLit"), Lit(Int(n)))
    if let Term::App(ctor, arg) = term {
        if let Term::Global(name) = ctor.as_ref() {
            if name == "SLit" {
                if let Term::Lit(Literal::Int(n)) = arg.as_ref() {
                    return Some(*n);
                }
            }
        }
    }
    None
}

/// Extract variable index from SVar i
fn extract_svar(term: &Term) -> Option<i64> {
    // Pattern: App(Global("SVar"), Lit(Int(i)))
    if let Term::App(ctor, arg) = term {
        if let Term::Global(name) = ctor.as_ref() {
            if name == "SVar" {
                if let Term::Lit(Literal::Int(i)) = arg.as_ref() {
                    return Some(*i);
                }
            }
        }
    }
    None
}

/// Extract name from SName "x"
fn extract_sname(term: &Term) -> Option<String> {
    // Pattern: App(Global("SName"), Lit(Text(s)))
    if let Term::App(ctor, arg) = term {
        if let Term::Global(name) = ctor.as_ref() {
            if name == "SName" {
                if let Term::Lit(Literal::Text(s)) = arg.as_ref() {
                    return Some(s.clone());
                }
            }
        }
    }
    None
}

/// Extract binary application: SApp (SApp (SName "op") a) b
fn extract_binary_app(term: &Term) -> Option<(String, Term, Term)> {
    // Structure: App(App(SApp, App(App(SApp, op_term), a)), b)
    // Which represents: SApp (SApp op a) b
    if let Term::App(outer, b) = term {
        if let Term::App(sapp_outer, inner) = outer.as_ref() {
            if let Term::Global(ctor) = sapp_outer.as_ref() {
                if ctor == "SApp" {
                    // inner should be: App(App(SApp, op), a)
                    if let Term::App(partial, a) = inner.as_ref() {
                        if let Term::App(sapp_inner, op_term) = partial.as_ref() {
                            if let Term::Global(ctor2) = sapp_inner.as_ref() {
                                if ctor2 == "SApp" {
                                    if let Some(op) = extract_sname(op_term) {
                                        return Some((
                                            op,
                                            a.as_ref().clone(),
                                            b.as_ref().clone(),
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// Convert a name to a unique negative variable index
fn name_to_var_index(name: &str) -> i64 {
    // Use a hash of the name, made negative to distinguish from SVar indices
    let hash: i64 = name
        .bytes()
        .fold(0i64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as i64));
    -(hash.abs() + 1_000_000) // Ensure it's negative and far from typical SVar indices
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
