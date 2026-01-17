//! Z3 Oracle integration for proof search fallback.
//!
//! When the structural backward chainer cannot derive a proof, the oracle
//! delegates to Z3, an SMT solver, for arithmetic, comparisons, and
//! uninterpreted function reasoning.
//!
//! # Architecture
//!
//! 1. Convert [`ProofExpr`] → [`VerifyExpr`](logicaffeine_verify::ir::VerifyExpr)
//! 2. Add assumptions from the knowledge base
//! 3. Ask Z3 to verify the goal
//! 4. Return [`DerivationTree`] with `OracleVerification` rule if successful
//!
//! # Limitations
//!
//! The oracle cannot reason about:
//! - Inductive constructs (`Ctor`, `Match`, `Fixpoint`, `TypedVar`)
//! - Lambda calculus (`Lambda`, `App`)
//! - Event semantics (`NeoEvent`)
//!
//! These constructs are detected by [`contains_inductive_constructs`] and cause
//! the oracle to return `Ok(None)` rather than attempting verification.
//!
//! # Feature Gate
//!
//! This module requires the `verification` feature flag:
//!
//! ```toml
//! [dependencies]
//! logicaffeine_proof = { version = "...", features = ["verification"] }
//! ```
//!
//! # Example
//!
//! ```ignore
//! use logicaffeine_proof::{ProofGoal, ProofExpr};
//! use logicaffeine_proof::oracle::try_oracle;
//!
//! let goal = ProofGoal::new(ProofExpr::Atom("P".into()));
//! let kb = vec![ProofExpr::Atom("P".into())];
//!
//! match try_oracle(&goal, &kb) {
//!     Ok(Some(tree)) => println!("Z3 verified: {}", tree),
//!     Ok(None) => println!("Z3 cannot verify"),
//!     Err(e) => println!("Error: {}", e),
//! }
//! ```

use crate::error::ProofResult;
use crate::{DerivationTree, InferenceRule, ProofExpr, ProofGoal, ProofTerm};

use logicaffeine_verify::ir::{VerifyExpr, VerifyOp, VerifyType};
use logicaffeine_verify::solver::VerificationSession;

// =============================================================================
// INDUCTIVE CONSTRUCT DETECTION
// =============================================================================

/// Check if an expression contains inductive constructs (Ctor, TypedVar, etc.)
/// that Z3 cannot handle without explicit axioms.
fn contains_inductive_constructs(expr: &ProofExpr) -> bool {
    match expr {
        // Inductive constructors - Z3 doesn't understand these
        ProofExpr::Ctor { .. } => true,
        ProofExpr::TypedVar { .. } => true,
        ProofExpr::Match { .. } => true,
        ProofExpr::Fixpoint { .. } => true,

        // Check sub-expressions
        ProofExpr::And(l, r)
        | ProofExpr::Or(l, r)
        | ProofExpr::Implies(l, r)
        | ProofExpr::Iff(l, r) => {
            contains_inductive_constructs(l) || contains_inductive_constructs(r)
        }

        ProofExpr::Not(inner) => contains_inductive_constructs(inner),

        ProofExpr::ForAll { body, .. } | ProofExpr::Exists { body, .. } => {
            contains_inductive_constructs(body)
        }

        ProofExpr::Identity(l, r) => {
            contains_inductive_constructs_term(l) || contains_inductive_constructs_term(r)
        }

        ProofExpr::Predicate { args, .. } => {
            args.iter().any(contains_inductive_constructs_term)
        }

        // Other expressions are fine
        _ => false,
    }
}

/// Check if a term contains inductive constructs.
fn contains_inductive_constructs_term(term: &ProofTerm) -> bool {
    match term {
        ProofTerm::Function(name, args) => {
            // Check for known inductive constructors
            matches!(name.as_str(), "Zero" | "Succ" | "Nil" | "Cons")
                || args.iter().any(contains_inductive_constructs_term)
        }
        ProofTerm::Variable(v) | ProofTerm::BoundVarRef(v) => {
            // Check for TypedVar pattern "name:Type"
            v.contains(':')
        }
        ProofTerm::Group(terms) => terms.iter().any(contains_inductive_constructs_term),
        ProofTerm::Constant(_) => false,
    }
}

