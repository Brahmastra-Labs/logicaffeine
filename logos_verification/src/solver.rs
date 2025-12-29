//! Z3 solver wrapper for LOGOS verification.

use z3::ast::{Ast, Bool};
use z3::{Config, Context, SatResult, Solver};

use crate::error::{CounterExample, VerificationError, VerificationResult};

/// The Z3-based verifier.
pub struct Verifier {
    cfg: Config,
}

impl Verifier {
    /// Create a new verifier.
    pub fn new() -> Self {
        let mut cfg = Config::new();
        // Set a reasonable timeout (10 seconds)
        cfg.set_param_value("timeout", "10000");
        Self { cfg }
    }

    /// Check if a boolean value is valid (i.e., always true).
    ///
    /// This is the most basic verification: `true` is valid, `false` is not.
    pub fn check_bool(&self, value: bool) -> VerificationResult {
        let ctx = Context::new(&self.cfg);
        let solver = Solver::new(&ctx);

        let assertion = Bool::from_bool(&ctx, value);

        // To prove P is valid: check if NOT(P) is UNSAT
        // If NOT(P) is unsatisfiable, then P is always true
        solver.assert(&assertion.not());

        match solver.check() {
            SatResult::Unsat => Ok(()), // NOT(P) is impossible -> P is valid
            SatResult::Sat => {
                // NOT(P) is satisfiable -> P is not always true
                Err(VerificationError::contradiction(
                    "The assertion is not always true.",
                    None,
                ))
            }
            SatResult::Unknown => Err(VerificationError::solver_unknown()),
        }
    }

    /// Verify that an integer variable satisfies a constraint.
    ///
    /// Given a value and a bound, checks if `value > bound` or `value < bound` etc.
    pub fn check_int_greater_than(&self, value: i64, bound: i64) -> VerificationResult {
        let ctx = Context::new(&self.cfg);
        let solver = Solver::new(&ctx);

        let v = z3::ast::Int::from_i64(&ctx, value);
        let b = z3::ast::Int::from_i64(&ctx, bound);
        let assertion = v.gt(&b);

        // To prove P is valid: check if NOT(P) is UNSAT
        solver.assert(&assertion.not());

        match solver.check() {
            SatResult::Unsat => Ok(()),
            SatResult::Sat => {
                Err(VerificationError::bounds_violation(
                    "value",
                    format!("> {}", bound),
                    format!("{}", value),
                ))
            }
            SatResult::Unknown => Err(VerificationError::solver_unknown()),
        }
    }

    /// Verify that an integer variable satisfies a constraint.
    pub fn check_int_less_than(&self, value: i64, bound: i64) -> VerificationResult {
        let ctx = Context::new(&self.cfg);
        let solver = Solver::new(&ctx);

        let v = z3::ast::Int::from_i64(&ctx, value);
        let b = z3::ast::Int::from_i64(&ctx, bound);
        let assertion = v.lt(&b);

        solver.assert(&assertion.not());

        match solver.check() {
            SatResult::Unsat => Ok(()),
            SatResult::Sat => Err(VerificationError::bounds_violation(
                "value",
                format!("< {}", bound),
                format!("{}", value),
            )),
            SatResult::Unknown => Err(VerificationError::solver_unknown()),
        }
    }

    /// Verify that two integer values are equal.
    pub fn check_int_equals(&self, left: i64, right: i64) -> VerificationResult {
        let ctx = Context::new(&self.cfg);
        let solver = Solver::new(&ctx);

        let l = z3::ast::Int::from_i64(&ctx, left);
        let r = z3::ast::Int::from_i64(&ctx, right);
        let assertion = l._eq(&r);

        solver.assert(&assertion.not());

        match solver.check() {
            SatResult::Unsat => Ok(()),
            SatResult::Sat => Err(VerificationError::contradiction(
                format!("{} is not equal to {}", left, right),
                Some(CounterExample {
                    assignments: vec![
                        ("left".to_string(), format!("{}", left)),
                        ("right".to_string(), format!("{}", right)),
                    ],
                }),
            )),
            SatResult::Unknown => Err(VerificationError::solver_unknown()),
        }
    }

