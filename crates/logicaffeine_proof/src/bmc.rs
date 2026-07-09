//! Bounded model checking, k-induction, and vacuity over `ProofExpr` — pure-Rust, certified,
//! Z3-free, browser-ready.
//!
//! A transition system is given as three closures over `signal@t` atoms:
//! - `init` — the constraint at `t = 0`,
//! - `trans(t)` — relates state `@t` to `@t+1`,
//! - `property(t)` — the safety property that must hold at `@t`.
//!
//! Every question reduces to one unsatisfiability query discharged by
//! [`crate::sat::prove_unsat`] (CDCL + RUP certification):
//! - **BMC** ([`find_counterexample`]) — is a violating state reachable within `k` steps?
//!   `init ∧ ⋀ trans ∧ ¬property(k)` satisfiable ⇒ a counterexample trace.
//! - **k-induction** ([`prove_invariant`]) — base (no violation in the first `k` steps) +
//!   step (`property` is `k`-inductive); both certified-UNSAT ⇒ the property holds for ALL
//!   reachable states (an unbounded proof, not merely bounded).
//! - **Vacuity** ([`check_vacuity`]) — can the antecedent ever fire? Unsatisfiable ⇒ a dead
//!   trigger and a vacuously-true property.

use crate::cdcl::SolveResult;
use crate::cnf::Cnf;
use crate::sat::{decode_model, prove_unsat, UnsatOutcome};
use crate::ProofExpr;

fn not(e: ProofExpr) -> ProofExpr {
    ProofExpr::Not(Box::new(e))
}

/// Conjoin a list of obligations. An empty list is the constant `true` (a tautology over a
/// reserved atom, since `ProofExpr` has no Boolean literal).
fn conj(mut parts: Vec<ProofExpr>) -> ProofExpr {
    match parts.len() {
        0 => {
            let c = ProofExpr::Atom("__bmc_true".to_string());
            ProofExpr::Or(Box::new(c.clone()), Box::new(not(c)))
        }
        1 => parts.pop().unwrap(),
        _ => {
            let mut acc = parts.pop().unwrap();
            while let Some(p) = parts.pop() {
                acc = ProofExpr::And(Box::new(p), Box::new(acc));
            }
            acc
        }
    }
}

/// `init ∧ trans(0) ∧ … ∧ trans(k-1)` — the symbolic paths of length `k` from an initial
/// state.
fn unrolled_path(
    init: &ProofExpr,
    trans: &dyn Fn(u32) -> ProofExpr,
    k: u32,
) -> Vec<ProofExpr> {
    let mut parts = vec![init.clone()];
    for i in 0..k {
        parts.push(trans(i));
    }
    parts
}

/// The verdict of bounded model checking.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BmcOutcome {
    /// A reachable state at depth `k` violates the property, with a witnessing trace.
    CounterexampleAt { k: u32, trace: Vec<(String, bool)> },
    /// No violating state is reachable within `max_k` transitions (a bounded guarantee).
    NoneWithin(u32),
    /// An obligation left the propositional fragment — escalate (e.g. bit-blast).
    Unsupported,
}

/// Bounded model checking: search for a reachable state violating `property` within `max_k`
/// transitions, returning the shallowest counterexample (and its trace) if one exists.
pub fn find_counterexample(
    init: &ProofExpr,
    trans: &dyn Fn(u32) -> ProofExpr,
    property: &dyn Fn(u32) -> ProofExpr,
    max_k: u32,
) -> BmcOutcome {
    for k in 0..=max_k {
        let mut parts = unrolled_path(init, trans, k);
        parts.push(not(property(k)));
        match prove_unsat(&conj(parts)) {
            UnsatOutcome::Sat(trace) => return BmcOutcome::CounterexampleAt { k, trace },
            UnsatOutcome::Refuted => continue,
            UnsatOutcome::Unsupported => return BmcOutcome::Unsupported,
        }
    }
    BmcOutcome::NoneWithin(max_k)
}

