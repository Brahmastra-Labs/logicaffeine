//! Z3 solver wrapper for Logicaffeine verification.
//!
//! This module provides two APIs for Z3-based verification:
//!
//! ## Low-Level API: [`Verifier`] and [`VerificationContext`]
//!
//! Direct Z3 access for single-shot checks and custom verification logic.
//! Use when you need fine-grained control over the solver.
//!
//! ```ignore
//! use logicaffeine_verify::Verifier;
//!
//! let verifier = Verifier::new();
//! assert!(verifier.check_bool(true).is_ok());
//! assert!(verifier.check_int_greater_than(10, 5).is_ok());
//! ```
//!
//! ## High-Level API: [`VerificationSession`]
//!
//! Works with the [`VerifyExpr`] IR for accumulating
//! declarations and assumptions before verification. Recommended for most use cases.
//!
//! ```ignore
//! use logicaffeine_verify::{VerificationSession, VerifyExpr, VerifyType};
//!
//! let mut session = VerificationSession::new();
//! session.declare("x", VerifyType::Int);
//! session.assume(&VerifyExpr::eq(VerifyExpr::var("x"), VerifyExpr::int(10)));
//! assert!(session.verify(&VerifyExpr::gt(VerifyExpr::var("x"), VerifyExpr::int(5))).is_ok());
//! ```

use std::collections::HashMap;

use z3::ast::{Ast, Bool, Dynamic, Int};
use z3::{Config, Context, FuncDecl, SatResult, Solver, Sort};

use crate::error::{CounterExample, VerificationError, VerificationResult};
use crate::ir::{VerifyExpr, VerifyOp, VerifyType};

/// Low-level Z3-based verifier for single-shot validity checks.
///
/// The verifier uses a 10-second timeout by default. For more complex
/// proofs with multiple constraints, use [`VerificationSession`] instead.
///
/// # Examples
///
/// ```ignore
/// use logicaffeine_verify::Verifier;
///
/// let verifier = Verifier::new();
///
/// // Boolean validity
/// assert!(verifier.check_bool(true).is_ok());
/// assert!(verifier.check_bool(false).is_err());
///
/// // Integer bounds
/// assert!(verifier.check_int_greater_than(10, 5).is_ok());
/// ```
pub struct Verifier {
    cfg: Config,
}

impl Verifier {
    /// Create a new verifier with a 10-second timeout.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use logicaffeine_verify::Verifier;
    ///
    /// let verifier = Verifier::new();
    /// ```
    pub fn new() -> Self {
        let mut cfg = Config::new();
        cfg.set_param_value("timeout", "10000");
        Self { cfg }
    }

    /// Check if a boolean value is valid (always true).
    ///
    /// Returns `Ok(())` if the value is `true`, an error otherwise.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use logicaffeine_verify::Verifier;
    ///
    /// let verifier = Verifier::new();
    /// assert!(verifier.check_bool(true).is_ok());
    /// assert!(verifier.check_bool(false).is_err());
    /// ```
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

    /// Verify that `value > bound`.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use logicaffeine_verify::Verifier;
    ///
    /// let verifier = Verifier::new();
    /// assert!(verifier.check_int_greater_than(10, 5).is_ok());  // 10 > 5
    /// assert!(verifier.check_int_greater_than(3, 5).is_err());  // 3 > 5 is false
    /// ```
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

    /// Verify that `value < bound`.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use logicaffeine_verify::Verifier;
    ///
    /// let verifier = Verifier::new();
    /// assert!(verifier.check_int_less_than(3, 5).is_ok());   // 3 < 5
    /// assert!(verifier.check_int_less_than(10, 5).is_err()); // 10 < 5 is false
    /// ```
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

