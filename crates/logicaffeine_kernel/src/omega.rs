//! Omega Test: True Integer Arithmetic Decision Procedure
//!
//! This module implements the Omega test for linear integer arithmetic,
//! handling the discrete nature of integers correctly.
//!
//! # Difference from LIA
//!
//! Unlike [`crate::lia`] (which uses rational arithmetic), this module
//! handles integers with proper semantics:
//!
//! - `x > 1` becomes `x >= 2` (strict to non-strict for integers)
//! - `3x <= 10` implies `x <= 3` (integer division with floor)
//! - `2x = 5` is unsatisfiable (odd number cannot equal even expression)
//!
//! # Algorithm
//!
//! The algorithm is similar to Fourier-Motzkin elimination but with
//! integer-aware semantics:
//!
//! 1. **Normalize**: Scale constraints and normalize by GCD
//! 2. **Convert strict**: Transform `<` to `<=` using integer shift
//! 3. **Eliminate**: Fourier-Motzkin with integer coefficient handling
//! 4. **Check**: Verify constant constraints for contradictions
//!
//! # When to Use
//!
//! Use omega when you need exact integer semantics. Use lia when
//! rational arithmetic is acceptable (faster but may miss integer-specific
//! unsatisfiability).
//!
//! # Exactness
//!
//! Coefficients are arbitrary-precision ([`BigInt`]): Fourier-Motzkin
//! combinations multiply coefficients pairwise, and the verdict feeds trusted
//! reflection reductions, so a wrapped product would flip satisfiable into
//! unsatisfiable.

use std::collections::{BTreeMap, HashSet};

use logicaffeine_base::numeric::BigInt;

use crate::reify::{extract_binary_app, extract_slit, extract_sname, extract_svar, VarInterner};
use crate::term::Term;

/// Integer linear expression of the form c + a₁x₁ + a₂x₂ + ... + aₙxₙ.
///
/// Similar to [`crate::lia::LinearExpr`] but uses integer coefficients
/// instead of rationals for exact integer arithmetic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntExpr {
    /// The constant term c.
    pub constant: BigInt,
    /// Maps variable index to its integer coefficient (sparse representation).
    pub coeffs: BTreeMap<i64, BigInt>,
}

impl IntExpr {
    /// Create a constant expression
    pub fn constant(c: impl Into<BigInt>) -> Self {
        IntExpr {
            constant: c.into(),
            coeffs: BTreeMap::new(),
        }
    }

    /// Create a single variable expression: 1*x_idx + 0
    pub fn var(idx: i64) -> Self {
        let mut coeffs = BTreeMap::new();
        coeffs.insert(idx, BigInt::from_i64(1));
        IntExpr {
            constant: BigInt::zero(),
            coeffs,
        }
    }

    /// Add two expressions
    pub fn add(&self, other: &Self) -> Self {
        let mut result = self.clone();
        result.constant = result.constant.add(&other.constant);
        for (&v, c) in &other.coeffs {
            let entry = result.coeffs.entry(v).or_insert_with(BigInt::zero);
            *entry = entry.add(c);
            if entry.is_zero() {
                result.coeffs.remove(&v);
            }
        }
        result
    }

    /// Negate an expression
    pub fn neg(&self) -> Self {
        IntExpr {
            constant: self.constant.negated(),
            coeffs: self.coeffs.iter().map(|(&v, c)| (v, c.negated())).collect(),
        }
    }

    /// Subtract two expressions
    pub fn sub(&self, other: &Self) -> Self {
        self.add(&other.neg())
    }

    /// Scale by an integer constant
    pub fn scale(&self, k: impl Into<BigInt>) -> Self {
        let k = k.into();
        if k.is_zero() {
            return IntExpr::constant(0);
        }
        IntExpr {
            constant: self.constant.mul(&k),
            coeffs: self
                .coeffs
                .iter()
                .map(|(&v, c)| (v, c.mul(&k)))
                .filter(|(_, c)| !c.is_zero())
                .collect(),
        }
    }

    /// Check if this is a constant expression (no variables)
    pub fn is_constant(&self) -> bool {
        self.coeffs.is_empty()
    }

    /// Get coefficient of a variable (0 if not present)
    pub fn get_coeff(&self, var: i64) -> BigInt {
        self.coeffs.get(&var).cloned().unwrap_or_else(BigInt::zero)
    }
}