/// Bounded model checking, **incrementally**: the unrolling is clausified ONCE into a single
/// persistent solver, and each depth's violation `¬property(k)` is checked by
/// [`crate::cdcl::Solver::solve_under_assumptions`]. Every clause learned while ruling out a
/// shallow depth is reused at the next — the IPASIR amortisation that makes deep BMC fast.
///
/// All transitions are asserted up front, so this is sound for **total** transition relations
/// — every hardware next-state function is total (a prefix always extends to a full path), so
/// the result matches the one-clause-per-call [`find_counterexample`]. For a partial /
/// over-constrained transition relation, prefer [`find_counterexample`].
pub fn find_counterexample_incremental(
    init: &ProofExpr,
    trans: &dyn Fn(u32) -> ProofExpr,
    property: &dyn Fn(u32) -> ProofExpr,
    max_k: u32,
) -> BmcOutcome {
    let mut cnf = Cnf::new();
    if cnf.assert(init).is_none() {
        return BmcOutcome::Unsupported;
    }
    for i in 0..max_k {
        if cnf.assert(&trans(i)).is_none() {
            return BmcOutcome::Unsupported;
        }
    }
    // Encode each depth's violation to an activation literal (defining clauses only — the
    // violation is asserted per query, via the assumption), and remember the exprs whose
    // atoms make up a decoded trace.
    let mut bad = Vec::with_capacity(max_k as usize + 1);
    let mut atom_exprs: Vec<ProofExpr> = vec![init.clone()];
    for i in 0..max_k {
        atom_exprs.push(trans(i));
    }
    for k in 0..=max_k {
        let violation = not(property(k));
        match cnf.encode(&violation) {
            Some(lit) => bad.push(lit),
            None => return BmcOutcome::Unsupported,
        }
        atom_exprs.push(violation);
    }

    let decode_cnf = cnf.clone();
    let mut solver = cnf.into_solver();
    let refs: Vec<&ProofExpr> = atom_exprs.iter().collect();
    for (k, &activation) in bad.iter().enumerate() {
        match solver.solve_under_assumptions(&[activation]) {
            SolveResult::Sat(model) => {
                return BmcOutcome::CounterexampleAt {
                    k: k as u32,
                    trace: decode_model(&decode_cnf, &model, &refs),
                }
            }
            SolveResult::Unsat => continue,
        }
    }
    BmcOutcome::NoneWithin(max_k)
}

// ── Temporal symmetry ──────────────────────────────────────────────────────────────────────────
//
// A `signal@t` atom names a base signal observed at a time frame. A permutation of the BASE signals
// that preserves the initial constraint, the (uniform) transition relation, and the property is a
// symmetry of the system at EVERY frame — so applying it uniformly across the whole unrolling is an
// automorphism of the BMC formula. Detecting it on the three single-frame obligations (not the giant
// flat unrolling) is the temporal, model-checking-specific move; breaking it at the initial frame
// prunes symmetric trajectories without changing whether a counterexample exists.

/// Split a `signal@t` atom into `(base, frame)`; `None` for atoms carrying no frame.
fn split_frame(a: &str) -> Option<(&str, &str)> {
    a.rsplit_once('@')
}

/// Rename a formula by swapping two base signals, preserving each atom's time frame.
fn swap_signals(e: &ProofExpr, a: &str, b: &str) -> ProofExpr {
    match e {
        ProofExpr::Atom(s) => match split_frame(s) {
            Some((base, frame)) => {
                let nb = if base == a {
                    b
                } else if base == b {
                    a
                } else {
                    base
                };
                ProofExpr::Atom(format!("{nb}@{frame}"))
            }
            None => e.clone(),
        },
        ProofExpr::Not(x) => not(swap_signals(x, a, b)),
        ProofExpr::And(x, y) => {
            ProofExpr::And(Box::new(swap_signals(x, a, b)), Box::new(swap_signals(y, a, b)))
        }
        ProofExpr::Or(x, y) => {
            ProofExpr::Or(Box::new(swap_signals(x, a, b)), Box::new(swap_signals(y, a, b)))
        }
        ProofExpr::Iff(x, y) => {
            ProofExpr::Iff(Box::new(swap_signals(x, a, b)), Box::new(swap_signals(y, a, b)))
        }
        ProofExpr::Implies(x, y) => {
            ProofExpr::Implies(Box::new(swap_signals(x, a, b)), Box::new(swap_signals(y, a, b)))
        }
        other => other.clone(),
    }
}