    /// Create a verification context for more complex proofs.
    pub fn context(&self) -> VerificationContext {
        let ctx = Context::new(&self.cfg);
        VerificationContext::new(ctx)
    }
}

impl Default for Verifier {
    fn default() -> Self {
        Self::new()
    }
}

/// A verification context for building up constraints incrementally.
pub struct VerificationContext {
    ctx: Context,
}

impl VerificationContext {
    fn new(ctx: Context) -> Self {
        Self { ctx }
    }

    /// Get the underlying Z3 context.
    pub fn z3_context(&self) -> &Context {
        &self.ctx
    }

    /// Create a new solver for this context.
    pub fn solver(&self) -> Solver {
        Solver::new(&self.ctx)
    }

    /// Create a boolean constant.
    pub fn bool_val(&self, value: bool) -> Bool {
        Bool::from_bool(&self.ctx, value)
    }

    /// Create an integer constant.
    pub fn int_val(&self, value: i64) -> z3::ast::Int {
        z3::ast::Int::from_i64(&self.ctx, value)
    }

    /// Create a named boolean variable.
    pub fn bool_var(&self, name: &str) -> Bool {
        Bool::new_const(&self.ctx, name)
    }

    /// Create a named integer variable.
    pub fn int_var(&self, name: &str) -> z3::ast::Int {
        z3::ast::Int::new_const(&self.ctx, name)
    }

    /// Check if an assertion is valid (always true).
    pub fn check_valid(&self, solver: &Solver, assertion: &Bool) -> VerificationResult {
        solver.push();
        solver.assert(&assertion.not());

        let result = match solver.check() {
            SatResult::Unsat => Ok(()),
            SatResult::Sat => {
                // Counter-example extraction will be implemented in Phase 2
                // when we have variable tracking
                Err(VerificationError::contradiction(
                    "Assertion is not valid",
                    None,
                ))
            }
            SatResult::Unknown => Err(VerificationError::solver_unknown()),
        };

        solver.pop(1);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tautology() {
        let verifier = Verifier::new();
        assert!(verifier.check_bool(true).is_ok());
    }

    #[test]
    fn test_contradiction() {
        let verifier = Verifier::new();
        assert!(verifier.check_bool(false).is_err());
    }

    #[test]
    fn test_int_greater_than_valid() {
        let verifier = Verifier::new();
        assert!(verifier.check_int_greater_than(10, 5).is_ok());
    }

    #[test]
    fn test_int_greater_than_invalid() {
        let verifier = Verifier::new();
        assert!(verifier.check_int_greater_than(3, 5).is_err());
    }

    #[test]
    fn test_int_equals_valid() {
        let verifier = Verifier::new();
        assert!(verifier.check_int_equals(42, 42).is_ok());
    }

    #[test]
    fn test_int_equals_invalid() {
        let verifier = Verifier::new();
        assert!(verifier.check_int_equals(1, 2).is_err());
    }

    #[test]
    fn test_context_api() {
        let verifier = Verifier::new();
        let vctx = verifier.context();
        let solver = vctx.solver();

        // P ∨ ¬P is a tautology
        let p = vctx.bool_var("p");
        let tautology = Bool::or(vctx.z3_context(), &[&p, &p.not()]);

        assert!(vctx.check_valid(&solver, &tautology).is_ok());
    }

    #[test]
    fn test_context_contradiction() {
        let verifier = Verifier::new();
        let vctx = verifier.context();
        let solver = vctx.solver();

        // P ∧ ¬P is a contradiction (not valid)
        let p = vctx.bool_var("p");
        let contradiction = Bool::and(vctx.z3_context(), &[&p, &p.not()]);

        assert!(vctx.check_valid(&solver, &contradiction).is_err());
    }
}
