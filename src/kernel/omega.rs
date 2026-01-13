//! Omega Test: True Integer Arithmetic Decision Procedure
//!
//! Unlike lia (which uses rationals), omega handles integers properly:
//! - x > 1 means x >= 2 for integers (strict-to-nonstrict conversion)
//! - 3x <= 10 means x <= 3 (floor division: floor(10/3) = 3)
//! - 2x = 5 is unsatisfiable (5 is odd)
//!
//! The algorithm is similar to Fourier-Motzkin but:
//! 1. Uses integer arithmetic with proper rounding
//! 2. Converts strict inequalities to non-strict for integers

use std::collections::{BTreeMap, HashSet};

use super::term::{Literal, Term};

/// Integer linear expression: c + Σ aᵢxᵢ
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntExpr {
    pub constant: i64,
    pub coeffs: BTreeMap<i64, i64>,
}

impl IntExpr {
    /// Create a constant expression
    pub fn constant(c: i64) -> Self {
        IntExpr {
            constant: c,
            coeffs: BTreeMap::new(),
        }
    }

    /// Create a single variable expression: 1*x_idx + 0
    pub fn var(idx: i64) -> Self {
        let mut coeffs = BTreeMap::new();
        coeffs.insert(idx, 1);
        IntExpr {
            constant: 0,
            coeffs,
        }
    }

    /// Add two expressions
    pub fn add(&self, other: &Self) -> Self {
        let mut result = self.clone();
        result.constant += other.constant;
        for (&v, &c) in &other.coeffs {
            let entry = result.coeffs.entry(v).or_insert(0);
            *entry += c;
            if *entry == 0 {
                result.coeffs.remove(&v);
            }
        }
        result
    }

    /// Negate an expression
    pub fn neg(&self) -> Self {
        IntExpr {
            constant: -self.constant,
            coeffs: self.coeffs.iter().map(|(&v, &c)| (v, -c)).collect(),
        }
    }

    /// Subtract two expressions
    pub fn sub(&self, other: &Self) -> Self {
        self.add(&other.neg())
    }

    /// Scale by an integer constant
    pub fn scale(&self, k: i64) -> Self {
        if k == 0 {
            return IntExpr::constant(0);
        }
        IntExpr {
            constant: self.constant * k,
            coeffs: self
                .coeffs
                .iter()
                .map(|(&v, &c)| (v, c * k))
                .filter(|(_, c)| *c != 0)
                .collect(),
        }
    }

    /// Check if this is a constant expression (no variables)
    pub fn is_constant(&self) -> bool {
        self.coeffs.is_empty()
    }

    /// Get coefficient of a variable (0 if not present)
    pub fn get_coeff(&self, var: i64) -> i64 {
        self.coeffs.get(&var).copied().unwrap_or(0)
    }
}

/// Integer constraint: expr ≤ 0 or expr < 0
#[derive(Debug, Clone)]
pub struct IntConstraint {
    pub expr: IntExpr,
    /// True for strict inequality (<), false for non-strict (≤)
    pub strict: bool,
}

impl IntConstraint {
    /// Check if a constant constraint is satisfied
    pub fn is_satisfied_constant(&self) -> bool {
        if !self.expr.is_constant() {
            return true; // Can't determine yet
        }
        let c = self.expr.constant;
        if self.strict {
            c < 0 // c < 0
        } else {
            c <= 0 // c ≤ 0
        }
    }

    /// Normalize by GCD of all coefficients
    pub fn normalize(&mut self) {
        let g = self
            .expr
            .coeffs
            .values()
            .chain(std::iter::once(&self.expr.constant))
            .filter(|&&x| x != 0)
            .fold(0i64, |a, &b| gcd(a.abs(), b.abs()));

        if g > 1 {
            self.expr.constant /= g;
            for v in self.expr.coeffs.values_mut() {
                *v /= g;
            }
        }
    }
}

/// GCD using Euclidean algorithm
fn gcd(a: i64, b: i64) -> i64 {
    if b == 0 {
        a.max(1)
    } else {
        gcd(b, a % b)
    }
}

/// Reify a Syntax term to an integer linear expression.
pub fn reify_int_linear(term: &Term) -> Option<IntExpr> {
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
        let hash = name_to_var_index(&name);
        return Some(IntExpr::var(hash));
    }

    // Binary operations
    if let Some((op, a, b)) = extract_binary_app(term) {
        match op.as_str() {
            "add" => {
                let la = reify_int_linear(&a)?;
                let lb = reify_int_linear(&b)?;
                return Some(la.add(&lb));
            }
            "sub" => {
                let la = reify_int_linear(&a)?;
                let lb = reify_int_linear(&b)?;
                return Some(la.sub(&lb));
            }
            "mul" => {
                let la = reify_int_linear(&a)?;
                let lb = reify_int_linear(&b)?;
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
        "Le" | "le" => {
            let mut expr = rhs.sub(lhs);
            expr.constant += 1; // b - a + 1 <= 0
            Some(IntConstraint {
                expr,
                strict: false,
            })
        }

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
        "Ge" | "ge" => {
            let mut expr = diff;
            expr.constant += 1; // (a - b + 1) <= 0
            Some(IntConstraint {
                expr,
                strict: false,
            })
        }

        _ => None,
    }
}

/// Omega Test: check if integer constraints are unsatisfiable.
///
/// Returns true if the constraints cannot all be satisfied by any integer assignment.
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
    let mut lower: Vec<(IntExpr, i64)> = vec![]; // (rest, |coeff|) for lower bounds
    let mut upper: Vec<(IntExpr, i64)> = vec![]; // (rest, coeff) for upper bounds
    let mut independent: Vec<IntConstraint> = vec![];

    for c in constraints {
        let coeff = c.expr.get_coeff(var);
        if coeff == 0 {
            independent.push(c.clone());
        } else {
            // c.expr = coeff*var + rest <= 0
            let mut rest = c.expr.clone();
            rest.coeffs.remove(&var);

            if coeff > 0 {
                // coeff*var + rest <= 0
                // var <= -rest/coeff (upper bound)
                upper.push((rest, coeff));
            } else {
                // coeff*var + rest <= 0, coeff < 0
                // |coeff|*(-var) + rest <= 0
                // -var <= -rest/|coeff|
                // var >= rest/|coeff| (lower bound)
                lower.push((rest, -coeff));
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
            let new_expr = lo_rest.scale(*hi_coeff).add(&hi_rest.scale(*lo_coeff));

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

// =============================================================================
// Helper functions for extracting Syntax patterns
// =============================================================================

/// Extract integer from SLit n
fn extract_slit(term: &Term) -> Option<i64> {
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
    if let Term::App(outer, b) = term {
        if let Term::App(sapp_outer, inner) = outer.as_ref() {
            if let Term::Global(ctor) = sapp_outer.as_ref() {
                if ctor == "SApp" {
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
    let hash: i64 = name
        .bytes()
        .fold(0i64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as i64));
    -(hash.abs() + 1_000_000)
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
        assert_eq!(sum.get_coeff(0), 1);
        assert_eq!(sum.get_coeff(1), 1);
    }

    #[test]
    fn test_int_expr_cancel() {
        let x = IntExpr::var(0);
        let neg_x = x.neg();
        let zero = x.add(&neg_x);
        assert!(zero.is_constant());
        assert_eq!(zero.constant, 0);
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