// =============================================================================
// PUBLIC API
// =============================================================================

/// Attempt to prove a goal using Z3 as an oracle.
///
/// This is the fallback when structural backward chaining fails. Z3 will verify
/// arithmetic, comparisons, and uninterpreted function reasoning.
///
/// # Arguments
///
/// * `goal` - The proof goal to verify
/// * `knowledge_base` - Facts and rules available as assumptions
///
/// # Returns
///
/// * `Ok(Some(tree))` - Z3 verified the goal; returns a [`DerivationTree`] with
///   [`InferenceRule::OracleVerification`]
/// * `Ok(None)` - Z3 cannot verify (unknown, unsat, or unsupported constructs)
/// * `Err(e)` - Internal error during verification
///
/// # Behavior
///
/// The function performs these steps:
/// 1. Check for inductive constructs (returns `None` if found)
/// 2. Infer types for all variables in goal and KB
/// 3. Declare variables in Z3 session
/// 4. Add context and KB as assumptions
/// 5. Convert goal to [`VerifyExpr`](logicaffeine_verify::ir::VerifyExpr)
/// 6. Ask Z3 to verify
///
/// # See Also
///
/// * [`proof_expr_to_verify_expr`] - Conversion from proof to verification expressions
/// * [`BackwardChainer`](crate::BackwardChainer) - The main proof engine that calls this
pub fn try_oracle(
    goal: &ProofGoal,
    knowledge_base: &[ProofExpr],
) -> ProofResult<Option<DerivationTree>> {
    // Skip oracle for goals containing inductive constructs
    // Z3 cannot reason about Peano arithmetic without explicit axioms
    if contains_inductive_constructs(&goal.target) {
        return Ok(None);
    }

    // Also skip if KB contains inductive constructs (they would corrupt Z3 context)
    for kb_expr in knowledge_base {
        if contains_inductive_constructs(kb_expr) {
            return Ok(None);
        }
    }
    // Collect all variables and their types
    let mut session = VerificationSession::new();
    let mut types = TypeInference::new();

    // Infer types from goal
    types.infer_from_expr(&goal.target);

    // Infer types from context and KB
    for ctx_expr in &goal.context {
        types.infer_from_expr(ctx_expr);
    }
    for kb_expr in knowledge_base {
        types.infer_from_expr(kb_expr);
    }

    // Declare all inferred variables
    for (name, ty) in types.variables.iter() {
        session.declare(name, *ty);
    }

    // Add context assumptions
    for ctx_expr in &goal.context {
        if let Some(verify_expr) = proof_expr_to_verify_expr(ctx_expr) {
            session.assume(&verify_expr);
        }
    }

    // Add KB as assumptions (simplified - in full version, would be more selective)
    for kb_expr in knowledge_base {
        if let Some(verify_expr) = proof_expr_to_verify_expr(kb_expr) {
            session.assume(&verify_expr);
        }
    }

    // Convert goal to VerifyExpr
    let goal_expr = match proof_expr_to_verify_expr(&goal.target) {
        Some(e) => e,
        None => return Ok(None), // Cannot convert, oracle can't help
    };

    // Ask Z3 to verify the goal
    match session.verify(&goal_expr) {
        Ok(()) => {
            // Z3 verified it!
            let tree = DerivationTree::leaf(
                goal.target.clone(),
                InferenceRule::OracleVerification("Verified by Z3".into()),
            );
            Ok(Some(tree))
        }
        Err(_) => {
            // Z3 could not verify (either invalid or unknown)
            Ok(None)
        }
    }
}

