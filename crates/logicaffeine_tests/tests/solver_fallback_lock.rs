//! solver_fallback_lock.rs — Invariant X (Graceful under adversity), solver axis, cross-checked
//! against an independent oracle per Invariant IV.
//!
//! The single most dangerous failure a proof engine can have
//! is a *silent wrong answer*: reporting UNSAT for a satisfiable formula (a false proof) or handing
//! back a "model" for an unsatisfiable one (a false counterexample). Under resource starvation the
//! only sound behaviors are the CORRECT verdict or an honest give-up — never a fabricated one.
//!
//! This lock proves that directly. It never grades the engine against itself: every verdict from
//! the native engine (`prove_unsat`, and the conflict-budgeted CDCL core) is cross-checked against a
//! brute-force truth-table oracle that is obviously correct by construction — it simply evaluates
//! the formula under all `2^k` assignments. Over a curated battery, structured pigeonhole instances,
//! and a large deterministic random battery, the invariants are:
//!   * `prove_unsat` NEVER returns `Refuted` for a satisfiable formula   (no false proof),
//!   * `prove_unsat` NEVER returns a model for an unsatisfiable formula   (no false counterexample),
//!   * any model it returns actually satisfies the formula,
//!   * `Unsupported` (honest "cannot decide") is always permitted, and
//!   * a conflict-STARVED budgeted solve returns only the correct verdict or an honest `Budget`.
//!
//! The engine here is the default, Z3-free native path — so this is also the "solver absent"
//! fallback: with no external oracle available, the engine still never lies.

use std::collections::{BTreeMap, BTreeSet};

use logicaffeine_proof::cdcl::BudgetedResult;
use logicaffeine_proof::cnf::Cnf;
use logicaffeine_proof::sat::{prove_unsat, UnsatOutcome};
use logicaffeine_proof::ProofExpr;

// ───────────────────────────── the independent oracle ─────────────────────────────

/// Evaluate a propositional `ProofExpr` under an assignment (unlisted atoms are `false`; the engine
/// only ever omits genuine don't-cares, which any value — including `false` — satisfies).
fn eval(e: &ProofExpr, asg: &BTreeMap<String, bool>) -> bool {
    match e {
        ProofExpr::Atom(a) => *asg.get(a).unwrap_or(&false),
        ProofExpr::Not(x) => !eval(x, asg),
        ProofExpr::And(a, b) => eval(a, asg) && eval(b, asg),
        ProofExpr::Or(a, b) => eval(a, asg) || eval(b, asg),
        ProofExpr::Implies(a, b) => !eval(a, asg) || eval(b, asg),
        ProofExpr::Iff(a, b) => eval(a, asg) == eval(b, asg),
        other => unreachable!(
            "this lock's oracle covers only the propositional fragment it builds; \
                               got {other:?}"
        ),
    }
}

fn collect_atoms(e: &ProofExpr, out: &mut BTreeSet<String>) {
    match e {
        ProofExpr::Atom(a) => {
            out.insert(a.clone());
        }
        ProofExpr::Not(x) => collect_atoms(x, out),
        ProofExpr::And(a, b)
        | ProofExpr::Or(a, b)
        | ProofExpr::Implies(a, b)
        | ProofExpr::Iff(a, b) => {
            collect_atoms(a, out);
            collect_atoms(b, out);
        }
        other => unreachable!("unexpected node in oracle atom scan: {other:?}"),
    }
}

/// The obviously-correct verdict: a satisfying assignment if one exists (exhaustive over the atoms),
/// else `None` for unsatisfiable. Capped at 20 atoms (`2^20` assignments) — every formula this lock
/// builds stays well under that.
fn brute_force(e: &ProofExpr) -> Option<BTreeMap<String, bool>> {
    let atoms: Vec<String> = {
        let mut s = BTreeSet::new();
        collect_atoms(e, &mut s);
        s.into_iter().collect()
    };
    assert!(
        atoms.len() <= 20,
        "oracle would enumerate 2^{} assignments",
        atoms.len()
    );
    for mask in 0u64..(1u64 << atoms.len()) {
        let asg: BTreeMap<String, bool> = atoms
            .iter()
            .enumerate()
            .map(|(i, a)| (a.clone(), (mask >> i) & 1 == 1))
            .collect();
        if eval(e, &asg) {
            return Some(asg);
        }
    }
    None
}

// ───────────────────────────── ProofExpr constructors ─────────────────────────────