/// Are two propositional obligations logically equivalent? (Their XOR is certified-UNSAT.) Semantic, so
/// it recognises symmetries a purely syntactic clause-set comparison would miss.
fn equivalent(e: &ProofExpr, f: &ProofExpr) -> bool {
    let xor = ProofExpr::Or(
        Box::new(ProofExpr::And(Box::new(e.clone()), Box::new(not(f.clone())))),
        Box::new(ProofExpr::And(Box::new(f.clone()), Box::new(not(e.clone())))),
    );
    matches!(prove_unsat(&xor), UnsatOutcome::Refuted)
}

/// Accumulate the base signals appearing in a formula.
fn base_signals(e: &ProofExpr, out: &mut std::collections::BTreeSet<String>) {
    match e {
        ProofExpr::Atom(s) => {
            if let Some((base, _)) = split_frame(s) {
                out.insert(base.to_string());
            }
        }
        ProofExpr::Not(x) => base_signals(x, out),
        ProofExpr::And(x, y)
        | ProofExpr::Or(x, y)
        | ProofExpr::Iff(x, y)
        | ProofExpr::Implies(x, y) => {
            base_signals(x, out);
            base_signals(y, out);
        }
        _ => {}
    }
}

/// The **temporal symmetries** of a transition system: pairs of base signals interchangeable across all
/// time — a swap that preserves the initial constraint, the transition relation (checked on the generic
/// frame `trans(0)`, hence on every frame by uniformity), and the property. Each surviving pair is a
/// genuine system automorphism when lifted uniformly through the unrolling.
pub fn temporal_symmetry_pairs(
    init: &ProofExpr,
    trans0: &ProofExpr,
    property0: &ProofExpr,
) -> Vec<(String, String)> {
    let mut set = std::collections::BTreeSet::new();
    base_signals(init, &mut set);
    base_signals(trans0, &mut set);
    base_signals(property0, &mut set);
    let sigs: Vec<String> = set.into_iter().collect();
    let mut pairs = Vec::new();
    for i in 0..sigs.len() {
        for j in (i + 1)..sigs.len() {
            let (a, b) = (sigs[i].as_str(), sigs[j].as_str());
            if equivalent(init, &swap_signals(init, a, b))
                && equivalent(trans0, &swap_signals(trans0, a, b))
                && equivalent(property0, &swap_signals(property0, a, b))
            {
                pairs.push((a.to_string(), b.to_string()));
            }
        }
    }
    pairs
}

/// Bounded model checking with **temporal symmetry breaking**. Interchangeable signals
/// ([`temporal_symmetry_pairs`]) give a system automorphism that holds at every frame, so a swap lifted
/// uniformly through the unrolling permutes counterexample trajectories. We order each pair at the initial
/// frame (`a@0 ≤ b@0`, i.e. `¬a@0 ∨ b@0`): a sound partial lex-leader that keeps at least one trajectory
/// per symmetry orbit. Hence a counterexample exists under the ordering iff one exists at all, and the
/// shallowest violating depth is unchanged — the verdict matches [`find_counterexample`], with the
/// symmetric search space pruned.
pub fn find_counterexample_symmetric(
    init: &ProofExpr,
    trans: &dyn Fn(u32) -> ProofExpr,
    property: &dyn Fn(u32) -> ProofExpr,
    max_k: u32,
) -> BmcOutcome {
    let pairs = temporal_symmetry_pairs(init, &trans(0), &property(0));
    let breaks: Vec<ProofExpr> = pairs
        .iter()
        .map(|(a, b)| {
            ProofExpr::Or(
                Box::new(not(ProofExpr::Atom(format!("{a}@0")))),
                Box::new(ProofExpr::Atom(format!("{b}@0"))),
            )
        })
        .collect();
    for k in 0..=max_k {
        let mut parts = unrolled_path(init, trans, k);
        parts.push(not(property(k)));
        parts.extend(breaks.iter().cloned());
        match prove_unsat(&conj(parts)) {
            UnsatOutcome::Sat(trace) => return BmcOutcome::CounterexampleAt { k, trace },
            UnsatOutcome::Refuted => continue,
            UnsatOutcome::Unsupported => return BmcOutcome::Unsupported,
        }
    }
    BmcOutcome::NoneWithin(max_k)
}

/// The verdict of k-induction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InductionOutcome {
    /// Base and step both certified — the property holds for ALL reachable states (unbounded).
    Proven,
    /// The base case found a real violation within the first `k` steps.
    CounterexampleAt { k: u32, trace: Vec<(String, bool)> },
    /// The base holds but the property is not `k`-inductive — try a larger `k` (or strengthen).
    NotInductive,
    /// An obligation left the propositional fragment — escalate.
    Unsupported,
}