// =============================================================================
// TYPE INFERENCE
// =============================================================================

/// Simple type inference for Z3 variable declaration.
struct TypeInference {
    variables: std::collections::HashMap<String, VerifyType>,
}

impl TypeInference {
    fn new() -> Self {
        Self {
            variables: std::collections::HashMap::new(),
        }
    }

    /// Infer types from a ProofExpr.
    fn infer_from_expr(&mut self, expr: &ProofExpr) {
        match expr {
            ProofExpr::Predicate { args, .. } => {
                for arg in args {
                    self.infer_from_term(arg, VerifyType::Int);
                }
            }

            ProofExpr::Identity(left, right) => {
                self.infer_from_term(left, VerifyType::Int);
                self.infer_from_term(right, VerifyType::Int);
            }

            ProofExpr::Atom(name) => {
                // Atoms are boolean propositions
                self.variables.insert(name.clone(), VerifyType::Bool);
            }

            ProofExpr::And(left, right)
            | ProofExpr::Or(left, right)
            | ProofExpr::Implies(left, right)
            | ProofExpr::Iff(left, right) => {
                self.infer_from_expr(left);
                self.infer_from_expr(right);
            }

            ProofExpr::Not(inner) => {
                self.infer_from_expr(inner);
            }

            ProofExpr::ForAll { body, .. } | ProofExpr::Exists { body, .. } => {
                self.infer_from_expr(body);
            }

            _ => {}
        }
    }

    /// Infer type of a term.
    fn infer_from_term(&mut self, term: &ProofTerm, context_type: VerifyType) {
        match term {
            ProofTerm::Variable(name) | ProofTerm::BoundVarRef(name) => {
                // Use context type if not already declared
                if !self.variables.contains_key(name) {
                    self.variables.insert(name.clone(), context_type);
                }
            }

            ProofTerm::Function(_, args) => {
                for arg in args {
                    self.infer_from_term(arg, VerifyType::Int);
                }
            }

            ProofTerm::Group(terms) => {
                for t in terms {
                    self.infer_from_term(t, VerifyType::Int);
                }
            }

            ProofTerm::Constant(_) => {
                // Constants don't need declaration
            }
        }
    }
}

// =============================================================================
// CONVERSION: ProofExpr → VerifyExpr
// =============================================================================

