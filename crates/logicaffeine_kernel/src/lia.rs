//! Linear Integer Arithmetic via Fourier-Motzkin Elimination
//!
//! This module implements a decision procedure for linear arithmetic over the
//! rationals: reify a Syntax goal into [`LinearExpr`] constraints, then decide
//! unsatisfiability with Fourier-Motzkin elimination.
//!
//! # Exactness
//!
//! Coefficients are exact arbitrary-precision rationals
//! ([`logicaffeine_base::numeric::Rational`]). The verdict feeds trusted
//! reflection reductions, so the arithmetic must be exact at every magnitude:
//! elimination multiplies coefficients pairwise, and a wrapped or declined
//! product either flips a verdict (unsound) or loses a refutation
//! (incomplete). There is no overflow path — the procedure is total.
//!
//! # Rational vs Integer Semantics
//!
//! This procedure decides satisfiability over the RATIONALS. It is sound for
//! integer goals (a rationally-unsatisfiable system has no integer solution
//! either) but incomplete for integer-specific facts — use [`crate::omega`]
//! when discreteness matters (`x > 1 ⟹ x ≥ 2`).

use std::collections::{BTreeMap, HashSet};

pub use logicaffeine_base::numeric::Rational;

use crate::reify::{extract_binary_app, extract_slit, extract_sname, extract_svar, VarInterner};
use crate::term::Term;

/// A linear expression of the form c₀ + c₁x₁ + c₂x₂ + ... + cₙxₙ.
///
/// Stored as a constant term plus a sparse map of variable coefficients.
/// Variables with coefficient 0 are automatically removed.
///
/// # Representation
///
/// The expression `3 + 2x - y` is stored as:
/// - `constant = 3`
/// - `coefficients = {0: 2, 1: -1}` (assuming x is var 0, y is var 1)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinearExpr {
    /// The constant term c₀.
    pub constant: Rational,
    /// Maps variable index to its coefficient (sparse representation).
    pub coefficients: BTreeMap<i64, Rational>,
}

impl LinearExpr {
    /// Create a constant expression
    pub fn constant(c: Rational) -> Self {
        LinearExpr {
            constant: c,
            coefficients: BTreeMap::new(),
        }
    }

    /// Create a single variable expression: 1*x_idx + 0
    pub fn var(idx: i64) -> Self {
        let mut coeffs = BTreeMap::new();
        coeffs.insert(idx, Rational::from_i64(1));
        LinearExpr {
            constant: Rational::zero(),
            coefficients: coeffs,
        }
    }

    /// Add two linear expressions
    pub fn add(&self, other: &LinearExpr) -> LinearExpr {
        let mut result = self.clone();
        result.constant = result.constant.add(&other.constant);
        for (var, coeff) in &other.coefficients {
            let entry = result
                .coefficients
                .entry(*var)
                .or_insert_with(Rational::zero);
            *entry = entry.add(coeff);
            if entry.is_zero() {
                result.coefficients.remove(var);
            }
        }
        result
    }

    /// Negate a linear expression
    pub fn neg(&self) -> LinearExpr {
        LinearExpr {
            constant: self.constant.negated(),
            coefficients: self
                .coefficients
                .iter()
                .map(|(v, c)| (*v, c.negated()))
                .collect(),
        }
    }

    /// Subtract two linear expressions
    pub fn sub(&self, other: &LinearExpr) -> LinearExpr {
        self.add(&other.neg())
    }

    /// Scale a linear expression by a rational constant
    pub fn scale(&self, c: &Rational) -> LinearExpr {
        if c.is_zero() {
            return LinearExpr::constant(Rational::zero());
        }
        LinearExpr {
            constant: self.constant.mul(c),
            coefficients: self
                .coefficients
                .iter()
                .map(|(v, coeff)| (*v, coeff.mul(c)))
                .filter(|(_, c)| !c.is_zero())
                .collect(),
        }
    }

    /// Check if this is a constant expression (no variables)
    pub fn is_constant(&self) -> bool {
        self.coefficients.is_empty()
    }

    /// Get coefficient of a variable (0 if not present)
    pub fn get_coeff(&self, var: i64) -> Rational {
        self.coefficients
            .get(&var)
            .cloned()
            .unwrap_or_else(Rational::zero)
    }
}

/// A linear constraint representing either `expr <= 0` or `expr < 0`.
///
/// All inequalities are normalized to this form during processing.
/// For example, `x >= 5` becomes `-x + 5 <= 0`, i.e., `5 - x <= 0`.
#[derive(Debug, Clone)]
pub struct Constraint {
    /// The linear expression (constraint is expr OP 0).
    pub expr: LinearExpr,
    /// If true, this is a strict inequality (`< 0`).
    /// If false, this is a non-strict inequality (`<= 0`).
    pub strict: bool,
}