    /// Verify that `left == right`.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use logicaffeine_verify::Verifier;
    ///
    /// let verifier = Verifier::new();
    /// assert!(verifier.check_int_equals(42, 42).is_ok());
    /// assert!(verifier.check_int_equals(1, 2).is_err());
    /// ```
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
    ///
    /// Use this when you need to build custom verification logic with
    /// multiple variables and constraints.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use logicaffeine_verify::Verifier;
    /// use z3::ast::Bool;
    ///
    /// let verifier = Verifier::new();
    /// let ctx = verifier.context();
    /// let solver = ctx.solver();
    ///
    /// // P ∨ ¬P is a tautology
    /// let p = ctx.bool_var("p");
    /// let tautology = Bool::or(ctx.z3_context(), &[&p, &p.not()]);
    /// assert!(ctx.check_valid(&solver, &tautology).is_ok());
    /// ```
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

/// A verification context for building constraints incrementally.
///
/// Provides direct access to Z3 types for constructing custom proofs.
/// For most use cases, prefer [`VerificationSession`] which works with
/// the higher-level [`VerifyExpr`] IR.
pub struct VerificationContext {
    ctx: Context,
}

impl VerificationContext {
    fn new(ctx: Context) -> Self {
        Self { ctx }
    }

    /// Get the underlying Z3 context.
    ///
    /// Use this when you need to call Z3 functions that require a context reference.
    pub fn z3_context(&self) -> &Context {
        &self.ctx
    }

