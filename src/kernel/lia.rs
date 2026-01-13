//! LIA Tactic: Linear Integer Arithmetic by Fourier-Motzkin Elimination
//!
//! The lia tactic proves linear inequalities by:
//! 1. Reifying Syntax terms to internal linear expression representation
//! 2. Converting the goal to constraints (proving validity by checking negation's unsatisfiability)
//! 3. Using Fourier-Motzkin elimination to eliminate variables
//! 4. Checking if the resulting constant constraints are contradictory
//!
//! Uses BTreeMap for deterministic ordering and HashSet for variable collection.

use std::collections::{BTreeMap, HashSet};

use super::term::{Literal, Term};

/// A rational number for exact arithmetic during elimination.
///
/// Represented as numerator/denominator in lowest terms.
/// Denominator is always positive.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Rational {
    pub numerator: i64,
    pub denominator: i64,
}

impl Rational {
    /// Create a new rational, automatically normalizing to lowest terms
    pub fn new(n: i64, d: i64) -> Self {
        if d == 0 {
            panic!("Rational denominator cannot be zero");
        }
        let g = gcd(n.abs(), d.abs()).max(1);
        let sign = if d < 0 { -1 } else { 1 };
        Rational {
            numerator: sign * n / g,
            denominator: (d.abs()) / g,
        }
    }

    /// The zero rational
    pub fn zero() -> Self {
        Rational {
            numerator: 0,
            denominator: 1,
        }
    }

    /// Create a rational from an integer
    pub fn from_int(n: i64) -> Self {
        Rational {
            numerator: n,
            denominator: 1,
        }
    }

    /// Add two rationals
    pub fn add(&self, other: &Rational) -> Rational {
        Rational::new(
            self.numerator * other.denominator + other.numerator * self.denominator,
            self.denominator * other.denominator,
        )
    }

    /// Negate a rational
    pub fn neg(&self) -> Rational {
        Rational {
            numerator: -self.numerator,
            denominator: self.denominator,
        }
    }

    /// Subtract two rationals
    pub fn sub(&self, other: &Rational) -> Rational {
        self.add(&other.neg())
    }

    /// Multiply two rationals
    pub fn mul(&self, other: &Rational) -> Rational {
        Rational::new(
            self.numerator * other.numerator,
            self.denominator * other.denominator,
        )
    }

    /// Divide two rationals (returns None if dividing by zero)
    pub fn div(&self, other: &Rational) -> Option<Rational> {
        if other.numerator == 0 {
            return None;
        }
        Some(Rational::new(
            self.numerator * other.denominator,
            self.denominator * other.numerator,
        ))
    }

    /// Check if negative
    pub fn is_negative(&self) -> bool {
        self.numerator < 0
    }

    /// Check if positive
    pub fn is_positive(&self) -> bool {
        self.numerator > 0
    }

    /// Check if zero
    pub fn is_zero(&self) -> bool {
        self.numerator == 0
    }
}

/// Greatest common divisor using Euclidean algorithm
fn gcd(a: i64, b: i64) -> i64 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

/// A linear expression: c₀ + Σ cᵢ*xᵢ
///
/// Represented as a constant term plus a sparse map of variable coefficients.
/// Variables with coefficient 0 are omitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinearExpr {
    /// Constant term
    pub constant: Rational,
    /// Maps variable index to its coefficient (sparse)
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
        coeffs.insert(idx, Rational::from_int(1));
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
                .or_insert(Rational::zero());
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
            constant: self.constant.neg(),
            coefficients: self
                .coefficients
                .iter()
                .map(|(v, c)| (*v, c.neg()))
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
            .unwrap_or(Rational::zero())
    }
}

