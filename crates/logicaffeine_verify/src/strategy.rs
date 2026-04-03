//! Strategy Selection Engine
//!
//! Automatically selects the best verification algorithm based on property structure.

use crate::ir::VerifyExpr;
use crate::equivalence::Trace;
use crate::kinduction::{self, KInductionResult, SignalDecl};

/// Verification strategy.
#[derive(Debug, Clone, PartialEq)]
pub enum Strategy {
    Bmc(u32),
    KInduction(u32),
    Ic3,
    Interpolation(u32),
    LivenessToSafety,
    Portfolio { strategies: Vec<Strategy>, timeout_each_ms: u64 },
}

/// Unified verification result from auto-selection.
#[derive(Debug)]
pub enum VerificationResult {
    Safe { strategy_used: Strategy },
    Unsafe { trace: Trace, strategy_used: Strategy },
    Unknown,
}

/// Select the best strategy based on property structure.
pub fn select_strategy(property: &VerifyExpr, _signals: &[SignalDecl]) -> Strategy {
    // Heuristic: analyze property structure
    if contains_liveness_pattern(property) {
        return Strategy::LivenessToSafety;
    }

    // For most safety properties, try IC3 first (most capable)
    if is_small_property(property) {
        Strategy::KInduction(10)
    } else {
        Strategy::Ic3
    }
}

/// Verify a property using automatic strategy selection.
pub fn verify_auto(
    init: &VerifyExpr,
    transition: &VerifyExpr,
    property: &VerifyExpr,
    signals: &[SignalDecl],
) -> VerificationResult {
    let strategy = select_strategy(property, signals);

    match &strategy {
        Strategy::KInduction(max_k) => {
            let result = kinduction::k_induction(init, transition, property, signals, *max_k);
            match result {
                KInductionResult::Proven { .. } => VerificationResult::Safe {
                    strategy_used: strategy,
                },
                KInductionResult::Counterexample { trace, .. } => VerificationResult::Unsafe {
                    trace,
                    strategy_used: strategy,
                },
                _ => {
                    // Fall through to IC3
                    try_ic3(init, transition, property)
                }
            }
        }
        Strategy::Ic3 => try_ic3(init, transition, property),
        Strategy::LivenessToSafety => {
            let result = crate::liveness::check_liveness(init, transition, &[], property, 10);
            match result {
                crate::liveness::LivenessResult::Live => VerificationResult::Safe {
                    strategy_used: strategy,
                },
                crate::liveness::LivenessResult::NotLive { trace, .. } => VerificationResult::Unsafe {
                    trace,
                    strategy_used: strategy,
                },
                _ => VerificationResult::Unknown,
            }
        }
        Strategy::Interpolation(bound) => {
            let result = crate::interpolation::itp_model_check(init, transition, property, *bound);
            match result {
                crate::interpolation::InterpolationResult::Safe
                | crate::interpolation::InterpolationResult::Fixpoint { .. } => {
                    VerificationResult::Safe { strategy_used: strategy }
                }
                crate::interpolation::InterpolationResult::Unsafe { trace } => {
                    VerificationResult::Unsafe { trace, strategy_used: strategy }
                }
                _ => VerificationResult::Unknown,
            }
        }
        Strategy::Bmc(bound) => {
            let result = kinduction::k_induction(init, transition, property, signals, *bound);
            match result {
                KInductionResult::Counterexample { trace, .. } => VerificationResult::Unsafe {
                    trace,
                    strategy_used: strategy,
                },
                KInductionResult::Proven { .. } => VerificationResult::Safe {
                    strategy_used: strategy,
                },
                _ => VerificationResult::Unknown,
            }
        }
        Strategy::Portfolio { strategies, .. } => {
            // Try strategies in order, return first definitive result
            for s in strategies {
                let result = verify_with_strategy(init, transition, property, signals, s);
                match &result {
                    VerificationResult::Safe { .. } | VerificationResult::Unsafe { .. } => return result,
                    VerificationResult::Unknown => continue,
                }
            }
            VerificationResult::Unknown
        }
    }
}

fn try_ic3(init: &VerifyExpr, transition: &VerifyExpr, property: &VerifyExpr) -> VerificationResult {
    let result = crate::ic3::ic3(init, transition, property, 20);
    match result {
        crate::ic3::Ic3Result::Safe { .. } => VerificationResult::Safe {
            strategy_used: Strategy::Ic3,
        },
        crate::ic3::Ic3Result::Unsafe { trace } => VerificationResult::Unsafe {
            trace,
            strategy_used: Strategy::Ic3,
        },
        _ => VerificationResult::Unknown,
    }
}

fn verify_with_strategy(
    init: &VerifyExpr,
    transition: &VerifyExpr,
    property: &VerifyExpr,
    signals: &[SignalDecl],
    strategy: &Strategy,
) -> VerificationResult {
    match strategy {
        Strategy::KInduction(k) => {
            let result = kinduction::k_induction(init, transition, property, signals, *k);
            match result {
                KInductionResult::Proven { .. } => VerificationResult::Safe {
                    strategy_used: strategy.clone(),
                },
                KInductionResult::Counterexample { trace, .. } => VerificationResult::Unsafe {
                    trace,
                    strategy_used: strategy.clone(),
                },
                _ => VerificationResult::Unknown,
            }
        }
        Strategy::Ic3 => try_ic3(init, transition, property),
        _ => VerificationResult::Unknown,
    }
}

fn contains_liveness_pattern(expr: &VerifyExpr) -> bool {
    // Simple heuristic: check for patterns that suggest liveness
    // G(F(p)) patterns would typically have nested temporal operators
    // For now, we don't have explicit temporal operators in VerifyExpr,
    // so this is a placeholder
    false
}

fn is_small_property(expr: &VerifyExpr) -> bool {
    expr_size(expr) < 20
}

fn expr_size(expr: &VerifyExpr) -> usize {
    match expr {
        VerifyExpr::Bool(_) | VerifyExpr::Int(_) | VerifyExpr::Var(_) => 1,
        VerifyExpr::BitVecConst { .. } => 1,
        VerifyExpr::Binary { left, right, .. } => 1 + expr_size(left) + expr_size(right),
        VerifyExpr::Not(inner) => 1 + expr_size(inner),
        VerifyExpr::Iff(l, r) => 1 + expr_size(l) + expr_size(r),
        VerifyExpr::ForAll { body, .. } | VerifyExpr::Exists { body, .. } => 1 + expr_size(body),
        VerifyExpr::Apply { args, .. } => 1 + args.iter().map(expr_size).sum::<usize>(),
        VerifyExpr::BitVecBinary { left, right, .. } => 1 + expr_size(left) + expr_size(right),
        VerifyExpr::BitVecExtract { operand, .. } => 1 + expr_size(operand),
        VerifyExpr::BitVecConcat(l, r) => 1 + expr_size(l) + expr_size(r),
        VerifyExpr::Select { array, index } => 1 + expr_size(array) + expr_size(index),
        VerifyExpr::Store { array, index, value } => 1 + expr_size(array) + expr_size(index) + expr_size(value),
        VerifyExpr::AtState { state, expr } => 1 + expr_size(state) + expr_size(expr),
        VerifyExpr::Transition { from, to } => 1 + expr_size(from) + expr_size(to),
    }
}