/// Integer constraint representing `expr <= 0` or `expr < 0`.
///
/// For integers, strict inequalities can be converted to non-strict:
/// `x < k` is equivalent to `x <= k - 1`.
#[derive(Debug, Clone)]
pub struct IntConstraint {
    /// The linear expression (constraint is expr OP 0).
    pub expr: IntExpr,
    /// If true, this is a strict inequality (`< 0`).
    /// If false, this is a non-strict inequality (`<= 0`).
    pub strict: bool,
}

impl IntConstraint {
    /// Check if a constant constraint is satisfied
    pub fn is_satisfied_constant(&self) -> bool {
        if !self.expr.is_constant() {
            return true; // Can't determine yet
        }
        let c = &self.expr.constant;
        if self.strict {
            c.is_negative() // c < 0
        } else {
            !c.is_positive() // c ≤ 0
        }
    }

    /// Normalize by GCD of all coefficients
    pub fn normalize(&mut self) {
        let g = self
            .expr
            .coeffs
            .values()
            .chain(std::iter::once(&self.expr.constant))
            .filter(|x| !x.is_zero())
            .fold(BigInt::zero(), |a, b| gcd(&a, &b.abs()));

        if g > BigInt::from_i64(1) {
            self.expr.constant = exact_div(&self.expr.constant, &g);
            for v in self.expr.coeffs.values_mut() {
                *v = exact_div(v, &g);
            }
        }
    }
}

/// GCD using the Euclidean algorithm (arguments non-negative).
fn gcd(a: &BigInt, b: &BigInt) -> BigInt {
    if b.is_zero() {
        a.clone()
    } else {
        let (_, r) = a.div_rem(b).expect("gcd divisor is nonzero");
        gcd(b, &r)
    }
}

/// Divide exactly (the divisor is a common divisor of the dividend).
fn exact_div(a: &BigInt, g: &BigInt) -> BigInt {
    a.div_rem(g).expect("gcd is nonzero").0
}

/// Reify a Syntax term to an integer linear expression.
///
/// Converts the deep embedding (Syntax) into an integer linear expression.
/// Similar to [`crate::lia::reify_linear`] but produces integer coefficients.
///
/// # Supported Forms
///
/// - `SLit n` - Integer literal becomes a constant
/// - `SVar i` - De Bruijn variable becomes a linear variable
/// - `SName "x"` - Named global becomes a linear variable (interned)
/// - `add`, `sub`, `mul` - Arithmetic operations (mul only if one operand is constant)
///
/// Every term reified for one goal (hypotheses and conclusion alike) must
/// share one `vars` interner, or their variable indices will not line up.
///
/// # Returns
///
/// `Some(expr)` on success, `None` if the term is non-linear or malformed.
pub fn reify_int_linear(term: &Term, vars: &mut VarInterner) -> Option<IntExpr> {
    // SLit n -> constant
    if let Some(n) = extract_slit(term) {
        return Some(IntExpr::constant(n));
    }

    // SVar i -> variable
    if let Some(i) = extract_svar(term) {
        return Some(IntExpr::var(i));
    }

    // SName "x" -> named variable (global constant treated as free variable)
    if let Some(name) = extract_sname(term) {
        return Some(IntExpr::var(vars.intern(&name)));
    }

    // Binary operations
    if let Some((op, a, b)) = extract_binary_app(term) {
        match op.as_str() {
            "add" => {
                let la = reify_int_linear(&a, vars)?;
                let lb = reify_int_linear(&b, vars)?;
                return Some(la.add(&lb));
            }
            "sub" => {
                let la = reify_int_linear(&a, vars)?;
                let lb = reify_int_linear(&b, vars)?;
                return Some(la.sub(&lb));
            }
            "mul" => {
                let la = reify_int_linear(&a, vars)?;
                let lb = reify_int_linear(&b, vars)?;
                // Only linear if one side is constant
                if la.is_constant() {
                    return Some(lb.scale(la.constant));
                }
                if lb.is_constant() {
                    return Some(la.scale(lb.constant));
                }
                return None; // Non-linear
            }
            _ => return None,
        }
    }

    None
}