/// k-induction: prove `property` is an invariant of the transition system.
///
/// - **Base**: for each `j < k`, `init ∧ path(j) ∧ ¬property(j)` is certified-UNSAT (no
///   violation in the first `k` steps; a SAT here is a genuine counterexample).
/// - **Step**: `property(0..k) ∧ trans(0..k) ∧ ¬property(k)` is certified-UNSAT (the property
///   is `k`-inductive).
///
/// Both certified ⇒ [`InductionOutcome::Proven`] for the unbounded system.
pub fn prove_invariant(
    init: &ProofExpr,
    trans: &dyn Fn(u32) -> ProofExpr,
    property: &dyn Fn(u32) -> ProofExpr,
    k: u32,
) -> InductionOutcome {
    // Base case: no violation reachable in the first k steps.
    for j in 0..k {
        let mut parts = unrolled_path(init, trans, j);
        parts.push(not(property(j)));
        match prove_unsat(&conj(parts)) {
            UnsatOutcome::Refuted => {}
            UnsatOutcome::Sat(trace) => {
                return InductionOutcome::CounterexampleAt { k: j, trace }
            }
            UnsatOutcome::Unsupported => return InductionOutcome::Unsupported,
        }
    }

    // Step case: property holds for k consecutive states ⇒ it holds at the next.
    let mut parts = Vec::new();
    for i in 0..k {
        parts.push(property(i));
        parts.push(trans(i));
    }
    parts.push(not(property(k)));
    match prove_unsat(&conj(parts)) {
        UnsatOutcome::Refuted => InductionOutcome::Proven,
        UnsatOutcome::Sat(_) => InductionOutcome::NotInductive,
        UnsatOutcome::Unsupported => InductionOutcome::Unsupported,
    }
}

/// The verdict of a vacuity check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VacuityOutcome {
    /// The antecedent is unsatisfiable — a dead trigger; the property holds vacuously.
    Vacuous,
    /// The antecedent can fire, with a witnessing assignment.
    Reachable(Vec<(String, bool)>),
    /// Not propositional — escalate.
    Unsupported,
}

