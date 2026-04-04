//! Multi-Clock Domain Modeling
//!
//! Real designs have multiple clock domains. Each domain unrolls independently.
//! Cross-domain references use interleaved scheduling based on clock ratios.
//!
//! The schedule is computed from each domain's ratio (numerator, denominator).
//! Within one LCM period, each domain fires proportionally to its normalized rate.
//! On global steps where a domain does NOT fire, a frame condition holds its
//! state variables unchanged.

use crate::ir::VerifyExpr;
use crate::equivalence::Trace;
use crate::kinduction;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct ClockDomain {
    pub name: String,
    pub frequency: Option<u64>,
    pub ratio: Option<(u32, u32)>,
}

#[derive(Debug, Clone)]
pub struct MultiClockModel {
    pub domains: Vec<ClockDomain>,
    pub init: VerifyExpr,
    pub transitions: HashMap<String, VerifyExpr>,
    pub property: VerifyExpr,
}

#[derive(Debug)]
pub enum MultiClockResult {
    Safe,
    Unsafe { trace: Trace },
    Unknown,
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a } else { gcd(b, a % b) }
}

fn lcm(a: u32, b: u32) -> u32 {
    if a == 0 || b == 0 { 1 } else { a / gcd(a, b) * b }
}

/// Compute the interleaved firing schedule for a set of clock domains.
///
/// Returns a `Vec<Vec<bool>>` where `schedule[global_step][domain_index]`
/// indicates whether that domain fires at that global step.
///
/// The schedule covers one full LCM period. Ratios are interpreted as
/// (numerator, denominator) meaning the domain fires `numerator` times
/// per `denominator`-unit base period. Domains without a ratio default
/// to (1, 1).
///
/// Within the LCM period, the fastest domain fires every step. Slower
/// domains fire at evenly-spaced intervals proportional to their rates.
pub fn compute_schedule(domains: &[ClockDomain]) -> Vec<Vec<bool>> {
    if domains.is_empty() {
        return vec![vec![]];
    }

    let rates: Vec<(u32, u32)> = domains.iter().map(|d| {
        d.ratio.unwrap_or((1, 1))
    }).collect();

    // Normalize rates to a common denominator, then the period is the max
    // scaled numerator. Each domain fires scaled_rate times in period steps.
    let common_denom: u32 = rates.iter().fold(1u32, |acc, &(_, d)| lcm(acc, d));
    let scaled: Vec<u32> = rates.iter().map(|&(n, d)| n * (common_denom / d)).collect();
    let period = *scaled.iter().max().unwrap_or(&1);

    if period == 0 {
        return vec![vec![false; domains.len()]];
    }

    let mut schedule = Vec::with_capacity(period as usize);
    for step in 0..period {
        let mut fires = Vec::with_capacity(domains.len());
        for &fire_count in &scaled {
            if fire_count == 0 {
                fires.push(false);
            } else if fire_count >= period {
                fires.push(true);
            } else {
                // Bresenham-style even distribution starting at step 0.
                // Domain fires at step s if (s * fire_count) % period < fire_count.
                let remainder = (step as u64 * fire_count as u64) % period as u64;
                fires.push(remainder < fire_count as u64);
            }
        }
        schedule.push(fires);
    }

    schedule
}

/// Extract the set of "next-state" variable base names from a transition expression.
///
/// Looks for variables matching `<name>@t1` and returns the base names.
fn extract_next_state_vars(expr: &VerifyExpr) -> HashSet<String> {
    let mut all_vars = HashSet::new();
    crate::equivalence::collect_vars_pub(expr, &mut all_vars);
    let mut bases = HashSet::new();
    for v in &all_vars {
        if let Some(base) = v.strip_suffix("@t1") {
            bases.insert(base.to_string());
        }
    }
    bases
}