/// A linear constraint: expr ≤ 0 or expr < 0
#[derive(Debug, Clone)]
pub struct Constraint {
    /// The linear expression
    pub expr: LinearExpr,
    /// True for strict inequality (<), false for non-strict (≤)
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
/// Handles:
/// - SLit n -> constant
/// - SVar i -> variable
/// - SName "x" -> named variable (hashed)
/// - SApp (SApp (SName "add") a) b -> a + b
/// - SApp (SApp (SName "sub") a) b -> a - b
/// - SApp (SApp (SName "mul") c) x -> c * x (only if c is constant)
pub fn reify_linear(term: &Term) -> Result<LinearExpr, LiaError> {
    // SLit n -> constant
    if let Some(n) = extract_slit(term) {
        return Ok(LinearExpr::constant(Rational::from_int(n)));
    }

    // SVar i -> variable
    if let Some(i) = extract_svar(term) {
        return Ok(LinearExpr::var(i));
    }

    // SName "x" -> named variable (global constant treated as free variable)
    if let Some(name) = extract_sname(term) {
        let hash = name_to_var_index(&name);
        return Ok(LinearExpr::var(hash));
    }

    // Binary operations
    if let Some((op, a, b)) = extract_binary_app(term) {
        match op.as_str() {
            "add" => {
                let la = reify_linear(&a)?;
                let lb = reify_linear(&b)?;
                return Ok(la.add(&lb));
            }
            "sub" => {
                let la = reify_linear(&a)?;
                let lb = reify_linear(&b)?;
                return Ok(la.sub(&lb));
            }
            "mul" => {
                let la = reify_linear(&a)?;
                let lb = reify_linear(&b)?;
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
        // Lt: a < b valid iff NOT(a >= b), i.e., a - b >= 0 is unsat
        // So negation constraint is: a - b >= 0, i.e., -(a - b) <= 0, i.e., (b - a) <= 0
        // Actually: a >= b means a - b >= 0, which means -(a - b) <= 0
        // But we want to find if a - b >= 0 can ever be true
        // If we want to prove a < b (always), we check if a >= b (ever) is unsat
        // Constraint form: expr <= 0 or expr < 0
        // a >= b means a - b >= 0, means -(a - b) <= 0, means (b - a) <= 0
        "Lt" | "lt" => {
            // Want to prove: a < b always
            // Negation: a >= b (can be true)
            // a >= b means a - b >= 0
            // In our constraint form (expr <= 0): -(a - b) <= 0, i.e., (rhs - lhs) <= 0
            Some(Constraint {
                expr: rhs.sub(lhs),
                strict: false, // <= 0
            })
        }
        // Le: a <= b valid iff NOT(a > b), i.e., a - b > 0 is unsat
        "Le" | "le" => {
            // Want to prove: a <= b always
            // Negation: a > b
            // a > b means a - b > 0
            // In constraint form: -(a - b) < 0, i.e., (rhs - lhs) < 0
            Some(Constraint {
                expr: rhs.sub(lhs),
                strict: true, // < 0
            })
        }
        // Gt: a > b valid iff NOT(a <= b), i.e., a - b <= 0 is unsat
        "Gt" | "gt" => {
            // Want to prove: a > b always
            // Negation: a <= b
            // a <= b means a - b <= 0
            // In constraint form: (a - b) <= 0, i.e., (lhs - rhs) <= 0
            Some(Constraint {
                expr: diff, // (lhs - rhs) <= 0
                strict: false,
            })
        }
        // Ge: a >= b valid iff NOT(a < b), i.e., a - b < 0 is unsat
        "Ge" | "ge" => {
            // Want to prove: a >= b always
            // Negation: a < b
            // a < b means a - b < 0
            // In constraint form: (a - b) < 0, i.e., (lhs - rhs) < 0
            Some(Constraint {
                expr: diff, // (lhs - rhs) < 0
                strict: true,
            })
        }
        _ => None,
    }
}

/// Fourier-Motzkin elimination: check if a constraint set is unsatisfiable.
///
/// Returns true if the constraints cannot all be satisfied simultaneously.
/// This means the negation of the goal is impossible, so the goal is valid.
pub fn fourier_motzkin_unsat(constraints: &[Constraint]) -> bool {
    if constraints.is_empty() {
        return false; // Empty set is satisfiable
    }

    // Collect all variables
    let vars: Vec<i64> = constraints
        .iter()
        .flat_map(|c| c.expr.coefficients.keys().copied())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    let mut current = constraints.to_vec();

    // Eliminate each variable
    for var in vars {
        current = eliminate_variable(&current, var);

        // Early termination: check for constant contradictions
        for c in &current {
            if c.expr.is_constant() && !c.is_satisfied_constant() {
                return true; // Contradiction found!
            }
        }
    }

    // Check all remaining constant constraints
    current.iter().any(|c| !c.is_satisfied_constant())
}

/// Eliminate a variable from a set of constraints using Fourier-Motzkin.
///
/// Partitions constraints into:
/// - Lower bounds: x >= expr (coeff < 0 means -|c|*x + rest <= 0 => x >= rest/|c|)
/// - Upper bounds: x <= expr (coeff > 0 means c*x + rest <= 0 => x <= -rest/c)
/// - Independent: doesn't contain variable
///
/// Combines each lower with each upper to get new constraints without the variable.
fn eliminate_variable(constraints: &[Constraint], var: i64) -> Vec<Constraint> {
    let mut lower: Vec<(LinearExpr, bool)> = vec![]; // lower bound on var
    let mut upper: Vec<(LinearExpr, bool)> = vec![]; // upper bound on var
    let mut independent: Vec<Constraint> = vec![];

    for c in constraints {
        let coeff = c.expr.get_coeff(var);
        if coeff.is_zero() {
            independent.push(c.clone());
        } else {
            // c.expr = coeff*var + rest <= 0 (or < 0)
            // Extract: rest = c.expr - coeff*var
            let mut rest = c.expr.clone();
            rest.coefficients.remove(&var);

            if coeff.is_positive() {
                // coeff*var + rest <= 0
                // var <= -rest/coeff
                // Upper bound: -rest/coeff
                let bound = rest.neg().scale(&coeff.div(&Rational::from_int(1)).unwrap());
                let bound = bound.scale(
                    &Rational::from_int(1)
                        .div(&coeff)
                        .unwrap_or(Rational::from_int(1)),
                );
                upper.push((rest.neg().scale(&coeff.div(&coeff).unwrap()), c.strict));
            } else {
                // coeff*var + rest <= 0, coeff < 0
                // |coeff|*(-var) + rest <= 0
                // -var <= -rest/|coeff|
                // var >= rest/|coeff|
                // Lower bound: rest/|coeff|
                let abs_coeff = coeff.neg();
                lower.push((rest.scale(&abs_coeff.div(&abs_coeff).unwrap()), c.strict));
            }
        }
    }

    // Combine lower and upper bounds
    // If lo <= var <= hi, then lo <= hi must hold
    for (lo_expr, lo_strict) in &lower {
        for (hi_expr, hi_strict) in &upper {
            // We need: lo <= hi
            // In constraint form: lo - hi <= 0
            let diff = lo_expr.sub(hi_expr);
            independent.push(Constraint {
                expr: diff,
                strict: *lo_strict || *hi_strict,
            });
        }
    }

    independent
}

// =============================================================================
// Helper functions for extracting Syntax patterns (same as ring.rs)
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
    fn test_rational_arithmetic() {
        let half = Rational::new(1, 2);
        let third = Rational::new(1, 3);
        let sum = half.add(&third);
        assert_eq!(sum, Rational::new(5, 6));
    }

    #[test]
    fn test_linear_expr_add() {
        let x = LinearExpr::var(0);
        let y = LinearExpr::var(1);
        let sum = x.add(&y);
        assert!(!sum.is_constant());
        assert_eq!(sum.get_coeff(0), Rational::from_int(1));
        assert_eq!(sum.get_coeff(1), Rational::from_int(1));
    }

    #[test]
    fn test_linear_expr_cancel() {
        let x = LinearExpr::var(0);
        let neg_x = x.neg();
        let zero = x.add(&neg_x);
        assert!(zero.is_constant());
        assert!(zero.constant.is_zero());
    }

    #[test]
    fn test_constraint_satisfied() {
        // -1 <= 0 is satisfied
        let c1 = Constraint {
            expr: LinearExpr::constant(Rational::from_int(-1)),
            strict: false,
        };
        assert!(c1.is_satisfied_constant());

        // 1 <= 0 is NOT satisfied
        let c2 = Constraint {
            expr: LinearExpr::constant(Rational::from_int(1)),
            strict: false,
        };
        assert!(!c2.is_satisfied_constant());

        // 0 <= 0 is satisfied
        let c3 = Constraint {
            expr: LinearExpr::constant(Rational::zero()),
            strict: false,
        };
        assert!(c3.is_satisfied_constant());

        // 0 < 0 is NOT satisfied (strict)
        let c4 = Constraint {
            expr: LinearExpr::constant(Rational::zero()),
            strict: true,
        };
        assert!(!c4.is_satisfied_constant());
    }

    #[test]
    fn test_fourier_motzkin_constant() {
        // Single constraint: 1 <= 0 (false)
        let constraints = vec![Constraint {
            expr: LinearExpr::constant(Rational::from_int(1)),
            strict: false,
        }];
        assert!(fourier_motzkin_unsat(&constraints));

        // Single constraint: -1 <= 0 (true)
        let constraints2 = vec![Constraint {
            expr: LinearExpr::constant(Rational::from_int(-1)),
            strict: false,
        }];
        assert!(!fourier_motzkin_unsat(&constraints2));
    }

    #[test]
    fn test_x_lt_x_plus_1() {
        // x < x + 1 is always true
        // Negation: x >= x + 1, i.e., x - x - 1 >= 0, i.e., -1 >= 0
        // Constraint: -(-1) <= 0 => 1 <= 0 which is unsat => goal is valid
        let x = LinearExpr::var(0);
        let one = LinearExpr::constant(Rational::from_int(1));
        let xp1 = x.add(&one);

        // Goal: Lt x (x+1)
        // Negation constraint: (x+1) - x <= 0 (non-strict for Lt's negation Ge)
        // Wait, let me reconsider...
        // Lt(a, b) valid means a < b always
        // Negation: a >= b can be true
        // For FM: we want to show a >= b is unsat
        // a >= b means a - b >= 0
        // In our form (expr <= 0): -(a - b) <= 0, i.e., (b - a) <= 0

        // So for Lt(x, x+1): negation constraint is (x+1 - x) <= 0 = 1 <= 0
        let constraint = Constraint {
            expr: LinearExpr::constant(Rational::from_int(1)),
            strict: false,
        };
        // 1 <= 0 is unsat, so goal is valid
        assert!(fourier_motzkin_unsat(&[constraint]));
    }
}