/// Convert a [`ProofExpr`] to [`VerifyExpr`](logicaffeine_verify::ir::VerifyExpr) for Z3 verification.
///
/// Transforms proof-level expressions into the verification IR that Z3 understands.
///
/// # Returns
///
/// * `Some(expr)` - Successfully converted expression
/// * `None` - Expression contains unsupported constructs
///
/// # Supported Constructs
///
/// | ProofExpr | VerifyExpr |
/// |-----------|------------|
/// | `Atom(name)` | `Var(name)` |
/// | `Predicate { Gt, [x, y] }` | `Binary(Gt, x, y)` |
/// | `And(l, r)` | `And(l, r)` |
/// | `Implies(l, r)` | `Implies(l, r)` |
/// | `ForAll { var, body }` | `ForAll([(var, Int)], body)` |
/// | `Identity(l, r)` | `Eq(l, r)` |
///
/// # Unsupported (returns `None`)
///
/// * `Lambda`, `App` - Higher-order functions
/// * `Ctor`, `Match`, `Fixpoint` - Inductive types
/// * `NeoEvent` - Event semantics
/// * `Hole`, `Term`, `Unsupported` - Meta-constructs
pub fn proof_expr_to_verify_expr(expr: &ProofExpr) -> Option<VerifyExpr> {
    match expr {
        ProofExpr::Atom(name) => Some(VerifyExpr::var(name)),

        ProofExpr::Predicate { name, args, .. } => {
            // Check for built-in comparison predicates
            if args.len() == 2 {
                let left = proof_term_to_verify_expr(&args[0])?;
                let right = proof_term_to_verify_expr(&args[1])?;

                match name.as_str() {
                    "Gt" => return Some(VerifyExpr::gt(left, right)),
                    "Lt" => return Some(VerifyExpr::lt(left, right)),
                    "Gte" => return Some(VerifyExpr::gte(left, right)),
                    "Lte" => return Some(VerifyExpr::lte(left, right)),
                    "Eq" => return Some(VerifyExpr::eq(left, right)),
                    "Neq" => return Some(VerifyExpr::neq(left, right)),
                    _ => {}
                }
            }

            // General predicate → uninterpreted function
            let verify_args: Vec<VerifyExpr> = args
                .iter()
                .filter_map(proof_term_to_verify_expr)
                .collect();
            Some(VerifyExpr::apply(name, verify_args))
        }

        ProofExpr::Identity(left, right) => {
            let l = proof_term_to_verify_expr(left)?;
            let r = proof_term_to_verify_expr(right)?;
            Some(VerifyExpr::eq(l, r))
        }

        ProofExpr::And(left, right) => {
            let l = proof_expr_to_verify_expr(left)?;
            let r = proof_expr_to_verify_expr(right)?;
            Some(VerifyExpr::and(l, r))
        }

        ProofExpr::Or(left, right) => {
            let l = proof_expr_to_verify_expr(left)?;
            let r = proof_expr_to_verify_expr(right)?;
            Some(VerifyExpr::or(l, r))
        }

        ProofExpr::Implies(left, right) => {
            let l = proof_expr_to_verify_expr(left)?;
            let r = proof_expr_to_verify_expr(right)?;
            Some(VerifyExpr::implies(l, r))
        }

        ProofExpr::Iff(left, right) => {
            // A ↔ B is (A → B) ∧ (B → A)
            let l = proof_expr_to_verify_expr(left)?;
            let r = proof_expr_to_verify_expr(right)?;
            Some(VerifyExpr::and(
                VerifyExpr::implies(l.clone(), r.clone()),
                VerifyExpr::implies(r, l),
            ))
        }

        ProofExpr::Not(inner) => {
            let i = proof_expr_to_verify_expr(inner)?;
            Some(VerifyExpr::not(i))
        }

        ProofExpr::ForAll { variable, body } => {
            let b = proof_expr_to_verify_expr(body)?;
            Some(VerifyExpr::forall(
                vec![(variable.clone(), VerifyType::Int)],
                b,
            ))
        }

        ProofExpr::Exists { variable, body } => {
            let b = proof_expr_to_verify_expr(body)?;
            Some(VerifyExpr::exists(
                vec![(variable.clone(), VerifyType::Int)],
                b,
            ))
        }

        // Modal and Temporal become uninterpreted functions
        ProofExpr::Modal { flavor, body, .. } => {
            let b = proof_expr_to_verify_expr(body)?;
            Some(VerifyExpr::apply(flavor, vec![b]))
        }

        ProofExpr::Temporal { operator, body } => {
            let b = proof_expr_to_verify_expr(body)?;
            Some(VerifyExpr::apply(operator, vec![b]))
        }

        // Inductive types - unsupported for now
        ProofExpr::Ctor { .. }
        | ProofExpr::Match { .. }
        | ProofExpr::Fixpoint { .. }
        | ProofExpr::TypedVar { .. } => None,

        // Others - not representable in Z3
        ProofExpr::Lambda { .. }
        | ProofExpr::App(_, _)
        | ProofExpr::NeoEvent { .. }
        | ProofExpr::Hole(_)
        | ProofExpr::Term(_)
        | ProofExpr::Unsupported(_) => None,
    }
}

