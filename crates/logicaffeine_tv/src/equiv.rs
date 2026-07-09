//! The SMT equivalence primitive, built on `logicaffeine_verify::check_equivalence`.

use logicaffeine_verify::{check_equivalence, EquivalenceResult, VerifyExpr};

/// Prove that a `Bool`-sorted `VerifyExpr` is valid (true under every assignment of its
/// free variables).
///
/// Implemented as an equivalence check against the constant `true`: the backend asserts
/// `¬(pred ↔ true)` and asks Z3 for a model. `Unsat` ⇒ the predicate is valid
/// ([`EquivalenceResult::Equivalent`]); `Sat` ⇒ a counterexample assignment
/// ([`EquivalenceResult::NotEquivalent`]).
pub fn prove_valid(pred: &VerifyExpr) -> EquivalenceResult {
    check_equivalence(pred, &VerifyExpr::bool(true), &[], 1)
}

/// True iff [`prove_valid`] proves the predicate.
pub fn is_valid(pred: &VerifyExpr) -> bool {
    matches!(prove_valid(pred), EquivalenceResult::Equivalent)
}