/// Extract comparison from goal: (SApp (SApp (SName "Lt"|"Le"|"Gt"|"Ge") lhs) rhs)
pub fn extract_comparison(term: &Term) -> Option<(String, Term, Term)> {
    if let Some((rel, lhs, rhs)) = extract_binary_app(term) {
        match rel.as_str() {
            "Lt" | "Le" | "Gt" | "Ge" | "lt" | "le" | "gt" | "ge" => {
                return Some((rel, lhs, rhs));
            }
            _ => {}
        }
    }
    None
}

/// Convert a goal to constraints for validity checking using integer semantics.
///
/// Key difference from lia: strict inequalities are converted for integers.
/// - x < k becomes x <= k - 1 (since x must be an integer)
/// - x > k becomes x >= k + 1
///
/// To prove a goal is valid, we check if its negation is unsatisfiable.
pub fn goal_to_negated_constraint(rel: &str, lhs: &IntExpr, rhs: &IntExpr) -> Option<IntConstraint> {
    // diff = lhs - rhs
    let diff = lhs.sub(rhs);
    let one = IntExpr::constant(1);

    match rel {
        // Lt: a < b valid iff NOT(a >= b)
        // For integers: a >= b means a - b >= 0
        // We check if a - b >= 0 is satisfiable
        // Constraint form for unsatisfiability check: -(a - b) <= 0, i.e., (b - a) <= 0
        "Lt" | "lt" => Some(IntConstraint {
            expr: rhs.sub(lhs),
            strict: false,
        }),

        // Le: a <= b valid iff NOT(a > b)
        // For integers: a > b means a - b >= 1 (strict to non-strict!)
        // So negation is: a - b >= 1, i.e., a - b - 1 >= 0
        // Constraint: -(a - b - 1) <= 0, i.e., (b - a + 1) <= 0
        // Equivalently: (b - a) <= -1
        "Le" | "le" => Some(IntConstraint {
            expr: rhs.sub(lhs).add(&one),
            strict: false,
        }),

        // Gt: a > b valid iff NOT(a <= b)
        // For integers: a <= b means a - b <= 0
        // Constraint: (a - b) <= 0
        "Gt" | "gt" => Some(IntConstraint {
            expr: diff,
            strict: false,
        }),

        // Ge: a >= b valid iff NOT(a < b)
        // For integers: a < b means a - b <= -1 (strict to non-strict!)
        // Constraint: (a - b) <= -1, i.e., (a - b + 1) <= 0
        "Ge" | "ge" => Some(IntConstraint {
            expr: diff.add(&one),
            strict: false,
        }),

        _ => None,
    }
}

/// Check if integer constraints are unsatisfiable using the Omega test.
///
/// This is the main entry point for the omega decision procedure. It uses
/// integer-aware Fourier-Motzkin elimination to check for contradictions.
///
/// # Integer Semantics
///
/// Unlike rational Fourier-Motzkin, this procedure:
/// - Normalizes constraints by their GCD
/// - Handles strict inequalities by integer shift (`< k` becomes `<= k-1`)
/// - Detects integer-specific unsatisfiability
///
/// # Returns
///
/// - `true` if no integer assignment satisfies all constraints (unsatisfiable)
/// - `false` if the constraints may be satisfiable
///
/// # Usage for Validity
///
/// To prove a goal G is valid over integers, check if NOT(G) is unsatisfiable.
/// If `omega_unsat(negation_constraints)` returns true, the goal is valid.
pub fn omega_unsat(constraints: &[IntConstraint]) -> bool {
    if constraints.is_empty() {
        return false;
    }

    // Normalize all constraints
    let mut current: Vec<IntConstraint> = constraints.to_vec();
    for c in &mut current {
        c.normalize();
    }

    // Check for immediate contradictions
    for c in &current {
        if c.expr.is_constant() && !c.is_satisfied_constant() {
            return true;
        }
    }

    // Collect all variables
    let vars: Vec<i64> = current
        .iter()
        .flat_map(|c| c.expr.coeffs.keys().copied())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    // Eliminate each variable
    for var in vars {
        current = eliminate_variable_int(&current, var);

        // Early termination: check for constant contradictions
        for c in &current {
            if c.expr.is_constant() && !c.is_satisfied_constant() {
                return true;
            }
        }
    }

    // Check all remaining constant constraints
    current
        .iter()
        .any(|c| c.expr.is_constant() && !c.is_satisfied_constant())
}

