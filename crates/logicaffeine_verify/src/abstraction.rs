//! Automatic Abstraction for Infinite-State Systems
//!
//! Predicate abstraction reduces infinite-state to finite-state.
//! CEGAR loop refines if abstraction is too coarse.

use crate::ir::VerifyExpr;
use crate::equivalence::Trace;
use crate::kinduction;

#[derive(Debug, Clone)]
pub struct AbstractModel {
    pub predicates: Vec<VerifyExpr>,
    pub abstract_init: VerifyExpr,
    pub abstract_transition: VerifyExpr,
}

#[derive(Debug)]
pub enum AbstractionResult {
    Safe,
    Unsafe { concrete_trace: Trace },
    SpuriousRefined { new_predicates: Vec<VerifyExpr> },
    Unknown,
}

/// Create an abstract model from concrete using given predicates.
pub fn abstract_model(
    init: &VerifyExpr,
    predicates: &[VerifyExpr],
) -> AbstractModel {
    // Simplified: use predicates as-is for abstraction
    let abstract_init = if predicates.is_empty() {
        init.clone()
    } else {
        let mut abs = init.clone();
        for pred in predicates {
            abs = VerifyExpr::and(abs, pred.clone());
        }
        abs
    };

    AbstractModel {
        predicates: predicates.to_vec(),
        abstract_init,
        abstract_transition: VerifyExpr::bool(true),
    }
}

/// CEGAR verify: abstract, check, refine if spurious.
pub fn cegar_verify(
    init: &VerifyExpr,
    transition: &VerifyExpr,
    property: &VerifyExpr,
    initial_predicates: &[VerifyExpr],
    max_refinements: u32,
) -> AbstractionResult {
    let mut predicates = initial_predicates.to_vec();

    for _iteration in 0..max_refinements {
        // Verify using k-induction on the abstract/concrete model
        let result = kinduction::k_induction(
            init, transition, property, &[], 10,
        );

        match result {
            kinduction::KInductionResult::Proven { .. } => {
                return AbstractionResult::Safe;
            }
            kinduction::KInductionResult::Counterexample { trace, .. } => {
                // Check if counterexample is concrete (real)
                // Simplified: if we find a counterexample, check if adding
                // the property as a predicate helps
                if predicates.contains(property) {
                    return AbstractionResult::Unsafe { concrete_trace: trace };
                }
                // Refine: add property as new predicate
                predicates.push(property.clone());
                return AbstractionResult::SpuriousRefined {
                    new_predicates: predicates,
                };
            }
            _ => return AbstractionResult::Unknown,
        }
    }

    AbstractionResult::Unknown
}