/// Build a frame condition for a domain: all its next-state variables equal current-state.
/// `var@t1 <=> var@t` for each variable in the domain's transition.
fn build_frame_condition(transition: &VerifyExpr) -> VerifyExpr {
    let bases = extract_next_state_vars(transition);
    let mut conditions: Vec<VerifyExpr> = bases.into_iter().map(|base| {
        VerifyExpr::iff(
            VerifyExpr::var(format!("{}@t1", base)),
            VerifyExpr::var(format!("{}@t", base)),
        )
    }).collect();
    // Sort for determinism
    conditions.sort_by(|a, b| format!("{:?}", a).cmp(&format!("{:?}", b)));
    if conditions.is_empty() {
        VerifyExpr::bool(true)
    } else {
        conditions.into_iter().reduce(|a, b| VerifyExpr::and(a, b)).unwrap()
    }
}

/// Verify a multi-clock domain design.
///
/// For single domain (or zero domains), delegates to standard k-induction.
/// For multiple domains, computes an interleaved schedule from clock ratios
/// and builds per-step transitions that apply each domain's transition on its
/// fire steps and a frame condition (state held) on non-fire steps.
pub fn verify_multiclock(model: &MultiClockModel, bound: u32) -> MultiClockResult {
    if model.domains.len() <= 1 {
        let transition = model.transitions.values().next()
            .cloned()
            .unwrap_or(VerifyExpr::bool(true));
        let result = kinduction::k_induction(&model.init, &transition, &model.property, &[], bound);
        match result {
            kinduction::KInductionResult::Proven { .. } => MultiClockResult::Safe,
            kinduction::KInductionResult::Counterexample { trace, .. } => MultiClockResult::Unsafe { trace },
            _ => MultiClockResult::Unknown,
        }
    } else {
        verify_multiclock_interleaved(model, bound)
    }
}

/// Multi-domain verification with interleaved scheduling.
///
/// Algorithm:
/// 1. Compute the schedule from domain ratios.
/// 2. For each global step in the schedule period, build a combined transition:
///    - If domain fires: apply its transition
///    - If domain does not fire: apply frame condition (state held)
/// 3. Use BMC-style unrolling with the schedule-aware transitions and
///    check the property at each step.
fn verify_multiclock_interleaved(model: &MultiClockModel, bound: u32) -> MultiClockResult {
    let schedule = compute_schedule(&model.domains);
    let period = schedule.len() as u32;

    // Build per-domain frame conditions
    let mut frame_conditions: HashMap<String, VerifyExpr> = HashMap::new();
    for domain in &model.domains {
        if let Some(trans) = model.transitions.get(&domain.name) {
            frame_conditions.insert(domain.name.clone(), build_frame_condition(trans));
        }
    }

    // Build the transition for each global step within one period.
    // Each step's transition is the conjunction of all domains' contributions:
    // - fire => domain's transition
    // - !fire => frame condition
    let mut step_transitions: Vec<VerifyExpr> = Vec::with_capacity(period as usize);
    for step_idx in 0..period as usize {
        let mut parts: Vec<VerifyExpr> = Vec::new();
        for (domain_idx, domain) in model.domains.iter().enumerate() {
            let fires = schedule.get(step_idx)
                .and_then(|s| s.get(domain_idx))
                .copied()
                .unwrap_or(true);
            if fires {
                if let Some(trans) = model.transitions.get(&domain.name) {
                    parts.push(trans.clone());
                }
            } else {
                if let Some(frame) = frame_conditions.get(&domain.name) {
                    parts.push(frame.clone());
                }
            }
        }
        let combined = if parts.is_empty() {
            VerifyExpr::bool(true)
        } else {
            parts.into_iter().reduce(|a, b| VerifyExpr::and(a, b)).unwrap()
        };
        step_transitions.push(combined);
    }

    // Now run k-induction where each step uses the schedule-appropriate transition.
    // We pass the schedule-aware transitions to a custom k-induction loop that
    // cycles through the schedule period.
    k_induction_with_schedule(
        &model.init,
        &step_transitions,
        &model.property,
        bound,
    )
}