/// Vacuity: can `antecedent` ever be satisfied? An unsatisfiable antecedent means the
/// property is never actually exercised (a vacuous pass / dead trigger).
pub fn check_vacuity(antecedent: &ProofExpr) -> VacuityOutcome {
    match prove_unsat(antecedent) {
        UnsatOutcome::Refuted => VacuityOutcome::Vacuous,
        UnsatOutcome::Sat(witness) => VacuityOutcome::Reachable(witness),
        UnsatOutcome::Unsupported => VacuityOutcome::Unsupported,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atom(s: &str) -> ProofExpr {
        ProofExpr::Atom(s.to_string())
    }
    fn iff(a: ProofExpr, b: ProofExpr) -> ProofExpr {
        ProofExpr::Iff(Box::new(a), Box::new(b))
    }

    // ── A latched-true register: x@0 = true, x@(t+1) ↔ x@t. Property: x is always true.
    fn latched_init() -> ProofExpr {
        atom("x@0")
    }
    fn latched_trans(t: u32) -> ProofExpr {
        iff(atom(&format!("x@{}", t + 1)), atom(&format!("x@{}", t)))
    }
    fn latched_prop(t: u32) -> ProofExpr {
        atom(&format!("x@{}", t))
    }

    // ── A toggle: q@0 = false, q@(t+1) ↔ ¬q@t. The (false) property "q is always false".
    fn toggle_init() -> ProofExpr {
        not(atom("q@0"))
    }
    fn toggle_trans(t: u32) -> ProofExpr {
        iff(atom(&format!("q@{}", t + 1)), not(atom(&format!("q@{}", t))))
    }
    fn toggle_always_false(t: u32) -> ProofExpr {
        not(atom(&format!("q@{}", t)))
    }

    #[test]
    fn bmc_finds_toggle_violation_at_step_one() {
        // "q always false" is violated: q@1 = ¬q@0 = true. BMC must find it at depth 1.
        let out = find_counterexample(&toggle_init(), &toggle_trans, &toggle_always_false, 5);
        match out {
            BmcOutcome::CounterexampleAt { k, trace } => {
                assert_eq!(k, 1, "shallowest violation is at step 1");
                assert!(
                    trace.iter().any(|(n, v)| n == "q@1" && *v),
                    "trace must show q@1 high: {trace:?}"
                );
            }
            other => panic!("expected a counterexample, got {other:?}"),
        }
    }

    #[test]
    fn bmc_no_counterexample_for_latched_invariant() {
        let out = find_counterexample(&latched_init(), &latched_trans, &latched_prop, 6);
        assert_eq!(out, BmcOutcome::NoneWithin(6));
    }

    #[test]
    fn k_induction_proves_latched_invariant() {
        // x is latched true and starts true ⇒ always true. 1-inductive.
        let out = prove_invariant(&latched_init(), &latched_trans, &latched_prop, 1);
        assert_eq!(out, InductionOutcome::Proven);
    }

    #[test]
    fn k_induction_finds_toggle_counterexample_in_base() {
        // "q always false" is false; the base case must surface the violation, not "Proven".
        let out = prove_invariant(&toggle_init(), &toggle_trans, &toggle_always_false, 3);
        match out {
            InductionOutcome::CounterexampleAt { k, .. } => assert_eq!(k, 1),
            other => panic!("expected a base-case counterexample, got {other:?}"),
        }
    }

    #[test]
    fn vacuity_detects_dead_trigger() {
        // An antecedent that contradicts itself never fires → vacuous.
        let dead = ProofExpr::And(Box::new(atom("req@0")), Box::new(not(atom("req@0"))));
        assert_eq!(check_vacuity(&dead), VacuityOutcome::Vacuous);
    }

    #[test]
    fn vacuity_accepts_a_live_trigger() {
        match check_vacuity(&atom("req@0")) {
            VacuityOutcome::Reachable(w) => {
                assert!(w.iter().any(|(n, v)| n == "req@0" && *v));
            }
            other => panic!("a live trigger must be Reachable, got {other:?}"),
        }
    }

    // ── Incremental BMC (reuses learned clauses across depths via assumptions). ──

    #[test]
    fn incremental_bmc_finds_toggle_violation_at_step_one() {
        match find_counterexample_incremental(&toggle_init(), &toggle_trans, &toggle_always_false, 5) {
            BmcOutcome::CounterexampleAt { k, trace } => {
                assert_eq!(k, 1);
                assert!(trace.iter().any(|(n, v)| n == "q@1" && *v), "trace: {trace:?}");
            }
            other => panic!("expected a counterexample, got {other:?}"),
        }
    }

    #[test]
    fn temporal_symmetry_detected_and_broken_in_bmc() {
        // Two interchangeable latches s0, s1. init: at least one on. property: not both on. The system is
        // symmetric under swapping s0 ↔ s1 at EVERY time frame.
        let sym_init = ProofExpr::Or(Box::new(atom("s0@0")), Box::new(atom("s1@0")));
        let sym_trans = |t: u32| {
            conj(vec![
                iff(atom(&format!("s0@{}", t + 1)), atom(&format!("s0@{}", t))),
                iff(atom(&format!("s1@{}", t + 1)), atom(&format!("s1@{}", t))),
            ])
        };
        let not_both = |t: u32| {
            not(ProofExpr::And(
                Box::new(atom(&format!("s0@{}", t))),
                Box::new(atom(&format!("s1@{}", t))),
            ))
        };

        // The temporal symmetry is detected: s0, s1 are interchangeable across time.
        let pairs = temporal_symmetry_pairs(&sym_init, &sym_trans(0), &not_both(0));
        assert_eq!(pairs, vec![("s0".to_string(), "s1".to_string())], "s0 ↔ s1 is a temporal symmetry");

        // Counterexample (both latches on at t=0 violates "not both") — symmetric BMC agrees with plain BMC.
        let plain = find_counterexample(&sym_init, &sym_trans, &not_both, 4);
        let broken = find_counterexample_symmetric(&sym_init, &sym_trans, &not_both, 4);
        assert!(matches!(plain, BmcOutcome::CounterexampleAt { k: 0, .. }), "plain: {plain:?}");
        assert!(matches!(broken, BmcOutcome::CounterexampleAt { k: 0, .. }), "broken: {broken:?}");

        // No counterexample for the same symmetric system with a satisfied property ("at least one on"):
        // the symmetry break must not turn a NoneWithin into a spurious counterexample.
        let at_least_one =
            |t: u32| ProofExpr::Or(Box::new(atom(&format!("s0@{}", t))), Box::new(atom(&format!("s1@{}", t))));
        assert_eq!(
            find_counterexample_symmetric(&sym_init, &sym_trans, &at_least_one, 5),
            find_counterexample(&sym_init, &sym_trans, &at_least_one, 5),
            "symmetric BMC matches plain BMC on a held invariant"
        );

        // An asymmetric system (the single-signal toggle) has no interchangeable signals, and the symmetric
        // search then coincides exactly with plain BMC.
        assert!(
            temporal_symmetry_pairs(&toggle_init(), &toggle_trans(0), &toggle_always_false(0)).is_empty(),
            "a one-signal system has no signal-swap symmetry"
        );
        assert_eq!(
            find_counterexample_symmetric(&toggle_init(), &toggle_trans, &toggle_always_false, 5),
            find_counterexample(&toggle_init(), &toggle_trans, &toggle_always_false, 5),
            "with no symmetry, symmetric BMC = plain BMC"
        );
    }

    #[test]
    fn incremental_bmc_no_counterexample_for_latched_invariant() {
        assert_eq!(
            find_counterexample_incremental(&latched_init(), &latched_trans, &latched_prop, 6),
            BmcOutcome::NoneWithin(6)
        );
    }

    fn sig(i: usize, t: u32) -> ProofExpr {
        ProofExpr::Atom(format!("s{i}@{t}"))
    }
    fn lit_b(b: bool, e: ProofExpr) -> ProofExpr {
        if b {
            e
        } else {
            ProofExpr::Not(Box::new(e))
        }
    }

    #[test]
    fn incremental_matches_nonincremental_and_simulation() {
        // Random DETERMINISTIC functional systems over `n` boolean signals: a fixed initial
        // state and `next_i = (maybe ¬) current_{src_i}`. The unique trace is computed by
        // direct simulation (the oracle); BOTH BMC variants must report the first depth it
        // violates the property — and agree with each other.
        let mut state = 0xb5ad_4ece_da1c_e2a9u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let n = 3usize;
        let max_k = 5u32;
        for _trial in 0..120 {
            let init_bits: Vec<bool> = (0..n).map(|_| next() & 1 == 0).collect();
            let src: Vec<usize> = (0..n).map(|_| (next() % n as u64) as usize).collect();
            let inv: Vec<bool> = (0..n).map(|_| next() & 1 == 0).collect();
            let p = (next() % n as u64) as usize;
            let want = next() & 1 == 0;

            let init_expr = conj((0..n).map(|i| lit_b(init_bits[i], sig(i, 0))).collect());
            let (src_t, inv_t) = (src.clone(), inv.clone());
            let trans = move |t: u32| {
                conj((0..n)
                    .map(|i| {
                        // s_i@(t+1) ↔ (inv_i ? : ¬) s_{src_i}@t
                        let rhs = lit_b(inv_t[i], sig(src_t[i], t));
                        ProofExpr::Iff(Box::new(sig(i, t + 1)), Box::new(rhs))
                    })
                    .collect())
            };
            let prop = move |t: u32| lit_b(want, sig(p, t));

            // Oracle: simulate the unique trace, find the first violating depth.
            let mut cur = init_bits.clone();
            let mut expected: Option<u32> = None;
            for k in 0..=max_k {
                if (cur[p] == want) == false {
                    expected = Some(k);
                    break;
                }
                let nxt: Vec<bool> = (0..n)
                    .map(|i| if inv[i] { cur[src[i]] } else { !cur[src[i]] })
                    .collect();
                cur = nxt;
            }

            let k_of = |o: &BmcOutcome| match o {
                BmcOutcome::CounterexampleAt { k, .. } => Some(*k),
                BmcOutcome::NoneWithin(_) => None,
                BmcOutcome::Unsupported => panic!("unexpected Unsupported"),
            };
            let ni = find_counterexample(&init_expr, &trans, &prop, max_k);
            let inc = find_counterexample_incremental(&init_expr, &trans, &prop, max_k);
            assert_eq!(k_of(&ni), expected, "non-incremental BMC disagrees with simulation");
            assert_eq!(k_of(&inc), expected, "incremental BMC disagrees with simulation");
        }
    }
}
