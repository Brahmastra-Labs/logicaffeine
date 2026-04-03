//! Assume-Guarantee Compositional Reasoning
//!
//! Decompose verification into per-component proofs with interface contracts.
//! Each component has assumptions (what it requires from environment) and
//! guarantees (what it provides). Verify each component independently.

use crate::ir::VerifyExpr;
use crate::equivalence::Trace;
use crate::kinduction;
use std::collections::HashMap;

/// A component specification for compositional verification.
#[derive(Debug, Clone)]
pub struct ComponentSpec {
    pub name: String,
    pub assumes: Vec<VerifyExpr>,
    pub guarantees: Vec<VerifyExpr>,
    pub init: VerifyExpr,
    pub transition: VerifyExpr,
}

/// Result of compositional verification.
#[derive(Debug)]
pub enum CompositionalResult {
    /// All components verified.
    AllVerified,
    /// A component failed verification.
    ComponentFailed { name: String, trace: Trace },
    /// Circular dependency detected.
    CircularDependency { components: Vec<String> },
    /// Could not determine.
    Unknown,
}

/// Verify a set of components compositionally.
///
/// For each component:
/// 1. Assume all assumptions hold
/// 2. Verify all guarantees under those assumptions
/// 3. Check that each component's guarantees discharge other components' assumptions
pub fn verify_compositional(components: &[ComponentSpec]) -> CompositionalResult {
    if components.is_empty() {
        return CompositionalResult::AllVerified;
    }

    let mut cfg = z3::Config::new();
    cfg.set_param_value("timeout", "30000");
    let ctx = z3::Context::new(&cfg);

    // Phase 1: Verify each component in isolation (assumptions → guarantees)
    for comp in components {
        for guarantee in &comp.guarantees {
            // Check: init AND assumptions AND transition → guarantee
            let assumptions = if comp.assumes.is_empty() {
                VerifyExpr::bool(true)
            } else {
                comp.assumes.iter().cloned().reduce(|a, b| VerifyExpr::and(a, b)).unwrap()
            };

            // Use k-induction to verify: under assumptions, guarantee holds
            let strengthened_init = VerifyExpr::and(comp.init.clone(), assumptions.clone());
            let strengthened_transition = VerifyExpr::and(comp.transition.clone(), assumptions.clone());

            let result = kinduction::k_induction(
                &strengthened_init,
                &strengthened_transition,
                guarantee,
                &[],
                10,
            );

            match result {
                kinduction::KInductionResult::Counterexample { trace, .. } => {
                    return CompositionalResult::ComponentFailed {
                        name: comp.name.clone(),
                        trace,
                    };
                }
                kinduction::KInductionResult::Proven { .. } => {
                    // This guarantee verified
                }
                _ => {
                    // Inconclusive — try IC3 as fallback
                    let ic3_result = crate::ic3::ic3(
                        &strengthened_init,
                        &strengthened_transition,
                        guarantee,
                        10,
                    );
                    match ic3_result {
                        crate::ic3::Ic3Result::Safe { .. } => {}
                        crate::ic3::Ic3Result::Unsafe { trace } => {
                            return CompositionalResult::ComponentFailed {
                                name: comp.name.clone(),
                                trace,
                            };
                        }
                        _ => return CompositionalResult::Unknown,
                    }
                }
            }
        }
    }

    // Phase 2: Check that guarantees discharge assumptions
    // For each component A's assumption, check that some component B's guarantee covers it
    // (simplified: check that the conjunction of all guarantees implies each assumption)
    let all_guarantees: Vec<VerifyExpr> = components.iter()
        .flat_map(|c| c.guarantees.clone())
        .collect();

    if !all_guarantees.is_empty() {
        let guarantees_conj = all_guarantees.iter().cloned()
            .reduce(|a, b| VerifyExpr::and(a, b)).unwrap();

        for comp in components {
            for assumption in &comp.assumes {
                // Check: guarantees_conj → assumption
                let check = VerifyExpr::implies(guarantees_conj.clone(), assumption.clone());
                let solver = z3::Solver::new(&ctx);
                let not_check = VerifyExpr::not(check);
                let encoded = encode_bool(&ctx, &not_check);
                solver.assert(&encoded);
                match solver.check() {
                    z3::SatResult::Sat => {
                        // Guarantee doesn't cover assumption — could be circular
                        // For now, report as circular dependency
                        return CompositionalResult::CircularDependency {
                            components: vec![comp.name.clone()],
                        };
                    }
                    z3::SatResult::Unsat => {} // Good: guarantees discharge assumption
                    z3::SatResult::Unknown => return CompositionalResult::Unknown,
                }
            }
        }
    }

    CompositionalResult::AllVerified
}

fn encode_bool<'ctx>(ctx: &'ctx z3::Context, expr: &VerifyExpr) -> z3::ast::Bool<'ctx> {
    let mut bool_vars = HashMap::new();
    let mut int_vars = HashMap::new();
    let mut all_vars = std::collections::HashSet::new();
    crate::equivalence::collect_vars_pub(expr, &mut all_vars);
    for name in &all_vars {
        bool_vars.insert(name.clone(), z3::ast::Bool::new_const(ctx, name.as_str()));
    }
    crate::equivalence::collect_int_vars_pub(expr, &mut int_vars, ctx);
    kinduction::encode_expr_bool(ctx, expr, &bool_vars, &int_vars)
}