fn atom(s: &str) -> ProofExpr {
    ProofExpr::Atom(s.to_string())
}
fn not(e: ProofExpr) -> ProofExpr {
    ProofExpr::Not(Box::new(e))
}
fn and(a: ProofExpr, b: ProofExpr) -> ProofExpr {
    ProofExpr::And(Box::new(a), Box::new(b))
}
fn or(a: ProofExpr, b: ProofExpr) -> ProofExpr {
    ProofExpr::Or(Box::new(a), Box::new(b))
}
fn implies(a: ProofExpr, b: ProofExpr) -> ProofExpr {
    ProofExpr::Implies(Box::new(a), Box::new(b))
}
fn iff(a: ProofExpr, b: ProofExpr) -> ProofExpr {
    ProofExpr::Iff(Box::new(a), Box::new(b))
}
fn and_all(mut v: Vec<ProofExpr>) -> ProofExpr {
    let mut acc = v.pop().expect("and_all needs at least one conjunct");
    while let Some(e) = v.pop() {
        acc = and(e, acc);
    }
    acc
}
fn or_all(mut v: Vec<ProofExpr>) -> ProofExpr {
    let mut acc = v.pop().expect("or_all needs at least one disjunct");
    while let Some(e) = v.pop() {
        acc = or(e, acc);
    }
    acc
}

/// Pigeonhole PHP(pigeons, holes): every pigeon sits in some hole, no hole holds two. Unsatisfiable
/// iff `pigeons > holes`. Exercises `prove_unsat`'s pigeonhole/cardinality fast-paths (which decide
/// in polynomial time where CDCL blows up) — and the oracle keeps them honest.
fn php(pigeons: usize, holes: usize) -> ProofExpr {
    let x = |p: usize, h: usize| atom(&format!("x{p}_{h}"));
    let mut clauses: Vec<ProofExpr> = Vec::new();
    for p in 0..pigeons {
        clauses.push(or_all((0..holes).map(|h| x(p, h)).collect()));
    }
    for h in 0..holes {
        for p in 0..pigeons {
            for q in (p + 1)..pigeons {
                clauses.push(or(not(x(p, h)), not(x(q, h))));
            }
        }
    }
    and_all(clauses)
}

// ───────────────────────────── the soundness cross-check ─────────────────────────────

/// Assert the engine's verdict on `e` is consistent with the exhaustive oracle. A false proof or a
/// false/invalid model is a P0 panic; `Unsupported` is always allowed.
fn assert_engine_never_lies(e: &ProofExpr) {
    let oracle = brute_force(e);
    match (prove_unsat(e), &oracle) {
        (UnsatOutcome::Refuted, Some(model)) => panic!(
            "FALSE PROOF: prove_unsat reported UNSAT, but the assignment {model:?} satisfies {e:?}",
        ),
        (UnsatOutcome::Sat(_), None) => {
            panic!("FALSE MODEL: prove_unsat returned a model for the unsatisfiable formula {e:?}")
        }
        (UnsatOutcome::Sat(model), Some(_)) => {
            let asg: BTreeMap<String, bool> = model.iter().cloned().collect();
            assert!(
                eval(e, &asg),
                "prove_unsat returned a NON-satisfying model {model:?} for {e:?}",
            );
        }
        (UnsatOutcome::Refuted, None) => {}  // correct UNSAT
        (UnsatOutcome::Unsupported, _) => {} // honest "cannot decide" — always permitted
    }
}

#[test]
fn prove_unsat_never_lies_on_the_curated_battery() {
    let battery = vec![
        and(atom("A"), not(atom("A"))), // UNSAT
        or(atom("A"), not(atom("A"))),  // tautology (SAT)
        and(atom("A"), atom("B")),      // SAT
        // (A∨B) ∧ ¬A ∧ ¬B — UNSAT
        and_all(vec![
            or(atom("A"), atom("B")),
            not(atom("A")),
            not(atom("B")),
        ]),
        // De Morgan: ¬(A∧B) ↔ (¬A∨¬B) — tautology
        iff(
            not(and(atom("A"), atom("B"))),
            or(not(atom("A")), not(atom("B"))),
        ),
        // A→B, A, ¬B — UNSAT (modus ponens denied)
        and_all(vec![
            implies(atom("A"), atom("B")),
            atom("A"),
            not(atom("B")),
        ]),
        // Contradiction hidden behind an implication chain, SAT overall
        or(implies(atom("A"), atom("B")), atom("C")),
        // XOR is satisfiable; XOR ∧ its negation is not
        iff(atom("A"), not(atom("B"))),
        and(iff(atom("A"), not(atom("B"))), iff(atom("A"), atom("B"))), // UNSAT
    ];
    for e in &battery {
        assert_engine_never_lies(e);
    }
}

