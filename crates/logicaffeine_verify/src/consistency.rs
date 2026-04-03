//! Multi-Property Consistency Checking
//!
//! Check if a set of properties can all hold simultaneously.
//! Conjoin all properties, check Z3 satisfiability.
//! If UNSAT → extract minimal unsatisfiable subset (MUS).
//! If SAT → check for vacuity and redundancy.

use crate::ir::{VerifyExpr, VerifyOp};
use crate::equivalence::Trace;
use std::collections::{HashMap, HashSet};
use z3::{ast::Ast, ast::Bool, ast::Int, Config, Context, SatResult, Solver};

// ═══════════════════════════════════════════════════════════════════════════
// LEGACY API (backward compatibility)
// ═══════════════════════════════════════════════════════════════════════════

/// Result of consistency checking (legacy API).
#[derive(Debug)]
pub enum ConsistencyResult {
    /// All properties can hold simultaneously.
    Consistent,
    /// At least two properties conflict. Returns the conflicting pair indices
    /// and a witness trace showing the contradiction.
    Inconsistent {
        conflicting: Vec<(usize, usize)>,
        witness: Trace,
    },
    /// Z3 returned unknown.
    Unknown,
}

/// Check if a set of properties can all hold simultaneously (legacy API).
///
/// Conjoins all properties and checks Z3 satisfiability.
/// If UNSAT, identifies the conflicting pair(s).
pub fn check_consistency(
    props: &[VerifyExpr],
    signals: &[String],
    bound: usize,
) -> ConsistencyResult {
    if props.is_empty() {
        return ConsistencyResult::Consistent;
    }

    let mut cfg = Config::new();
    cfg.set_param_value("timeout", "10000");
    let ctx = Context::new(&cfg);
    let solver = Solver::new(&ctx);

    // Declare all signal@timestep variables
    let mut var_map: HashMap<String, Bool> = HashMap::new();
    for sig in signals {
        for t in 0..=bound {
            let var_name = format!("{}@{}", sig, t);
            var_map.insert(var_name.clone(), Bool::new_const(&ctx, var_name.as_str()));
        }
    }

    // Collect all variables from all properties
    let mut all_vars: HashSet<String> = HashSet::new();
    for prop in props {
        crate::equivalence::collect_vars_pub(prop, &mut all_vars);
    }
    for var_name in &all_vars {
        if !var_map.contains_key(var_name) {
            var_map.insert(var_name.clone(), Bool::new_const(&ctx, var_name.as_str()));
        }
    }

    // Encode and conjoin all properties
    let empty_int_vars = HashMap::new();
    let encoder = crate::equivalence::EquivEncoder::new_from_bool_vars(&ctx, &var_map, &empty_int_vars);
    for prop in props {
        let encoded = encoder.encode_as_bool(prop);
        solver.assert(&encoded);
    }

    match solver.check() {
        SatResult::Sat => ConsistencyResult::Consistent,
        SatResult::Unsat => {
            // Find conflicting pairs by checking each pair
            let mut conflicting = Vec::new();
            for i in 0..props.len() {
                for j in (i + 1)..props.len() {
                    let pair_solver = Solver::new(&ctx);
                    let ei = encoder.encode_as_bool(&props[i]);
                    let ej = encoder.encode_as_bool(&props[j]);
                    pair_solver.assert(&ei);
                    pair_solver.assert(&ej);
                    if pair_solver.check() == SatResult::Unsat {
                        conflicting.push((i, j));
                    }
                }
            }
            ConsistencyResult::Inconsistent {
                conflicting,
                witness: Trace { cycles: vec![] },
            }
        }
        SatResult::Unknown => ConsistencyResult::Unknown,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// NEW API: Spec Consistency Analysis
// ═══════════════════════════════════════════════════════════════════════════

/// A formula labeled with its source text for error reporting.
#[derive(Debug, Clone)]
pub struct LabeledFormula {
    pub index: usize,
    pub label: String,
    pub expr: VerifyExpr,
}

/// Configuration for spec consistency analysis.
#[derive(Debug, Clone)]
pub struct ConsistencyConfig {
    /// Z3 timeout per query in milliseconds.
    pub timeout_ms: u64,
    /// BMC bound for temporal unrolling (used by orchestrator, not by Z3 checks).
    pub temporal_bound: u32,
    /// Whether to check for vacuous implications.
    pub check_vacuity: bool,
    /// Whether to check for redundant formulas.
    pub check_redundancy: bool,
    /// Whether to identify pairwise conflicts (only runs when UNSAT).
    pub check_pairwise: bool,
}

impl Default for ConsistencyConfig {
    fn default() -> Self {
        Self {
            timeout_ms: 5000,
            temporal_bound: 8,
            check_vacuity: true,
            check_redundancy: true,
            check_pairwise: true,
        }
    }
}

/// Full consistency analysis report.
#[derive(Debug)]
pub struct ConsistencyReport {
    pub satisfiability: SatisfiabilityResult,
    pub vacuity: Vec<VacuityFinding>,
    pub redundancies: Vec<RedundancyFinding>,
    pub pairwise_conflicts: Vec<PairwiseConflict>,
}

/// Whether the conjunction of all formulas is satisfiable.
#[derive(Debug, PartialEq)]
pub enum SatisfiabilityResult {
    /// All formulas can hold simultaneously.
    Satisfiable,
    /// The formulas are contradictory. `mus` contains the indices of
    /// the Minimal Unsatisfiable Subset.
    Unsatisfiable { mus: Vec<usize> },
    /// Z3 returned unknown (timeout or undecidable).
    Unknown,
}

/// A formula whose implication antecedent can never be satisfied.
#[derive(Debug)]
pub struct VacuityFinding {
    pub formula_index: usize,
    pub label: String,
}

/// A formula that is logically entailed by the remaining formulas.
#[derive(Debug)]
pub struct RedundancyFinding {
    pub redundant_index: usize,
    pub label: String,
    pub entailed_by: Vec<usize>,
}

/// A pair of formulas that cannot hold simultaneously.
#[derive(Debug)]
pub struct PairwiseConflict {
    pub i: usize,
    pub j: usize,
}

// ═══════════════════════════════════════════════════════════════════════════
// CORE ANALYSIS ENGINE
// ═══════════════════════════════════════════════════════════════════════════

/// Run full consistency analysis on a set of labeled formulas.
///
/// Orchestration order:
/// 1. Check full conjunction satisfiability
/// 2. If UNSAT → extract MUS, optionally run pairwise
/// 3. If SAT → optionally run vacuity and redundancy checks
pub fn check_spec_consistency(
    formulas: &[LabeledFormula],
    config: &ConsistencyConfig,
) -> ConsistencyReport {
    if formulas.is_empty() {
        return ConsistencyReport {
            satisfiability: SatisfiabilityResult::Satisfiable,
            vacuity: vec![],
            redundancies: vec![],
            pairwise_conflicts: vec![],
        };
    }

    let mut cfg = Config::new();
    cfg.set_param_value("timeout", &config.timeout_ms.to_string());
    let ctx = Context::new(&cfg);

    // Collect all variable names from all formulas
    let mut all_vars: HashSet<String> = HashSet::new();
    for lf in formulas {
        crate::equivalence::collect_vars_pub(&lf.expr, &mut all_vars);
    }

    // Declare all variables as Z3 Bools, and collect integer variables
    let mut bool_vars: HashMap<String, Bool> = HashMap::new();
    let mut int_vars: HashMap<String, Int> = HashMap::new();
    for var_name in &all_vars {
        bool_vars.insert(
            var_name.clone(),
            Bool::new_const(&ctx, var_name.as_str()),
        );
    }
    // Also detect integer variables from arithmetic contexts
    let exprs: Vec<&VerifyExpr> = formulas.iter().map(|lf| &lf.expr).collect();
    for expr in &exprs {
        crate::equivalence::collect_int_vars_pub(expr, &mut int_vars, &ctx);
    }

    let encoder = crate::equivalence::EquivEncoder::new_from_bool_vars(&ctx, &bool_vars, &int_vars);

    // Step 1: Check full conjunction satisfiability
    let solver = Solver::new(&ctx);
    for lf in formulas {
        let encoded = encoder.encode_as_bool(&lf.expr);
        solver.assert(&encoded);
    }

    match solver.check() {
        SatResult::Sat => {
            // Satisfiable — run vacuity and redundancy checks
            let vacuity = if config.check_vacuity {
                detect_vacuity(&ctx, &encoder, formulas, config.timeout_ms)
            } else {
                vec![]
            };

            let redundancies = if config.check_redundancy {
                detect_redundancy(&ctx, &encoder, formulas, config.timeout_ms)
            } else {
                vec![]
            };

            // Skip pairwise when SAT — no pair can conflict if the full conjunction is SAT
            ConsistencyReport {
                satisfiability: SatisfiabilityResult::Satisfiable,
                vacuity,
                redundancies,
                pairwise_conflicts: vec![],
            }
        }
        SatResult::Unsat => {
            // UNSAT — extract MUS
            let mus = extract_mus(&ctx, &encoder, formulas, config.timeout_ms);

            // Optionally find all pairwise conflicts
            let pairwise_conflicts = if config.check_pairwise {
                detect_pairwise_conflicts(&ctx, &encoder, formulas, config.timeout_ms)
            } else {
                vec![]
            };

            ConsistencyReport {
                satisfiability: SatisfiabilityResult::Unsatisfiable { mus },
                vacuity: vec![],
                redundancies: vec![],
                pairwise_conflicts,
            }
        }
        SatResult::Unknown => {
            ConsistencyReport {
                satisfiability: SatisfiabilityResult::Unknown,
                vacuity: vec![],
                redundancies: vec![],
                pairwise_conflicts: vec![],
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// MUS EXTRACTION
// ═══════════════════════════════════════════════════════════════════════════

/// Deletion-based Minimal Unsatisfiable Subset extraction.
///
/// Precondition: the conjunction of all formulas is UNSAT.
/// For each formula, checks whether removing it makes the remaining set SAT.
/// If so, the formula is necessary for the contradiction (kept in MUS).
/// If not, the formula is not necessary (removed from MUS).
///
/// O(n) Z3 calls.
fn extract_mus<'ctx>(
    ctx: &'ctx Context,
    encoder: &crate::equivalence::EquivEncoder<'ctx>,
    formulas: &[LabeledFormula],
    timeout_ms: u64,
) -> Vec<usize> {
    let n = formulas.len();
    let mut in_mus: Vec<bool> = vec![true; n];

    for i in 0..n {
        // Try removing formula i — check if the rest is still UNSAT
        let solver = Solver::new(ctx);
        solver.set_params(&{
            let mut params = z3::Params::new(ctx);
            params.set_u32("timeout", timeout_ms as u32);
            params
        });

        for j in 0..n {
            if j == i || !in_mus[j] {
                continue;
            }
            let encoded = encoder.encode_as_bool(&formulas[j].expr);
            solver.assert(&encoded);
        }

        match solver.check() {
            SatResult::Sat => {
                // Removing i made it SAT → i is necessary for UNSAT → keep in MUS
            }
            SatResult::Unsat => {
                // Still UNSAT without i → i is not necessary → remove from MUS
                in_mus[i] = false;
            }
            SatResult::Unknown => {
                // Conservative: keep it in MUS
            }
        }
    }

    in_mus.iter().enumerate()
        .filter(|(_, &b)| b)
        .map(|(i, _)| i)
        .collect()
}

// ═══════════════════════════════════════════════════════════════════════════
// VACUITY DETECTION
// ═══════════════════════════════════════════════════════════════════════════

/// Extract the antecedent from an implication-shaped formula.
///
/// Looks through ForAll wrappers to find the implication core.
fn extract_antecedent(expr: &VerifyExpr) -> Option<&VerifyExpr> {
    match expr {
        VerifyExpr::Binary { op: VerifyOp::Implies, left, .. } => Some(left),
        VerifyExpr::ForAll { body, .. } => extract_antecedent(body),
        _ => None,
    }
}

/// Detect vacuously true implications.
///
/// For each formula with implication shape (P → Q), checks whether P
/// can be satisfied under the context of all other formulas.
/// If not, the implication is vacuously true.
fn detect_vacuity<'ctx>(
    ctx: &'ctx Context,
    encoder: &crate::equivalence::EquivEncoder<'ctx>,
    formulas: &[LabeledFormula],
    timeout_ms: u64,
) -> Vec<VacuityFinding> {
    let mut findings = Vec::new();

    for i in 0..formulas.len() {
        let antecedent = match extract_antecedent(&formulas[i].expr) {
            Some(a) => a,
            None => continue,
        };

        // Build context: conjunction of all other formulas
        let solver = Solver::new(ctx);
        solver.set_params(&{
            let mut params = z3::Params::new(ctx);
            params.set_u32("timeout", timeout_ms as u32);
            params
        });

        for j in 0..formulas.len() {
            if j == i {
                continue;
            }
            let encoded = encoder.encode_as_bool(&formulas[j].expr);
            solver.assert(&encoded);
        }

        // Assert the antecedent — can it be true?
        let ante_encoded = encoder.encode_as_bool(antecedent);
        solver.assert(&ante_encoded);

        if solver.check() == SatResult::Unsat {
            findings.push(VacuityFinding {
                formula_index: i,
                label: formulas[i].label.clone(),
            });
        }
    }

    findings
}

// ═══════════════════════════════════════════════════════════════════════════
// REDUNDANCY DETECTION
// ═══════════════════════════════════════════════════════════════════════════

/// Detect formulas that are logically entailed by the remaining formulas.
///
/// For each formula F_i, checks whether context ∧ ¬F_i is satisfiable.
/// If UNSAT, F_i is entailed by the others (redundant).
fn detect_redundancy<'ctx>(
    ctx: &'ctx Context,
    encoder: &crate::equivalence::EquivEncoder<'ctx>,
    formulas: &[LabeledFormula],
    timeout_ms: u64,
) -> Vec<RedundancyFinding> {
    let mut findings = Vec::new();

    for i in 0..formulas.len() {
        let solver = Solver::new(ctx);
        solver.set_params(&{
            let mut params = z3::Params::new(ctx);
            params.set_u32("timeout", timeout_ms as u32);
            params
        });

        // Assert all other formulas as context
        for j in 0..formulas.len() {
            if j == i {
                continue;
            }
            let encoded = encoder.encode_as_bool(&formulas[j].expr);
            solver.assert(&encoded);
        }

        // Assert the negation of F_i
        let fi_encoded = encoder.encode_as_bool(&formulas[i].expr);
        solver.assert(&fi_encoded.not());

        if solver.check() == SatResult::Unsat {
            // context ∧ ¬F_i is UNSAT → F_i is entailed by others
            let entailed_by: Vec<usize> = (0..formulas.len())
                .filter(|&j| j != i)
                .collect();
            findings.push(RedundancyFinding {
                redundant_index: i,
                label: formulas[i].label.clone(),
                entailed_by,
            });
        }
    }

    findings
}

// ═══════════════════════════════════════════════════════════════════════════
// PAIRWISE CONFLICT DETECTION
// ═══════════════════════════════════════════════════════════════════════════

/// Identify all pairs of formulas that cannot hold simultaneously.
///
/// Only meaningful when the full conjunction is UNSAT.
fn detect_pairwise_conflicts<'ctx>(
    ctx: &'ctx Context,
    encoder: &crate::equivalence::EquivEncoder<'ctx>,
    formulas: &[LabeledFormula],
    timeout_ms: u64,
) -> Vec<PairwiseConflict> {
    let mut conflicts = Vec::new();

    for i in 0..formulas.len() {
        for j in (i + 1)..formulas.len() {
            let solver = Solver::new(ctx);
            solver.set_params(&{
                let mut params = z3::Params::new(ctx);
                params.set_u32("timeout", timeout_ms as u32);
                params
            });

            let ei = encoder.encode_as_bool(&formulas[i].expr);
            let ej = encoder.encode_as_bool(&formulas[j].expr);
            solver.assert(&ei);
            solver.assert(&ej);

            if solver.check() == SatResult::Unsat {
                conflicts.push(PairwiseConflict { i, j });
            }
        }
    }

    conflicts
}