impl Constraint {
    /// Check if a constant constraint is satisfied
    /// For non-constant constraints, returns true (we can't tell yet)
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
}

/// Error during reification to linear expression
#[derive(Debug)]
pub enum LiaError {
    /// Expression is not linear (e.g., x*y)
    NonLinear(String),
    /// Malformed term structure
    MalformedTerm,
    /// Goal is not an inequality
    NotInequality,
}

/// Reify a Syntax term to a linear expression.
///
/// Converts the deep embedding of terms (Syntax) into a linear expression
/// suitable for Fourier-Motzkin elimination.
///
/// # Supported Forms
///
/// - `SLit n` - Integer literal becomes a constant
/// - `SVar i` - De Bruijn variable becomes a linear variable
/// - `SName "x"` - Named global becomes a linear variable (interned)
/// - `SApp (SApp (SName "add") a) b` - Linear addition
/// - `SApp (SApp (SName "sub") a) b` - Linear subtraction
/// - `SApp (SApp (SName "mul") c) x` - Scaling (only if one operand is constant)
///
/// Every term reified for one goal (both sides of a comparison, hypotheses
/// and conclusion) must share one `vars` interner, or their variable indices
/// will not line up.
///
/// # Errors
///
/// Returns [`LiaError::NonLinear`] if the term contains non-linear operations
/// (e.g., multiplication of two variables).
pub fn reify_linear(term: &Term, vars: &mut VarInterner) -> Result<LinearExpr, LiaError> {
    // SLit n -> constant
    if let Some(n) = extract_slit(term) {
        return Ok(LinearExpr::constant(Rational::from_i64(n)));
    }

    // SVar i -> variable
    if let Some(i) = extract_svar(term) {
        return Ok(LinearExpr::var(i));
    }

    // SName "x" -> named variable (global constant treated as free variable)
    if let Some(name) = extract_sname(term) {
        return Ok(LinearExpr::var(vars.intern(&name)));
    }

    // Binary operations
    if let Some((op, a, b)) = extract_binary_app(term) {
        match op.as_str() {
            "add" => {
                let la = reify_linear(&a, vars)?;
                let lb = reify_linear(&b, vars)?;
                return Ok(la.add(&lb));
            }
            "sub" => {
                let la = reify_linear(&a, vars)?;
                let lb = reify_linear(&b, vars)?;
                return Ok(la.sub(&lb));
            }
            "mul" => {
                let la = reify_linear(&a, vars)?;
                let lb = reify_linear(&b, vars)?;
                // Only linear if one side is constant
                if la.is_constant() {
                    return Ok(lb.scale(&la.constant));
                }
                if lb.is_constant() {
                    return Ok(la.scale(&lb.constant));
                }
                return Err(LiaError::NonLinear(
                    "multiplication of two variables is not linear".to_string(),
                ));
            }
            "div" | "mod" => {
                return Err(LiaError::NonLinear(format!(
                    "operation '{}' is not supported in lia",
                    op
                )));
            }
            _ => {
                return Err(LiaError::NonLinear(format!("unknown operation '{}'", op)));
            }
        }
    }

    Err(LiaError::MalformedTerm)
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

/// Convert a goal to constraints for validity checking.
///
/// To prove a goal is valid, we check if its negation is unsatisfiable.
/// - Lt(a, b) is valid iff a - b < 0 always, i.e., negation a - b >= 0 is unsat
/// - Le(a, b) is valid iff a - b <= 0 always, i.e., negation a - b > 0 is unsat
pub fn goal_to_negated_constraint(
    rel: &str,
    lhs: &LinearExpr,
    rhs: &LinearExpr,
) -> Option<Constraint> {
    // diff = lhs - rhs
    let diff = lhs.sub(rhs);

    match rel {
        // Lt: a < b valid iff NOT(a >= b), i.e., a - b >= 0 is unsat.
        // a >= b means a - b >= 0; in constraint form (expr <= 0) that is
        // (rhs - lhs) <= 0.
        "Lt" | "lt" => Some(Constraint {
            expr: rhs.sub(lhs),
            strict: false, // <= 0
        }),
        // Le: a <= b valid iff NOT(a > b), i.e., a - b > 0 is unsat.
        // a > b means a - b > 0; in constraint form: (rhs - lhs) < 0.
        "Le" | "le" => Some(Constraint {
            expr: rhs.sub(lhs),
            strict: true, // < 0
        }),
        // Gt: a > b valid iff NOT(a <= b), i.e., a - b <= 0 is unsat.
        "Gt" | "gt" => Some(Constraint {
            expr: diff, // (lhs - rhs) <= 0
            strict: false,
        }),
        // Ge: a >= b valid iff NOT(a < b), i.e., a - b < 0 is unsat.
        "Ge" | "ge" => Some(Constraint {
            expr: diff, // (lhs - rhs) < 0
            strict: true,
        }),
        _ => None,
    }
}

/// Check if a constraint set is unsatisfiable using Fourier-Motzkin elimination.
///
/// This is the core decision procedure. It eliminates variables one by one
/// until only constant constraints remain, then checks for contradictions.
///
/// # Algorithm
///
/// For each variable x in the system:
/// 1. Partition constraints into lower bounds on x, upper bounds on x, and independent
/// 2. For each pair (lower, upper), derive a new constraint without x
/// 3. Check for immediate contradictions (e.g., `5 <= 0`)
///
/// # Returns
///
/// - `true` if the constraints are unsatisfiable (contradiction found)
/// - `false` if the constraints may be satisfiable
///
/// # Usage for Validity
///
/// To prove a goal G is valid, we check if NOT(G) is unsatisfiable.
/// If `fourier_motzkin_unsat(negation_constraints)` returns true, the goal is valid.
pub fn fourier_motzkin_unsat(constraints: &[Constraint]) -> bool {
    fourier_motzkin_unsat_within(constraints, usize::MAX).expect("an elimination without a budget always decides")
}

/// [`fourier_motzkin_unsat`] under a budget: `None` once an elimination step would leave more than `budget`
/// constraints. Each eliminated variable pairs every lower bound on it with every upper bound, so the set can square
/// at every step; a caller that may give up (the optimizer's prover) reads `None` as "not shown unsatisfiable".
pub fn fourier_motzkin_unsat_within(constraints: &[Constraint], budget: usize) -> Option<bool> {
    if constraints.is_empty() {
        return Some(false); // Empty set is satisfiable
    }

    // Collect all variables (sorted for a deterministic elimination order —
    // the verdict is order-invariant, but reproducibility is not).
    let mut vars: Vec<i64> = constraints
        .iter()
        .flat_map(|c| c.expr.coefficients.keys().copied())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    vars.sort_unstable();

    let mut current = constraints.to_vec();

    // Eliminate each variable
    for var in vars {
        current = eliminate_variable(&current, var, budget)?;

        // Early termination: check for constant contradictions
        for c in &current {
            if c.expr.is_constant() && !c.is_satisfied_constant() {
                return Some(true); // Contradiction found!
            }
        }
    }

    // Check all remaining constant constraints
    Some(current.iter().any(|c| !c.is_satisfied_constant()))
}

/// Eliminate a variable from a set of constraints using Fourier-Motzkin.
///
/// Partitions constraints into:
/// - Lower bounds: x >= expr (coeff < 0 means -|c|*x + rest <= 0 => x >= rest/|c|)
/// - Upper bounds: x <= expr (coeff > 0 means c*x + rest <= 0 => x <= -rest/c)
/// - Independent: doesn't contain variable
///
/// Combines each lower with each upper to get new constraints without the variable; `None` when that would leave
/// more than `budget` constraints.
fn eliminate_variable(constraints: &[Constraint], var: i64, budget: usize) -> Option<Vec<Constraint>> {
    let mut lower: Vec<(LinearExpr, bool)> = vec![]; // lower bound on var
    let mut upper: Vec<(LinearExpr, bool)> = vec![]; // upper bound on var
    let mut independent: Vec<Constraint> = vec![];

    for c in constraints {
        let coeff = c.expr.get_coeff(var);
        if coeff.is_zero() {
            independent.push(c.clone());
            continue;
        }
        // c.expr = coeff*var + rest  (OP) 0, with OP ∈ {<=, <}. Isolate var by
        // dividing through by `coeff`: the bound expression is `-rest/coeff`.
        // For coeff > 0 this is an UPPER bound (var <= -rest/coeff); for
        // coeff < 0 dividing flips the relation into a LOWER bound
        // (var >= -rest/coeff). The division is what makes the combined
        // `lo <= hi` constraint correct for |coeff| ≠ 1.
        let mut rest = c.expr.clone();
        rest.coefficients.remove(&var);
        let inv = coeff.recip().expect("coefficient is nonzero");
        let bound = rest.neg().scale(&inv);
        if coeff.is_positive() {
            upper.push((bound, c.strict));
        } else {
            lower.push((bound, c.strict));
        }
    }

    if independent.len().saturating_add(lower.len().saturating_mul(upper.len())) > budget {
        return None;
    }
    // lo <= var <= hi  ⟹  lo <= hi must hold (strict if either bound is strict).
    for (lo_expr, lo_strict) in &lower {
        for (hi_expr, hi_strict) in &upper {
            // In constraint form: lo - hi <= 0 (or < 0).
            let diff = lo_expr.sub(hi_expr);
            independent.push(Constraint {
                expr: diff,
                strict: *lo_strict || *hi_strict,
            });
        }
    }

    Some(independent)
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/lia/tests.rs"]
mod tests;