/// Convert a [`ProofTerm`] to [`VerifyExpr`](logicaffeine_verify::ir::VerifyExpr).
///
/// Transforms proof-level terms into verification expressions for Z3.
///
/// # Conversion Rules
///
/// | ProofTerm | VerifyExpr |
/// |-----------|------------|
/// | `Constant("42")` | `Int(42)` |
/// | `Constant("foo")` | `Var("foo")` |
/// | `Variable(name)` | `Var(name)` |
/// | `BoundVarRef(name)` | `Var(name)` |
/// | `Function("Add", [x, y])` | `Binary(Add, x, y)` |
/// | `Function(name, args)` | `Apply(name, args)` |
///
/// Numeric string constants are parsed as integers; non-numeric become variables.
/// Arithmetic functions (`Add`, `Sub`, `Mul`, `Div`) are converted to binary operations.
pub fn proof_term_to_verify_expr(term: &ProofTerm) -> Option<VerifyExpr> {
    match term {
        ProofTerm::Constant(s) => {
            // Try to parse as integer
            if let Ok(n) = s.parse::<i64>() {
                Some(VerifyExpr::int(n))
            } else {
                // Non-numeric constant becomes a variable
                Some(VerifyExpr::var(s))
            }
        }

        ProofTerm::Variable(name) | ProofTerm::BoundVarRef(name) => Some(VerifyExpr::var(name)),

        ProofTerm::Function(name, args) => {
            // Check for built-in arithmetic functions
            if args.len() == 2 {
                let left = proof_term_to_verify_expr(&args[0])?;
                let right = proof_term_to_verify_expr(&args[1])?;

                match name.as_str() {
                    "Add" => {
                        return Some(VerifyExpr::binary(VerifyOp::Add, left, right))
                    }
                    "Sub" => {
                        return Some(VerifyExpr::binary(VerifyOp::Sub, left, right))
                    }
                    "Mul" => {
                        return Some(VerifyExpr::binary(VerifyOp::Mul, left, right))
                    }
                    "Div" => {
                        return Some(VerifyExpr::binary(VerifyOp::Div, left, right))
                    }
                    _ => {}
                }
            }

            // General function → Apply
            let verify_args: Vec<VerifyExpr> = args
                .iter()
                .filter_map(proof_term_to_verify_expr)
                .collect();
            Some(VerifyExpr::apply(name, verify_args))
        }

        ProofTerm::Group(terms) => {
            // Group of terms - convert each (used for tuple-like structures)
            if terms.len() == 1 {
                proof_term_to_verify_expr(&terms[0])
            } else {
                // Multi-term group not directly supported
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_atom() {
        let expr = ProofExpr::Atom("P".into());
        let result = proof_expr_to_verify_expr(&expr);
        assert!(matches!(result, Some(VerifyExpr::Var(s)) if s == "P"));
    }

    #[test]
    fn test_convert_gt_predicate() {
        let expr = ProofExpr::Predicate {
            name: "Gt".into(),
            args: vec![
                ProofTerm::Variable("x".into()),
                ProofTerm::Constant("10".into()),
            ],
            world: None,
        };
        let result = proof_expr_to_verify_expr(&expr);
        assert!(matches!(
            result,
            Some(VerifyExpr::Binary {
                op: VerifyOp::Gt,
                ..
            })
        ));
    }

    #[test]
    fn test_convert_implication() {
        let expr = ProofExpr::Implies(
            Box::new(ProofExpr::Atom("P".into())),
            Box::new(ProofExpr::Atom("Q".into())),
        );
        let result = proof_expr_to_verify_expr(&expr);
        assert!(matches!(
            result,
            Some(VerifyExpr::Binary {
                op: VerifyOp::Implies,
                ..
            })
        ));
    }

    #[test]
    fn test_convert_arithmetic_function() {
        let term = ProofTerm::Function(
            "Add".into(),
            vec![
                ProofTerm::Variable("x".into()),
                ProofTerm::Constant("5".into()),
            ],
        );
        let result = proof_term_to_verify_expr(&term);
        assert!(matches!(
            result,
            Some(VerifyExpr::Binary {
                op: VerifyOp::Add,
                ..
            })
        ));
    }
}