    /// Create a new solver for this context.
    ///
    /// The solver accumulates assertions and can check their satisfiability.
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
    ///
    /// Uses the standard validity check: P is valid iff ¬P is unsatisfiable.
    /// The solver state is preserved using push/pop.
    pub fn check_valid(&self, solver: &Solver, assertion: &Bool) -> VerificationResult {
        solver.push();
        solver.assert(&assertion.not());

        let result = match solver.check() {
            SatResult::Unsat => Ok(()),
            SatResult::Sat => {
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

// ============================================================
// High-Level Session API
// ============================================================

/// A verification session for working with the Verification IR.
///
/// A session accumulates variable declarations and assumptions,
/// then verifies assertions against that context. This is the recommended
/// API for most verification tasks.
///
/// Each verification call creates a fresh Z3 context to avoid lifetime issues.
///
/// # Examples
///
/// ```ignore
/// use logicaffeine_verify::{VerificationSession, VerifyExpr, VerifyType};
///
/// let mut session = VerificationSession::new();
///
/// // Declare variables
/// session.declare("x", VerifyType::Int);
///
/// // Add assumptions
/// session.assume(&VerifyExpr::gt(VerifyExpr::var("x"), VerifyExpr::int(0)));
///
/// // Verify assertions
/// let result = session.verify(&VerifyExpr::gte(VerifyExpr::var("x"), VerifyExpr::int(0)));
/// assert!(result.is_ok());
/// ```
///
/// # Modus Ponens Example
///
/// ```ignore
/// use logicaffeine_verify::{VerificationSession, VerifyExpr, VerifyType};
///
/// let mut session = VerificationSession::new();
/// session.declare("x", VerifyType::Object);
///
/// // All mortals are human
/// session.assume(&VerifyExpr::implies(
///     VerifyExpr::apply("Mortal", vec![VerifyExpr::var("x")]),
///     VerifyExpr::apply("Human", vec![VerifyExpr::var("x")]),
/// ));
///
/// // x is mortal
/// session.assume(&VerifyExpr::apply("Mortal", vec![VerifyExpr::var("x")]));
///
/// // Therefore x is human
/// assert!(session.verify(&VerifyExpr::apply("Human", vec![VerifyExpr::var("x")])).is_ok());
/// ```
pub struct VerificationSession {
    vars: HashMap<String, VerifyType>,
    assumptions: Vec<VerifyExpr>,
}

impl VerificationSession {
    /// Create a new verification session.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use logicaffeine_verify::VerificationSession;
    ///
    /// let session = VerificationSession::new();
    /// ```
    pub fn new() -> Self {
        Self {
            vars: HashMap::new(),
            assumptions: Vec::new(),
        }
    }

    /// Declare a variable with a type.
    ///
    /// Variables must be declared before they can be used in assumptions
    /// or verifications.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use logicaffeine_verify::{VerificationSession, VerifyType};
    ///
    /// let mut session = VerificationSession::new();
    /// session.declare("x", VerifyType::Int);
    /// session.declare("p", VerifyType::Bool);
    /// session.declare("socrates", VerifyType::Object);
    /// ```
    pub fn declare(&mut self, name: &str, ty: VerifyType) {
        self.vars.insert(name.to_string(), ty);
    }

    /// Add an assumption (constraint) to the session.
    ///
    /// Assumptions constrain the verification context. Subsequent calls to
    /// [`verify`](Self::verify) will check validity under all assumptions.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use logicaffeine_verify::{VerificationSession, VerifyExpr, VerifyType};
    ///
    /// let mut session = VerificationSession::new();
    /// session.declare("x", VerifyType::Int);
    ///
    /// // Assume x = 10
    /// session.assume(&VerifyExpr::eq(VerifyExpr::var("x"), VerifyExpr::int(10)));
    ///
    /// // Assume x > 0
    /// session.assume(&VerifyExpr::gt(VerifyExpr::var("x"), VerifyExpr::int(0)));
    /// ```
    pub fn assume(&mut self, expr: &VerifyExpr) {
        self.assumptions.push(expr.clone());
    }

    /// Verify a predicate with a temporary variable binding.
    ///
    /// Used for refinement type checking. Creates a scoped context where
    /// `var_name = value` is assumed, then verifies that `predicate` holds.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use logicaffeine_verify::{VerificationSession, VerifyExpr, VerifyType};
    ///
    /// let session = VerificationSession::new();
    ///
    /// // Check that 10 satisfies the predicate x > 5
    /// let result = session.verify_with_binding(
    ///     "x",
    ///     VerifyType::Int,
    ///     &VerifyExpr::int(10),
    ///     &VerifyExpr::gt(VerifyExpr::var("x"), VerifyExpr::int(5)),
    /// );
    /// assert!(result.is_ok());
    /// ```
    pub fn verify_with_binding(
        &self,
        var_name: &str,
        var_type: VerifyType,
        value: &VerifyExpr,
        predicate: &VerifyExpr,
    ) -> VerificationResult {
        // Create a fresh Z3 context
        let mut cfg = Config::new();
        cfg.set_param_value("timeout", "10000");
        let ctx = Context::new(&cfg);
        let solver = Solver::new(&ctx);

        // Copy existing vars and add the bound variable
        let mut vars = self.vars.clone();
        vars.insert(var_name.to_string(), var_type);

        let encoder = Encoder::new(&ctx, &vars);

        // Add all existing assumptions
        for assumption in &self.assumptions {
            let ast = encoder.encode(assumption);
            if let Some(b) = ast.as_bool() {
                solver.assert(&b);
            }
        }

        // Add the binding: var_name == value
        let binding = VerifyExpr::eq(
            VerifyExpr::var(var_name),
            value.clone(),
        );
        let binding_ast = encoder.encode(&binding);
        if let Some(b) = binding_ast.as_bool() {
            solver.assert(&b);
        }

        // Verify the predicate
        let pred_ast = encoder.encode(predicate);
        let assertion = pred_ast.as_bool().ok_or_else(|| {
            VerificationError::solver_error("Refinement predicate must be boolean")
        })?;

        solver.push();
        solver.assert(&assertion.not());

        let result = match solver.check() {
            SatResult::Unsat => Ok(()),
            SatResult::Sat => Err(VerificationError::refinement_violation(
                var_name,
                "The value does not satisfy the refinement predicate.",
            )),
            SatResult::Unknown => Err(VerificationError::solver_unknown()),
        };

        solver.pop(1);
        result
    }

    /// Verify that an assertion is valid given current assumptions.
    ///
    /// Uses the standard validity check: P is valid iff ¬P is unsatisfiable.
    /// Returns `Ok(())` if the assertion can be proven, an error otherwise.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use logicaffeine_verify::{VerificationSession, VerifyExpr, VerifyType};
    ///
    /// let mut session = VerificationSession::new();
    /// session.declare("x", VerifyType::Int);
    /// session.assume(&VerifyExpr::eq(VerifyExpr::var("x"), VerifyExpr::int(10)));
    ///
    /// // This should pass: 10 > 5
    /// assert!(session.verify(&VerifyExpr::gt(VerifyExpr::var("x"), VerifyExpr::int(5))).is_ok());
    ///
    /// // This should fail: 10 < 5
    /// assert!(session.verify(&VerifyExpr::lt(VerifyExpr::var("x"), VerifyExpr::int(5))).is_err());
    /// ```
    pub fn verify(&self, expr: &VerifyExpr) -> VerificationResult {
        // Create a fresh Z3 context for this verification
        let mut cfg = Config::new();
        cfg.set_param_value("timeout", "10000");
        let ctx = Context::new(&cfg);
        let solver = Solver::new(&ctx);

        // Create an encoder for this context
        let encoder = Encoder::new(&ctx, &self.vars);

        // Add all assumptions
        for assumption in &self.assumptions {
            let ast = encoder.encode(assumption);
            if let Some(b) = ast.as_bool() {
                solver.assert(&b);
            }
        }

        // Encode the assertion we want to verify
        let ast = encoder.encode(expr);
        let assertion = ast.as_bool().ok_or_else(|| {
            VerificationError::solver_error("Assertion must be boolean")
        })?;

        // To prove P is valid: check if NOT(P) is UNSAT
        solver.push();
        solver.assert(&assertion.not());

        let result = match solver.check() {
            SatResult::Unsat => Ok(()),
            SatResult::Sat => {
                Err(VerificationError::contradiction(
                    "Assertion cannot be proven valid",
                    None,
                ))
            }
            SatResult::Unknown => Err(VerificationError::solver_unknown()),
        };

        solver.pop(1);
        result
    }
}

impl Default for VerificationSession {
    fn default() -> Self {
        Self::new()
    }
}

/// Internal encoder that converts VerifyExpr to Z3 AST.
struct Encoder<'ctx> {
    ctx: &'ctx Context,
    vars: &'ctx HashMap<String, VerifyType>,
}

impl<'ctx> Encoder<'ctx> {
    fn new(ctx: &'ctx Context, vars: &'ctx HashMap<String, VerifyType>) -> Self {
        Self { ctx, vars }
    }

    fn encode(&self, expr: &VerifyExpr) -> Dynamic<'ctx> {
        match expr {
            VerifyExpr::Int(n) => Dynamic::from_ast(&Int::from_i64(self.ctx, *n)),
            VerifyExpr::Bool(b) => Dynamic::from_ast(&Bool::from_bool(self.ctx, *b)),

            VerifyExpr::Var(name) => {
                let ty = self.vars.get(name).copied().unwrap_or(VerifyType::Int);
                match ty {
                    VerifyType::Int => Dynamic::from_ast(&Int::new_const(self.ctx, name.as_str())),
                    VerifyType::Bool => Dynamic::from_ast(&Bool::new_const(self.ctx, name.as_str())),
                    VerifyType::Object => {
                        // For Object types, use Int as a placeholder
                        Dynamic::from_ast(&Int::new_const(self.ctx, name.as_str()))
                    }
                }
            }

            VerifyExpr::Binary { op, left, right } => {
                let l = self.encode(left);
                let r = self.encode(right);
                self.encode_binary(op, l, r)
            }

            VerifyExpr::Not(inner) => {
                let i = self.encode(inner);
                if let Some(b) = i.as_bool() {
                    Dynamic::from_ast(&b.not())
                } else {
                    i
                }
            }

            VerifyExpr::Apply { name, args } => {
                self.encode_apply(name, args)
            }

            VerifyExpr::ForAll { vars: _, body } => {
                // Simplified: just encode the body
                self.encode(body)
            }

            VerifyExpr::Exists { vars: _, body } => {
                self.encode(body)
            }
        }
    }

    fn encode_binary(&self, op: &VerifyOp, l: Dynamic<'ctx>, r: Dynamic<'ctx>) -> Dynamic<'ctx> {
        match op {
            // Arithmetic
            VerifyOp::Add => {
                if let (Some(li), Some(ri)) = (l.as_int(), r.as_int()) {
                    Dynamic::from_ast(&(li + ri))
                } else {
                    l
                }
            }
            VerifyOp::Sub => {
                if let (Some(li), Some(ri)) = (l.as_int(), r.as_int()) {
                    Dynamic::from_ast(&(li - ri))
                } else {
                    l
                }
            }
            VerifyOp::Mul => {
                if let (Some(li), Some(ri)) = (l.as_int(), r.as_int()) {
                    Dynamic::from_ast(&(li * ri))
                } else {
                    l
                }
            }
            VerifyOp::Div => {
                if let (Some(li), Some(ri)) = (l.as_int(), r.as_int()) {
                    Dynamic::from_ast(&(li / ri))
                } else {
                    l
                }
            }

            // Comparison
            VerifyOp::Gt => {
                if let (Some(li), Some(ri)) = (l.as_int(), r.as_int()) {
                    Dynamic::from_ast(&li.gt(&ri))
                } else {
                    Dynamic::from_ast(&Bool::from_bool(self.ctx, false))
                }
            }
            VerifyOp::Lt => {
                if let (Some(li), Some(ri)) = (l.as_int(), r.as_int()) {
                    Dynamic::from_ast(&li.lt(&ri))
                } else {
                    Dynamic::from_ast(&Bool::from_bool(self.ctx, false))
                }
            }
            VerifyOp::Gte => {
                if let (Some(li), Some(ri)) = (l.as_int(), r.as_int()) {
                    Dynamic::from_ast(&li.ge(&ri))
                } else {
                    Dynamic::from_ast(&Bool::from_bool(self.ctx, false))
                }
            }
            VerifyOp::Lte => {
                if let (Some(li), Some(ri)) = (l.as_int(), r.as_int()) {
                    Dynamic::from_ast(&li.le(&ri))
                } else {
                    Dynamic::from_ast(&Bool::from_bool(self.ctx, false))
                }
            }

            // Equality
            VerifyOp::Eq => Dynamic::from_ast(&l._eq(&r)),
            VerifyOp::Neq => Dynamic::from_ast(&l._eq(&r).not()),

            // Logic
            VerifyOp::And => {
                if let (Some(lb), Some(rb)) = (l.as_bool(), r.as_bool()) {
                    Dynamic::from_ast(&Bool::and(self.ctx, &[&lb, &rb]))
                } else {
                    Dynamic::from_ast(&Bool::from_bool(self.ctx, false))
                }
            }
            VerifyOp::Or => {
                if let (Some(lb), Some(rb)) = (l.as_bool(), r.as_bool()) {
                    Dynamic::from_ast(&Bool::or(self.ctx, &[&lb, &rb]))
                } else {
                    Dynamic::from_ast(&Bool::from_bool(self.ctx, false))
                }
            }
            VerifyOp::Implies => {
                if let (Some(lb), Some(rb)) = (l.as_bool(), r.as_bool()) {
                    Dynamic::from_ast(&lb.implies(&rb))
                } else {
                    Dynamic::from_ast(&Bool::from_bool(self.ctx, true))
                }
            }
        }
    }

    fn encode_apply(&self, name: &str, args: &[VerifyExpr]) -> Dynamic<'ctx> {
        let int_sort = Sort::int(self.ctx);
        let domain: Vec<&Sort> = args.iter().map(|_| &int_sort).collect();
        let range = Sort::bool(self.ctx);

        let func_decl = FuncDecl::new(self.ctx, name, &domain, &range);

        let encoded_args: Vec<Dynamic> = args.iter().map(|a| self.encode(a)).collect();
        let arg_refs: Vec<&dyn Ast> = encoded_args.iter().map(|a| a as &dyn Ast).collect();

        Dynamic::from_ast(&func_decl.apply(&arg_refs))
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

    // ============================================================
    // VerificationSession Tests
    // ============================================================

    #[test]
    fn test_session_integer_bounds() {
        let mut session = VerificationSession::new();

        // Declare x as Int
        session.declare("x", VerifyType::Int);

        // Assume: x = 10
        session.assume(&VerifyExpr::eq(
            VerifyExpr::var("x"),
            VerifyExpr::int(10),
        ));

        // Verify: x > 5 (should pass)
        let result = session.verify(&VerifyExpr::gt(
            VerifyExpr::var("x"),
            VerifyExpr::int(5),
        ));
        assert!(result.is_ok(), "10 > 5 should be provable");
    }

    #[test]
    fn test_session_integer_contradiction() {
        let mut session = VerificationSession::new();

        // Declare x as Int
        session.declare("x", VerifyType::Int);

        // Assume: x = 10
        session.assume(&VerifyExpr::eq(
            VerifyExpr::var("x"),
            VerifyExpr::int(10),
        ));

        // Verify: x < 5 (should FAIL)
        let result = session.verify(&VerifyExpr::lt(
            VerifyExpr::var("x"),
            VerifyExpr::int(5),
        ));
        assert!(result.is_err(), "10 < 5 should not be provable");
    }

    #[test]
    fn test_session_uninterpreted_functions() {
        let mut session = VerificationSession::new();

        // Declare x as Object
        session.declare("x", VerifyType::Object);

        // Assume: Mortal(x) -> Human(x)
        session.assume(&VerifyExpr::implies(
            VerifyExpr::apply("Mortal", vec![VerifyExpr::var("x")]),
            VerifyExpr::apply("Human", vec![VerifyExpr::var("x")]),
        ));

        // Assume: Mortal(x)
        session.assume(&VerifyExpr::apply("Mortal", vec![VerifyExpr::var("x")]));

        // Verify: Human(x) - Z3 should deduce this structurally
        let result = session.verify(&VerifyExpr::apply("Human", vec![VerifyExpr::var("x")]));
        assert!(result.is_ok(), "Should deduce Human(x) from Mortal(x) and Mortal(x)->Human(x)");
    }

    #[test]
    fn test_session_modal_structural_reasoning() {
        let mut session = VerificationSession::new();

        // Declare A and B as Objects (representing propositions)
        session.declare("A", VerifyType::Object);
        session.declare("B", VerifyType::Object);

        // Assume: Possible(A) -> Possible(B)
        session.assume(&VerifyExpr::implies(
            VerifyExpr::apply("Possible", vec![VerifyExpr::var("A")]),
            VerifyExpr::apply("Possible", vec![VerifyExpr::var("B")]),
        ));

        // Assume: Possible(A)
        session.assume(&VerifyExpr::apply("Possible", vec![VerifyExpr::var("A")]));

        // Verify: Possible(B)
        let result = session.verify(&VerifyExpr::apply("Possible", vec![VerifyExpr::var("B")]));
        assert!(result.is_ok(), "Should deduce Possible(B) from modus ponens");
    }

    #[test]
    fn test_session_arithmetic() {
        let mut session = VerificationSession::new();

        // Declare x and y
        session.declare("x", VerifyType::Int);
        session.declare("y", VerifyType::Int);

        // Assume: x = 5, y = 3
        session.assume(&VerifyExpr::eq(VerifyExpr::var("x"), VerifyExpr::int(5)));
        session.assume(&VerifyExpr::eq(VerifyExpr::var("y"), VerifyExpr::int(3)));

        // Verify: x + y > 7 (5 + 3 = 8 > 7)
        let sum = VerifyExpr::binary(
            VerifyOp::Add,
            VerifyExpr::var("x"),
            VerifyExpr::var("y"),
        );
        let result = session.verify(&VerifyExpr::gt(sum, VerifyExpr::int(7)));
        assert!(result.is_ok(), "5 + 3 > 7 should be provable");
    }

    #[test]
    fn test_session_logic_and_or() {
        let mut session = VerificationSession::new();

        // Declare p and q as Bool
        session.declare("p", VerifyType::Bool);
        session.declare("q", VerifyType::Bool);

        // Assume: p = true, q = false
        session.assume(&VerifyExpr::eq(VerifyExpr::var("p"), VerifyExpr::bool(true)));
        session.assume(&VerifyExpr::eq(VerifyExpr::var("q"), VerifyExpr::bool(false)));

        // Verify: p || q (true || false = true)
        let result = session.verify(&VerifyExpr::or(
            VerifyExpr::var("p"),
            VerifyExpr::var("q"),
        ));
        assert!(result.is_ok(), "true || false should be provable");

        // Verify: !(p && q) (!(true && false) = true)
        let result = session.verify(&VerifyExpr::not(VerifyExpr::and(
            VerifyExpr::var("p"),
            VerifyExpr::var("q"),
        )));
        assert!(result.is_ok(), "!(true && false) should be provable");
    }
}