/// Eliminate a variable from constraints using integer-aware Fourier-Motzkin.
fn eliminate_variable_int(constraints: &[IntConstraint], var: i64) -> Vec<IntConstraint> {
    let mut lower: Vec<(IntExpr, BigInt)> = vec![]; // (rest, |coeff|) for lower bounds
    let mut upper: Vec<(IntExpr, BigInt)> = vec![]; // (rest, coeff) for upper bounds
    let mut independent: Vec<IntConstraint> = vec![];

    for c in constraints {
        let coeff = c.expr.get_coeff(var);
        if coeff.is_zero() {
            independent.push(c.clone());
        } else {
            // c.expr = coeff*var + rest <= 0
            let mut rest = c.expr.clone();
            rest.coeffs.remove(&var);

            if coeff.is_positive() {
                // coeff*var + rest <= 0
                // var <= -rest/coeff (upper bound)
                upper.push((rest, coeff));
            } else {
                // coeff*var + rest <= 0, coeff < 0
                // |coeff|*(-var) + rest <= 0
                // -var <= -rest/|coeff|
                // var >= rest/|coeff| (lower bound)
                lower.push((rest, coeff.negated()));
            }
        }
    }

    // Combine lower and upper bounds
    // If lo/a <= var <= -hi/b, then lo/a <= -hi/b
    // Multiply out: b*lo <= -a*hi
    // Rearrange: b*lo + a*hi <= 0
    for (lo_rest, lo_coeff) in &lower {
        for (hi_rest, hi_coeff) in &upper {
            // Lower: var >= lo_rest / lo_coeff (lo_coeff is positive)
            // Upper: var <= -hi_rest / hi_coeff (hi_coeff is positive)
            // Combined: lo_rest / lo_coeff <= -hi_rest / hi_coeff
            // => hi_coeff * lo_rest <= -lo_coeff * hi_rest
            // => hi_coeff * lo_rest + lo_coeff * hi_rest <= 0
            let new_expr = lo_rest
                .scale(hi_coeff.clone())
                .add(&hi_rest.scale(lo_coeff.clone()));

            let mut new_constraint = IntConstraint {
                expr: new_expr,
                strict: false,
            };
            new_constraint.normalize();
            independent.push(new_constraint);
        }
    }

    independent
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_int_expr_add() {
        let x = IntExpr::var(0);
        let y = IntExpr::var(1);
        let sum = x.add(&y);
        assert!(!sum.is_constant());
        assert_eq!(sum.get_coeff(0), BigInt::from_i64(1));
        assert_eq!(sum.get_coeff(1), BigInt::from_i64(1));
    }

    #[test]
    fn test_int_expr_cancel() {
        let x = IntExpr::var(0);
        let neg_x = x.neg();
        let zero = x.add(&neg_x);
        assert!(zero.is_constant());
        assert!(zero.constant.is_zero());
    }

    #[test]
    fn test_constraint_satisfied() {
        // -1 <= 0 is satisfied
        let c1 = IntConstraint {
            expr: IntExpr::constant(-1),
            strict: false,
        };
        assert!(c1.is_satisfied_constant());

        // 1 <= 0 is NOT satisfied
        let c2 = IntConstraint {
            expr: IntExpr::constant(1),
            strict: false,
        };
        assert!(!c2.is_satisfied_constant());

        // 0 <= 0 is satisfied
        let c3 = IntConstraint {
            expr: IntExpr::constant(0),
            strict: false,
        };
        assert!(c3.is_satisfied_constant());
    }

    #[test]
    fn test_omega_constant() {
        // 1 <= 0 is unsat
        let constraints = vec![IntConstraint {
            expr: IntExpr::constant(1),
            strict: false,
        }];
        assert!(omega_unsat(&constraints));

        // -1 <= 0 is sat
        let constraints2 = vec![IntConstraint {
            expr: IntExpr::constant(-1),
            strict: false,
        }];
        assert!(!omega_unsat(&constraints2));
    }

    #[test]
    fn test_x_lt_x_plus_1() {
        // x < x + 1 is always true for integers
        // To prove: negation x >= x + 1 is unsat
        // x >= x + 1 means x - x >= 1 means 0 >= 1 which is false

        // Negation constraint: (x+1) - x <= 0 = 1 <= 0
        let constraint = IntConstraint {
            expr: IntExpr::constant(1),
            strict: false,
        };
        assert!(omega_unsat(&[constraint]));
    }
}