/// K-induction that cycles through a vector of transitions (one per schedule step).
///
/// This is like standard k-induction but the transition at global step `t` is
/// `step_transitions[t % period]` instead of a single uniform transition.
fn k_induction_with_schedule(
    init: &VerifyExpr,
    step_transitions: &[VerifyExpr],
    property: &VerifyExpr,
    max_k: u32,
) -> MultiClockResult {
    use z3::{Config, Context, SatResult, Solver, ast::Bool, ast::Ast};

    let period = step_transitions.len() as u32;
    if period == 0 {
        return MultiClockResult::Safe;
    }

    let mut cfg = Config::new();
    cfg.set_param_value("timeout", "30000");
    let ctx = Context::new(&cfg);

    for k in 1..=max_k {
        // ---- Base case ----
        // init(0) AND T_0(0,1) AND T_1(1,2) AND ... AND T_{k-2}(k-2,k-1) AND NOT P(i) for some i
        let base_result = {
            let solver = Solver::new(&ctx);

            let init_0 = kinduction::instantiate_at(init, 0);
            let init_z3 = encode_to_bool_mc(&ctx, &init_0);
            solver.assert(&init_z3);

            for t in 0..k.saturating_sub(1) {
                let sched_idx = (t % period) as usize;
                let trans = kinduction::instantiate_transition(&step_transitions[sched_idx], t);
                let trans_z3 = encode_to_bool_mc(&ctx, &trans);
                solver.assert(&trans_z3);
            }

            let mut not_props: Vec<Bool> = Vec::new();
            for t in 0..k {
                let prop_t = kinduction::instantiate_at(property, t);
                let prop_z3 = encode_to_bool_mc(&ctx, &prop_t);
                not_props.push(prop_z3.not());
            }
            let not_prop_refs: Vec<&Bool> = not_props.iter().collect();
            let some_violation = Bool::or(&ctx, &not_prop_refs);
            solver.assert(&some_violation);

            solver.check()
        };

        match base_result {
            SatResult::Sat => {
                return MultiClockResult::Unsafe {
                    trace: Trace { cycles: vec![] },
                };
            }
            SatResult::Unknown => return MultiClockResult::Unknown,
            SatResult::Unsat => {}
        }

        // ---- Inductive step ----
        // P(0) AND P(1) AND ... AND P(k-1) AND T_0(0,1) AND ... AND T_{k-1}(k-1,k) AND NOT P(k)
        let step_result = {
            let solver = Solver::new(&ctx);

            for t in 0..k {
                let prop_t = kinduction::instantiate_at(property, t);
                let prop_z3 = encode_to_bool_mc(&ctx, &prop_t);
                solver.assert(&prop_z3);
            }

            for t in 0..k {
                let sched_idx = (t % period) as usize;
                let trans = kinduction::instantiate_transition(&step_transitions[sched_idx], t);
                let trans_z3 = encode_to_bool_mc(&ctx, &trans);
                solver.assert(&trans_z3);
            }

            let prop_k = kinduction::instantiate_at(property, k);
            let prop_k_z3 = encode_to_bool_mc(&ctx, &prop_k);
            solver.assert(&prop_k_z3.not());

            solver.check()
        };

        match step_result {
            SatResult::Unsat => {
                return MultiClockResult::Safe;
            }
            SatResult::Unknown => return MultiClockResult::Unknown,
            SatResult::Sat => {}
        }
    }

    MultiClockResult::Unknown
}

/// Encode a VerifyExpr to Z3 Bool for multi-clock verification.
fn encode_to_bool_mc<'ctx>(ctx: &'ctx z3::Context, expr: &VerifyExpr) -> z3::ast::Bool<'ctx> {
    let mut all_vars = HashSet::new();
    crate::equivalence::collect_vars_pub(expr, &mut all_vars);

    let mut bool_vars: HashMap<String, z3::ast::Bool<'ctx>> = HashMap::new();
    let mut int_vars: HashMap<String, z3::ast::Int<'ctx>> = HashMap::new();

    for name in &all_vars {
        bool_vars.insert(name.clone(), z3::ast::Bool::new_const(ctx, name.as_str()));
    }
    crate::equivalence::collect_int_vars_pub(expr, &mut int_vars, ctx);

    kinduction::encode_expr_bool(ctx, expr, &bool_vars, &int_vars)
}