#[test]
fn prove_unsat_never_lies_on_pigeonhole_instances() {
    // Unsatisfiable side (pigeons > holes) — must be Refuted or Unsupported, never a model.
    for (p, h) in [(2, 1), (3, 2), (4, 3), (5, 4)] {
        assert_engine_never_lies(&php(p, h));
    }
    // Satisfiable side (pigeons ≤ holes) — must never be Refuted.
    for (p, h) in [(1, 1), (2, 2), (2, 3), (3, 3)] {
        assert_engine_never_lies(&php(p, h));
    }
}

// ───────────────────────────── deterministic random battery ─────────────────────────────

struct SplitMix64(u64);
impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// A random propositional formula over `atoms` atoms, depth-bounded so the oracle stays cheap.
fn random_formula(rng: &mut SplitMix64, atoms: u64, depth: u32) -> ProofExpr {
    if depth == 0 || rng.below(3) == 0 {
        let name = (b'A' + rng.below(atoms) as u8) as char;
        let leaf = atom(&name.to_string());
        return if rng.below(2) == 0 { leaf } else { not(leaf) };
    }
    match rng.below(5) {
        0 => not(random_formula(rng, atoms, depth - 1)),
        1 => and(
            random_formula(rng, atoms, depth - 1),
            random_formula(rng, atoms, depth - 1),
        ),
        2 => or(
            random_formula(rng, atoms, depth - 1),
            random_formula(rng, atoms, depth - 1),
        ),
        3 => implies(
            random_formula(rng, atoms, depth - 1),
            random_formula(rng, atoms, depth - 1),
        ),
        _ => iff(
            random_formula(rng, atoms, depth - 1),
            random_formula(rng, atoms, depth - 1),
        ),
    }
}

#[test]
fn prove_unsat_never_lies_on_a_random_battery() {
    let mut rng = SplitMix64(0x5EED_C0FFEE_u64 ^ 0x1234_5678);
    // 6 atoms → 64-row oracle; 2000 formulas of depth ≤ 6. Small enough that every instance decides
    // far under prove_unsat's easy-search budget, so the whole battery runs in well under a second.
    for _ in 0..2000 {
        let e = random_formula(&mut rng, 6, 6);
        assert_engine_never_lies(&e);
    }
}

// ───────────────────────────── budget-starvation soundness ─────────────────────────────

/// The literal timeout axis: a conflict-STARVED budgeted solve may only return the CORRECT verdict
/// or an honest `Budget` — never the wrong one. This is where a lesser engine, pressed for time,
/// would fabricate an answer.
#[test]
fn budgeted_solve_never_fabricates_a_verdict_under_starvation() {
    let cases: Vec<(ProofExpr, bool)> = vec![
        (and(atom("A"), not(atom("A"))), false), // UNSAT
        (or(atom("A"), atom("B")), true),        // SAT
        (
            and_all(vec![
                or(atom("A"), atom("B")),
                not(atom("A")),
                not(atom("B")),
            ]),
            false,
        ), // UNSAT
        (implies(atom("A"), atom("B")), true),   // SAT
        (php(3, 2), false),                      // UNSAT (hard)
        (php(4, 3), false),                      // UNSAT (harder)
        (php(3, 3), true),                       // SAT
    ];
    for (e, expect_sat) in &cases {
        // Fresh solver per budget: solve_budgeted mutates solver state.
        for &budget in &[0u64, 1, 5, 50] {
            let mut cnf = Cnf::new();
            if cnf.assert(e).is_none() {
                continue; // not CNF-assertable → not this test's concern
            }
            let mut solver = cnf.into_solver();
            match solver.solve_budgeted(budget) {
                BudgetedResult::Sat(_) => assert!(
                    *expect_sat,
                    "budgeted solve fabricated SAT on an UNSAT formula (budget {budget}): {e:?}",
                ),
                BudgetedResult::Unsat => assert!(
                    !*expect_sat,
                    "budgeted solve fabricated UNSAT on a SAT formula (budget {budget}): {e:?}",
                ),
                BudgetedResult::Budget => {} // honest give-up — always sound
            }
        }
    }
}
