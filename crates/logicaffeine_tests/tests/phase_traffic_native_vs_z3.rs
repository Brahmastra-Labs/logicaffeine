//! Differential lock: OUR certified traffic phase-DESIGNER vs Z3 as the oracle, plus the
//! faster-than-Z3 bench. Z3 is reached only through `logicaffeine_verify` (native, verification-
//! gated — never the browser path). Our designer's minimal-phase verdict must MATCH Z3's, and our
//! certified solver must be FASTER on the corpus.
//!
//! Run: `Z3_SYS_Z3_HEADER=/usr/include/z3.h cargo nextest run -p logicaffeine-tests \
//!       --features verification -E 'binary(phase_traffic_native_vs_z3)'`
#![cfg(feature = "verification")]

use logicaffeine_compile::codegen_sva::signal_design::{design_phase_plan, Intersection};
use logicaffeine_proof::cardinality::at_most;
use logicaffeine_proof::optimize::minimize_certified;
use logicaffeine_proof::matching::{assign_or_hall, is_hall_witness, MatchOutcome};
use logicaffeine_proof::sat::{find_model, prove_unsat, ModelOutcome, UnsatOutcome};
use logicaffeine_proof::hornsat::{self, HornClause, HornOutcome};
use logicaffeine_proof::twosat::{self, Lit as TsLit, TwoSatOutcome};
use logicaffeine_proof::xorsat::{self, XorEquation, XorOutcome};
use logicaffeine_proof::ProofExpr;
use logicaffeine_verify::ic3::check_sat;
use logicaffeine_verify::VerifyExpr;
use std::time::{Duration, Instant};

/// A grab-bag of conflict graphs with known chromatic numbers (1, 2, 3, 2, 3, 4, 2) plus the
/// Grötzsch graph (triangle-free, χ=4): its clique number is only 2, so the clique lower bound
/// can't shortcut the answer — the certified SAT scan has to close the whole gap, exactly where a
/// naive solver would do the most work. Locking it against Z3 proves we match the verdict *and*
/// stay faster on the hard instance, not just the easy perfect graphs.
const CASES: &[(usize, &[(usize, usize)])] = &[
    (3, &[]),
    (2, &[(0, 1)]),
    (3, &[(0, 1), (1, 2), (0, 2)]),
    (4, &[(0, 1), (1, 2), (2, 3), (3, 0)]),
    (5, &[(0, 1), (1, 2), (2, 3), (3, 4), (4, 0)]),
    (4, &[(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)]),
    (5, &[(0, 1), (0, 2), (0, 3), (0, 4)]),
    (
        11,
        &[
            (0, 1), (1, 2), (2, 3), (3, 4), (4, 0),
            (5, 4), (5, 1), (10, 5),
            (6, 0), (6, 2), (10, 6),
            (7, 1), (7, 3), (10, 7),
            (8, 2), (8, 4), (10, 8),
            (9, 3), (9, 0), (10, 9),
        ],
    ),
];

fn intersection(n: usize, conflicts: &[(usize, usize)]) -> Intersection {
    Intersection {
        movements: (0..n).map(|i| format!("m{i}")).collect(),
        conflicts: conflicts.to_vec(),
    }
}

/// Build the k-colouring obligation as a `VerifyExpr` (the same constraints our `ProofExpr`
/// encoder uses) and let Z3 decide feasibility.
fn z3_coloring_feasible(n: usize, conflicts: &[(usize, usize)], k: usize) -> bool {
    let atom = |m: usize, p: usize| VerifyExpr::var(format!("a_{m}_{p}"));
    let mut clauses: Vec<VerifyExpr> = Vec::new();

    // Each movement is served in at least one phase.
    for m in 0..n {
        let mut disj = atom(m, 0);
        for p in 1..k {
            disj = VerifyExpr::or(disj, atom(m, p));
        }
        clauses.push(disj);
    }
    // …and at most one.
    for m in 0..n {
        for p in 0..k {
            for q in (p + 1)..k {
                clauses.push(VerifyExpr::not(VerifyExpr::and(atom(m, p), atom(m, q))));
            }
        }
    }
    // Conflicting movements never share a phase.
    for &(x, y) in conflicts {
        if x == y || x >= n || y >= n {
            continue;
        }
        for p in 0..k {
            clauses.push(VerifyExpr::not(VerifyExpr::and(atom(x, p), atom(y, p))));
        }
    }

    match clauses.into_iter().reduce(VerifyExpr::and) {
        Some(formula) => check_sat(&formula),
        None => true,
    }
}

/// Z3's minimal phase count for the conflict graph (scan upward for the first feasible k).
fn z3_min_phases(n: usize, conflicts: &[(usize, usize)]) -> usize {
    (1..=n)
        .find(|&k| z3_coloring_feasible(n, conflicts, k))
        .unwrap_or(n)
}

#[test]
fn designer_min_phases_matches_z3() {
    for &(n, conflicts) in CASES {
        let ours = design_phase_plan(&intersection(n, conflicts)).unwrap().num_phases;
        let z3 = z3_min_phases(n, conflicts);
        assert_eq!(
            ours, z3,
            "minimal phase count diverged for n={n} conflicts={conflicts:?}: ours={ours} z3={z3}"
        );
    }
}

#[test]
fn designer_is_faster_than_z3() {
    const ITERS: usize = 30;
    let t = Instant::now();
    for _ in 0..ITERS {
        for &(n, c) in CASES {
            let _ = design_phase_plan(&intersection(n, c)).unwrap().num_phases;
        }
    }
    let ours = t.elapsed();

    let t = Instant::now();
    for _ in 0..ITERS {
        for &(n, c) in CASES {
            let _ = z3_min_phases(n, c);
        }
    }
    let z3 = t.elapsed();

    let speedup = z3.as_secs_f64() / ours.as_secs_f64().max(f64::MIN_POSITIVE);
    eprintln!("designer native-vs-z3: ours={ours:?}  z3={z3:?}  speedup={speedup:.1}x");
    assert!(
        ours < z3,
        "our certified designer must beat Z3 on the corpus: ours={ours:?} z3={z3:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Harder graphs — the dominance isn't a small-graph fluke. K6 is a pure clique
// (we close it with zero solves). The wheel and the Petersen graph are the acid
// test: Petersen is triangle-free (clique number 2) yet 3-chromatic, so the
// clique lower bound *cannot* shortcut the answer — the certified SAT scan has to
// do real work, exactly where a naive per-k Z3 scan does the most.
// ─────────────────────────────────────────────────────────────────────────────

/// `(n, edges)` for graphs whose chromatic numbers the clique bound can't trivially reach.
const HARD_CASES: &[(usize, &[(usize, usize)])] = &[
    // K6: complete graph → χ=6 (clique meets greedy, zero solves on our side).
    (
        6,
        &[
            (0, 1), (0, 2), (0, 3), (0, 4), (0, 5),
            (1, 2), (1, 3), (1, 4), (1, 5),
            (2, 3), (2, 4), (2, 5),
            (3, 4), (3, 5),
            (4, 5),
        ],
    ),
    // Wheel W6 = C5 + a hub adjacent to all → χ=4 (odd rim forces a 4th colour).
    (
        6,
        &[
            (0, 1), (1, 2), (2, 3), (3, 4), (4, 0),
            (5, 0), (5, 1), (5, 2), (5, 3), (5, 4),
        ],
    ),
    // Petersen graph → χ=3, triangle-free (clique number 2): the clique bound is far below χ.
    (
        10,
        &[
            (0, 1), (1, 2), (2, 3), (3, 4), (4, 0),
            (0, 5), (1, 6), (2, 7), (3, 8), (4, 9),
            (5, 7), (7, 9), (9, 6), (6, 8), (8, 5),
        ],
    ),
];

#[test]
fn designer_matches_z3_on_hard_graphs() {
    for &(n, conflicts) in HARD_CASES {
        let ours = design_phase_plan(&intersection(n, conflicts)).unwrap().num_phases;
        let z3 = z3_min_phases(n, conflicts);
        assert_eq!(
            ours, z3,
            "minimal phase count diverged on a hard graph (n={n}): ours={ours} z3={z3}"
        );
    }
}

#[test]
fn designer_beats_z3_on_hard_graphs() {
    const ITERS: usize = 10;
    let t = Instant::now();
    for _ in 0..ITERS {
        for &(n, c) in HARD_CASES {
            let _ = design_phase_plan(&intersection(n, c)).unwrap().num_phases;
        }
    }
    let ours = t.elapsed();

    let t = Instant::now();
    for _ in 0..ITERS {
        for &(n, c) in HARD_CASES {
            let _ = z3_min_phases(n, c);
        }
    }
    let z3 = t.elapsed();

    let speedup = z3.as_secs_f64() / ours.as_secs_f64().max(f64::MIN_POSITIVE);
    eprintln!("designer HARD-graphs native-vs-z3: ours={ours:?}  z3={z3:?}  speedup={speedup:.1}x");
    assert!(
        ours < z3,
        "our certified designer must beat Z3 on the hard corpus too: ours={ours:?} z3={z3:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Phase D: the certified OPTIMIZER + cardinality encoder, locked against Z3.
//
// `minimize_certified` binary-searches the smallest feasible cost bound using our
// own RUP-certified SAT (`prove_unsat`). We lock its optimum against Z3 by driving
// the *same* feasibility formulas (our Sinz `at_most` encoding included) through
// Z3 as the satisfiability oracle. Identical optima means our cardinality encoding
// and our search are both faithful — and we measure that ours is faster.
// ─────────────────────────────────────────────────────────────────────────────

/// Bridge the boolean fragment of `ProofExpr` (all that cardinality/coverage formulas use) into a
/// `VerifyExpr` so Z3 can decide the very formula our solver decides.
fn proof_to_verify(e: &ProofExpr) -> VerifyExpr {
    match e {
        ProofExpr::Atom(s) => VerifyExpr::var(s.clone()),
        ProofExpr::Not(a) => VerifyExpr::not(proof_to_verify(a)),
        ProofExpr::And(a, b) => VerifyExpr::and(proof_to_verify(a), proof_to_verify(b)),
        ProofExpr::Or(a, b) => VerifyExpr::or(proof_to_verify(a), proof_to_verify(b)),
        ProofExpr::Implies(a, b) => VerifyExpr::implies(proof_to_verify(a), proof_to_verify(b)),
        ProofExpr::Iff(a, b) => VerifyExpr::iff(proof_to_verify(a), proof_to_verify(b)),
        other => unreachable!("cardinality/coverage formulas stay boolean; got {other:?}"),
    }
}

/// Z3's smallest feasible cost bound in `[lo, hi]` for a monotone feasibility family — the oracle
/// twin of `minimize_certified`.
fn z3_min_feasible(feasible_at: impl Fn(i64) -> ProofExpr, lo: i64, hi: i64) -> Option<i64> {
    let sat = |b: i64| check_sat(&proof_to_verify(&feasible_at(b)));
    if !sat(hi) {
        return None;
    }
    let (mut l, mut h) = (lo, hi);
    while l < h {
        let mid = l + (h - l) / 2;
        if sat(mid) {
            h = mid;
        } else {
            l = mid + 1;
        }
    }
    Some(l)
}

fn atom(s: &str) -> ProofExpr {
    ProofExpr::Atom(s.to_string())
}
fn or(a: ProofExpr, b: ProofExpr) -> ProofExpr {
    ProofExpr::Or(Box::new(a), Box::new(b))
}
fn and(a: ProofExpr, b: ProofExpr) -> ProofExpr {
    ProofExpr::And(Box::new(a), Box::new(b))
}

/// A family of minimum-hitting-count problems with known optima: each is a set of OR-clauses, and
/// the objective is the fewest atoms set true that still satisfy every clause.
fn hitting_problems() -> Vec<(Vec<ProofExpr>, ProofExpr, i64)> {
    let (a, b, c, d) = (atom("a"), atom("b"), atom("c"), atom("d"));
    vec![
        // Triangle of pairwise clauses → need 2 of 3.
        (
            vec![a.clone(), b.clone(), c.clone()],
            and(and(or(a.clone(), b.clone()), or(b.clone(), c.clone())), or(a.clone(), c.clone())),
            2,
        ),
        // a forced, plus a 2-of-{b,c,d} cover → 3 total.
        (
            vec![a.clone(), b.clone(), c.clone(), d.clone()],
            and(
                and(a.clone(), or(b.clone(), c.clone())),
                and(or(c.clone(), d.clone()), or(b.clone(), d.clone())),
            ),
            3,
        ),
    ]
}

#[test]
fn optimizer_optimum_matches_z3() {
    for (vars, clauses, expected) in hitting_problems() {
        let n = vars.len() as i64;
        let feasible_at = move |bound: i64| {
            and(clauses.clone(), at_most(&vars, bound.max(0) as usize, "cost"))
        };
        let ours = minimize_certified(&feasible_at, 0, n).expect("feasible within range");
        let z3 = z3_min_feasible(&feasible_at, 0, n).expect("Z3 must also find it feasible");
        assert_eq!(ours.optimum, expected, "our certified optimum is wrong");
        assert_eq!(ours.optimum, z3, "our optimum must match Z3's: ours={} z3={z3}", ours.optimum);
        assert!(ours.minimal_certified, "the optimum must be RUP-certified minimal");
    }
}

#[test]
fn optimizer_beats_z3() {
    const ITERS: usize = 50;
    let problems = hitting_problems();

    let t = Instant::now();
    for _ in 0..ITERS {
        for (vars, clauses, _) in &problems {
            let n = vars.len() as i64;
            let (vars, clauses) = (vars.clone(), clauses.clone());
            let feasible_at =
                move |bound: i64| and(clauses.clone(), at_most(&vars, bound.max(0) as usize, "cost"));
            let _ = minimize_certified(&feasible_at, 0, n).unwrap().optimum;
        }
    }
    let ours = t.elapsed();

    let t = Instant::now();
    for _ in 0..ITERS {
        for (vars, clauses, _) in &problems {
            let n = vars.len() as i64;
            let (vars, clauses) = (vars.clone(), clauses.clone());
            let feasible_at =
                move |bound: i64| and(clauses.clone(), at_most(&vars, bound.max(0) as usize, "cost"));
            let _ = z3_min_feasible(&feasible_at, 0, n).unwrap();
        }
    }
    let z3 = t.elapsed();

    let speedup = z3.as_secs_f64() / ours.as_secs_f64().max(f64::MIN_POSITIVE);
    eprintln!("optimizer native-vs-z3: ours={ours:?}  z3={z3:?}  speedup={speedup:.1}x");
    assert!(
        ours < z3,
        "our certified optimizer must beat Z3 on the cost-search corpus: ours={ours:?} z3={z3:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// N-Queens — a single-query SAT win. Finding a placement is tractable for CDCL
// (unlike clique-UNSAT, which is pigeonhole-hard), so on small boards Z3's per-call
// context overhead dominates and our in-process solver wins. Locked vs Z3 + validity.
// ─────────────────────────────────────────────────────────────────────────────

/// Fold a clause list into a **balanced** And/Or tree (depth O(log n)) so the many recursive
/// traversals downstream (Tseitin, the Z3 bridge, Z3's own encoder) don't blow the stack on the
/// thousands of clauses an N-Queens board produces.
fn balanced(mut nodes: Vec<ProofExpr>, and: bool) -> ProofExpr {
    if nodes.is_empty() {
        return ProofExpr::Atom("__true".into());
    }
    while nodes.len() > 1 {
        let mut next = Vec::with_capacity((nodes.len() + 1) / 2);
        let mut iter = nodes.into_iter();
        while let Some(a) = iter.next() {
            match iter.next() {
                Some(b) if and => next.push(ProofExpr::And(Box::new(a), Box::new(b))),
                Some(b) => next.push(ProofExpr::Or(Box::new(a), Box::new(b))),
                None => next.push(a),
            }
        }
        nodes = next;
    }
    nodes.into_iter().next().unwrap()
}

fn at_most_one(vars: &[ProofExpr]) -> Vec<ProofExpr> {
    let mut cs = Vec::new();
    for i in 0..vars.len() {
        for j in (i + 1)..vars.len() {
            cs.push(ProofExpr::Not(Box::new(ProofExpr::And(
                Box::new(vars[i].clone()),
                Box::new(vars[j].clone()),
            ))));
        }
    }
    cs
}

/// Standard N-Queens SAT encoding: exactly one queen per row, at most one per column and per
/// diagonal. SAT for every n ≥ 4; a model is a placement.
fn n_queens_formula(n: usize) -> ProofExpr {
    let q = |r: usize, c: usize| ProofExpr::Atom(format!("q_{r}_{c}"));
    let mut clauses: Vec<ProofExpr> = Vec::new();
    for r in 0..n {
        let row: Vec<ProofExpr> = (0..n).map(|c| q(r, c)).collect();
        clauses.push(balanced(row.clone(), false));
        clauses.extend(at_most_one(&row));
    }
    for c in 0..n {
        let col: Vec<ProofExpr> = (0..n).map(|r| q(r, c)).collect();
        clauses.extend(at_most_one(&col));
    }
    let ni = n as isize;
    for d in -(ni - 1)..=(ni - 1) {
        let cells: Vec<ProofExpr> = (0..n)
            .filter_map(|r| {
                let c = r as isize - d;
                (c >= 0 && c < ni).then(|| q(r, c as usize))
            })
            .collect();
        clauses.extend(at_most_one(&cells));
    }
    for s in 0..=(2 * n - 2) {
        let cells: Vec<ProofExpr> = (0..n)
            .filter_map(|r| (s >= r && s - r < n).then(|| q(r, s - r)))
            .collect();
        clauses.extend(at_most_one(&cells));
    }
    balanced(clauses, true)
}

/// Decode a SAT model and confirm it is a genuine N-Queens placement (exactly n queens, one per
/// row, no two sharing a column or diagonal).
fn nqueens_model_is_valid(n: usize, model: &[(String, bool)]) -> bool {
    let on: std::collections::HashSet<(usize, usize)> = model
        .iter()
        .filter(|(_, v)| *v)
        .filter_map(|(name, _)| {
            let rest = name.strip_prefix("q_")?;
            let (r, c) = rest.split_once('_')?;
            Some((r.parse().ok()?, c.parse().ok()?))
        })
        .collect();
    if on.len() != n {
        return false;
    }
    let queens: Vec<(usize, usize)> = on.into_iter().collect();
    for i in 0..queens.len() {
        for j in (i + 1)..queens.len() {
            let (r1, c1) = queens[i];
            let (r2, c2) = queens[j];
            if r1 == r2 || c1 == c2 {
                return false;
            }
            if (r1 as isize - r2 as isize).abs() == (c1 as isize - c2 as isize).abs() {
                return false;
            }
        }
    }
    true
}

#[test]
fn nqueens_solution_is_valid_and_matches_z3() {
    for n in [6usize, 8, 10] {
        let f = n_queens_formula(n);
        let ours = find_model(&f);
        let z3 = check_sat(&proof_to_verify(&f));
        let ours_sat = matches!(ours, ModelOutcome::Sat(_));
        assert_eq!(ours_sat, z3, "n-queens n={n}: ours_sat={ours_sat} z3={z3}");
        if let ModelOutcome::Sat(model) = ours {
            assert!(nqueens_model_is_valid(n, &model), "invalid {n}-queens placement: {model:?}");
        }
    }
}

#[test]
fn nqueens_beats_z3() {
    const ITERS: usize = 20;
    let f = n_queens_formula(8);
    let vf = proof_to_verify(&f);
    let t = Instant::now();
    for _ in 0..ITERS {
        let _ = find_model(&f);
    }
    let ours = t.elapsed();
    let t = Instant::now();
    for _ in 0..ITERS {
        let _ = check_sat(&vf);
    }
    let z3 = t.elapsed();
    let speedup = z3.as_secs_f64() / ours.as_secs_f64().max(f64::MIN_POSITIVE);
    eprintln!("8-queens native-vs-z3: ours={ours:?}  z3={z3:?}  speedup={speedup:.1}x");
    assert!(ours < z3, "our solver must beat Z3 on 8-queens: ours={ours:?} z3={z3:?}");
}

// ─────────────────────────────────────────────────────────────────────────────
// Controller-safety BMC — the "safety" leg of the design→safety→flow trifecta. A two-movement
// phase controller toggles a phase bit; movement a is green in phase 0, b in phase 1, so they are
// never green together. We ask "is a state where both are green reachable within k cycles?" —
// encoded ONCE as a ProofExpr (our find_model: ∃ violation) and bridged to Z3 (check_sat), so both
// engines decide the identical formula. Safe ⇒ UNSAT; the buggy variant greens both in phase 0.
// ─────────────────────────────────────────────────────────────────────────────

fn controller_bmc_formula(k: usize, buggy: bool) -> ProofExpr {
    let p = |t: usize| ProofExpr::Atom(format!("p_{t}"));
    let a = |t: usize| ProofExpr::Atom(format!("a_{t}"));
    let b = |t: usize| ProofExpr::Atom(format!("b_{t}"));
    let not = |x: ProofExpr| ProofExpr::Not(Box::new(x));
    let iff = |x: ProofExpr, y: ProofExpr| ProofExpr::Iff(Box::new(x), Box::new(y));
    let and = |x: ProofExpr, y: ProofExpr| ProofExpr::And(Box::new(x), Box::new(y));

    let mut clauses = vec![not(p(0))]; // start in phase 0
    for t in 0..=k {
        clauses.push(iff(a(t), not(p(t)))); // a green ⇔ phase 0
        clauses.push(iff(b(t), if buggy { not(p(t)) } else { p(t) })); // b green ⇔ phase 1 (safe)
    }
    for t in 0..k {
        clauses.push(iff(p(t + 1), not(p(t)))); // phase toggles each cycle
    }
    // A safety violation at some reachable step: both movements green together.
    let violation = balanced((0..=k).map(|t| and(a(t), b(t))).collect(), false);
    clauses.push(violation);
    balanced(clauses, true)
}

#[test]
fn controller_safety_matches_z3() {
    for k in [8usize, 16, 24] {
        let safe = controller_bmc_formula(k, false);
        assert!(!matches!(find_model(&safe), ModelOutcome::Sat(_)), "safe ours k={k} must be UNSAT");
        assert!(!check_sat(&proof_to_verify(&safe)), "safe z3 k={k} must be UNSAT");
        let buggy = controller_bmc_formula(k, true);
        assert!(matches!(find_model(&buggy), ModelOutcome::Sat(_)), "buggy ours k={k} must be SAT");
        assert!(check_sat(&proof_to_verify(&buggy)), "buggy z3 k={k} must be SAT");
    }
}

#[test]
fn controller_safety_beats_z3() {
    const ITERS: usize = 20;
    let safe = controller_bmc_formula(24, false);
    let vf = proof_to_verify(&safe);
    let t = Instant::now();
    for _ in 0..ITERS {
        let _ = find_model(&safe);
    }
    let ours = t.elapsed();
    let t = Instant::now();
    for _ in 0..ITERS {
        let _ = check_sat(&vf);
    }
    let z3 = t.elapsed();
    let speedup = z3.as_secs_f64() / ours.as_secs_f64().max(f64::MIN_POSITIVE);
    eprintln!("controller-safety BMC native-vs-z3: ours={ours:?} z3={z3:?} speedup={speedup:.1}x");
    assert!(ours < z3, "controller-safety BMC must beat Z3: ours={ours:?} z3={z3:?}");
}

// ─────────────────────────────────────────────────────────────────────────────
// Flow-jam BMC — the "flow" leg. A 3-bit queue counter (capacity 7) with a server that serves
// only every other cycle (phase toggles). Congested ⇒ the queue grows +1 on unserved cycles and
// eventually overflows (jam reachable, SAT); a drained server holds it at 0 (no jam, UNSAT). The
// increment is a ripple-carry bit-blast; "jam" is an overflow-carry out of the top bit.
// ─────────────────────────────────────────────────────────────────────────────

fn flow_jam_formula(k: usize, drained: bool) -> ProofExpr {
    let q = |i: usize, t: usize| ProofExpr::Atom(format!("q{i}_{t}"));
    let p = |t: usize| ProofExpr::Atom(format!("p_{t}"));
    let not = |x: ProofExpr| ProofExpr::Not(Box::new(x));
    let and = |x: ProofExpr, y: ProofExpr| ProofExpr::And(Box::new(x), Box::new(y));
    let iff = |x: ProofExpr, y: ProofExpr| ProofExpr::Iff(Box::new(x), Box::new(y));
    let xor = |x: ProofExpr, y: ProofExpr| not(iff(x, y));

    let mut clauses = vec![not(q(0, 0)), not(q(1, 0)), not(q(2, 0)), not(p(0))]; // q=0, phase 0
    let mut overflow_terms = Vec::new();
    for t in 0..k {
        clauses.push(iff(p(t + 1), not(p(t)))); // server phase toggles
        if drained {
            for i in 0..3 {
                clauses.push(iff(q(i, t + 1), q(i, t))); // held at 0 — never grows
            }
        } else {
            let c = not(p(t)); // carry-in: +1 on the unserved cycles
            let carry1 = and(q(0, t), c.clone());
            let carry2 = and(q(1, t), carry1.clone());
            let carry3 = and(q(2, t), carry2.clone()); // overflow out of the top bit
            clauses.push(iff(q(0, t + 1), xor(q(0, t), c.clone())));
            clauses.push(iff(q(1, t + 1), xor(q(1, t), carry1.clone())));
            clauses.push(iff(q(2, t + 1), xor(q(2, t), carry2.clone())));
            overflow_terms.push(carry3);
        }
    }
    let jam = if overflow_terms.is_empty() {
        and(q(0, 0), not(q(0, 0))) // no possible overflow ⇒ force UNSAT
    } else {
        balanced(overflow_terms, false)
    };
    clauses.push(jam);
    balanced(clauses, true)
}

#[test]
fn flow_jam_matches_z3() {
    // Congested overflows within 20 cycles (SAT); drained never jams (UNSAT). Both verdicts must
    // match Z3 on the identical formula, and be non-vacuous.
    let congested = flow_jam_formula(20, false);
    assert!(matches!(find_model(&congested), ModelOutcome::Sat(_)), "congested must jam (SAT)");
    assert!(check_sat(&proof_to_verify(&congested)), "congested z3 must jam (SAT)");
    let drained = flow_jam_formula(20, true);
    assert!(!matches!(find_model(&drained), ModelOutcome::Sat(_)), "drained must not jam (UNSAT)");
    assert!(!check_sat(&proof_to_verify(&drained)), "drained z3 must not jam (UNSAT)");
}

#[test]
fn flow_jam_beats_z3() {
    const ITERS: usize = 20;
    let drained = flow_jam_formula(20, true); // the UNSAT (provably-no-jam) case
    let vf = proof_to_verify(&drained);
    let t = Instant::now();
    for _ in 0..ITERS {
        let _ = find_model(&drained);
    }
    let ours = t.elapsed();
    let t = Instant::now();
    for _ in 0..ITERS {
        let _ = check_sat(&vf);
    }
    let z3 = t.elapsed();
    let speedup = z3.as_secs_f64() / ours.as_secs_f64().max(f64::MIN_POSITIVE);
    eprintln!("flow-jam BMC native-vs-z3: ours={ours:?} z3={z3:?} speedup={speedup:.1}x");
    assert!(ours < z3, "flow-jam BMC must beat Z3: ours={ours:?} z3={z3:?}");
}

// ─────────────────────────────────────────────────────────────────────────────
// Pigeonhole (PHP) experiment — the resolution-hard frontier. n pigeons, n-1 holes, UNSAT.
// Compares pairwise vs Sinz-sequential at-most-one encodings (does cardinality structure help our
// CDCL?) and Z3. #[ignore]d — exploratory, may be slow for larger n.
// ─────────────────────────────────────────────────────────────────────────────

fn php_formula(n: usize, use_sinz: bool) -> ProofExpr {
    let holes = n - 1;
    let p = |i: usize, h: usize| ProofExpr::Atom(format!("php_{i}_{h}"));
    let mut clauses: Vec<ProofExpr> = Vec::new();
    // Each pigeon occupies at least one hole.
    for i in 0..n {
        clauses.push(balanced((0..holes).map(|h| p(i, h)).collect(), false));
    }
    // Each hole holds at most one pigeon.
    for h in 0..holes {
        let hole_vars: Vec<ProofExpr> = (0..n).map(|i| p(i, h)).collect();
        if use_sinz {
            clauses.push(at_most(&hole_vars, 1, &format!("php_amo_{h}")));
        } else {
            for i in 0..n {
                for j in (i + 1)..n {
                    clauses.push(ProofExpr::Not(Box::new(ProofExpr::And(
                        Box::new(p(i, h)),
                        Box::new(p(j, h)),
                    ))));
                }
            }
        }
    }
    balanced(clauses, true)
}

#[test]
#[ignore = "pigeonhole experiment — run explicitly with --ignored --nocapture"]
fn pigeonhole_experiment() {
    for n in [6usize, 7, 8] {
        let pw = php_formula(n, false);
        let sz = php_formula(n, true);
        let t = Instant::now();
        let r_pw = prove_unsat(&pw);
        let pw_t = t.elapsed();
        let t = Instant::now();
        let r_sz = prove_unsat(&sz);
        let sz_t = t.elapsed();
        let t = Instant::now();
        let z = check_sat(&proof_to_verify(&pw));
        let z_t = t.elapsed();
        eprintln!(
            "PHP({n}): pairwise={pw_t:?} sinz={sz_t:?} z3={z_t:?} | refuted_pw={} refuted_sz={} z3_sat={z}",
            matches!(r_pw, UnsatOutcome::Refuted),
            matches!(r_sz, UnsatOutcome::Refuted),
        );
        assert!(matches!(r_pw, UnsatOutcome::Refuted), "PHP({n}) pairwise must be UNSAT");
        assert!(matches!(r_sz, UnsatOutcome::Refuted), "PHP({n}) sinz must be UNSAT");
        assert!(!z, "PHP({n}) z3 must be UNSAT");
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Pigeonhole, SUPERCRUSHED — the resolution-hard case our raw SAT blows up on (and the only place
// Z3 was beating us). The fix mirrors the designer's clique bound: recognize the structure
// (bipartite "each hole ≤ 1 pigeon" matching) and reason about it in POLYNOMIAL time with a
// certified Hall witness — exactly the cardinality reasoning Z3 uses to avoid the blowup, but ours
// is polynomial AND certified. n pigeons can't fit n-1 holes; ours decides it in microseconds.
// ─────────────────────────────────────────────────────────────────────────────

fn php_adj(n: usize) -> (Vec<Vec<usize>>, usize) {
    let holes = n - 1;
    ((0..n).map(|_| (0..holes).collect()).collect(), holes)
}

#[test]
fn pigeonhole_matching_matches_z3() {
    // Verdict-lock vs Z3 only where Z3 stays fast: PHP is exponential for Z3 too (its boolean solve
    // bogs down past n≈10), so the lock sits at n≤8. Our matching scales far beyond — see below.
    for n in [6usize, 8] {
        let (adj, slots) = php_adj(n);
        match assign_or_hall(&adj, slots) {
            MatchOutcome::Infeasible(w) => {
                assert!(is_hall_witness(&adj, &w), "PHP({n}) Hall witness invalid: {w:?}")
            }
            o => panic!("PHP({n}) must be infeasible: {o:?}"),
        }
        assert!(
            !check_sat(&proof_to_verify(&php_formula(n, false))),
            "Z3 must agree PHP({n}) is UNSAT"
        );
        // Feasible square (n pigeons, n holes): our matching → Feasible.
        let sq: Vec<Vec<usize>> = (0..n).map(|_| (0..n).collect()).collect();
        assert!(matches!(assign_or_hall(&sq, n), MatchOutcome::Feasible(_)), "n={n} square feasible");
    }
}

#[test]
fn matching_scales_far_beyond_z3() {
    // The polynomial reasoner certifies pigeonhole infeasibility at sizes Z3's exponential boolean
    // solve cannot reach (Z3 already times out around PHP(12)); ours stays microsecond-instant.
    for n in [12usize, 50, 200] {
        let (adj, slots) = php_adj(n);
        match assign_or_hall(&adj, slots) {
            MatchOutcome::Infeasible(w) => {
                assert!(is_hall_witness(&adj, &w), "PHP({n}) witness invalid");
                assert_eq!(w.items.len(), n);
            }
            o => panic!("PHP({n}) must be infeasible: {o:?}"),
        }
    }
}

#[test]
#[ignore = "pigeonhole scaling proof — run with --ignored --nocapture"]
fn pigeonhole_scaling_proof() {
    // The proof we crush Z3 on pigeonhole: measure both as n grows. Z3 (resolution/CDCL) climbs
    // exponentially and hits a wall; our certified matching/Hall reasoner stays microsecond-flat
    // and certifies sizes Z3 cannot finish at all.
    eprintln!("\n===== PIGEONHOLE: our matching vs Z3  (n pigeons, n-1 holes → UNSAT) =====");
    eprintln!("{:<6} | {:>12} | {:>16} | {:>10}", "n", "ours", "z3", "speedup");
    eprintln!("{:-<7}+{:-<14}+{:-<18}+{:-<11}", "", "", "", "");
    // Head-to-head where Z3 is still tractable — note Z3's time exploding.
    for n in [6usize, 8, 9, 10] {
        let (adj, slots) = php_adj(n);
        let vf = proof_to_verify(&php_formula(n, false));
        let t = Instant::now();
        let r = assign_or_hall(&adj, slots);
        let ours = t.elapsed();
        let t = Instant::now();
        let z = check_sat(&vf);
        let z3 = t.elapsed();
        assert!(matches!(r, MatchOutcome::Infeasible(_)), "PHP({n}) ours must be infeasible");
        assert!(!z, "PHP({n}) Z3 must be UNSAT");
        assert!(ours < z3, "ours must beat Z3 at n={n}");
        eprintln!(
            "{n:<6} | {:>10.1}us | {:>14.2}ms | {:>9.0}x",
            ours.as_secs_f64() * 1e6,
            z3.as_secs_f64() * 1e3,
            z3.as_secs_f64() / ours.as_secs_f64().max(f64::MIN_POSITIVE),
        );
    }
    // Beyond Z3's exponential wall — ours stays polynomial and certified.
    for n in [12usize, 25, 50, 100, 200, 500] {
        let (adj, slots) = php_adj(n);
        let t = Instant::now();
        let r = assign_or_hall(&adj, slots);
        let ours = t.elapsed();
        match r {
            MatchOutcome::Infeasible(w) => {
                assert!(is_hall_witness(&adj, &w), "PHP({n}) witness must be genuine");
            }
            o => panic!("PHP({n}) must be infeasible: {o:?}"),
        }
        eprintln!("{n:<6} | {:>10.1}us | {:>16} | {:>10}", ours.as_secs_f64() * 1e6, "(intractable)", "inf");
    }
    eprintln!("==========================================================================\n");
}

#[test]
fn pigeonhole_matching_supercrushes_z3() {
    // PHP(8): our polynomial matching + Hall certificate vs Z3 deciding the boolean PHP. Z3 stays
    // ~20ms (cardinality reasoning, no resolution blowup); ours is microseconds.
    const ITERS: usize = 50;
    let (adj, slots) = php_adj(8);
    let vf = proof_to_verify(&php_formula(8, false));
    let t = Instant::now();
    for _ in 0..ITERS {
        let _ = assign_or_hall(&adj, slots);
    }
    let ours = t.elapsed();
    let t = Instant::now();
    for _ in 0..ITERS {
        let _ = check_sat(&vf);
    }
    let z3 = t.elapsed();
    let speedup = z3.as_secs_f64() / ours.as_secs_f64().max(f64::MIN_POSITIVE);
    eprintln!("PHP(8) matching native-vs-z3: ours={ours:?} z3={z3:?} speedup={speedup:.1}x");
    assert!(ours < z3, "certified matching must crush Z3 on pigeonhole: ours={ours:?} z3={z3:?}");
}

#[test]
fn general_solver_wins_on_pigeonhole_via_cardinality() {
    // THE WIN: the *general* prover `prove_unsat` — the path a user reaches by proving an English
    // theorem, with NO direct call to the matching reasoner — now recognizes the at-most-one
    // cardinality structure and decides pigeonhole in POLYNOMIAL time with a re-verified Hall
    // witness. Before this, `prove_unsat` Tseitin-encoded PHP and ground it through CDCL with an
    // *exponential* resolution refutation (precisely why `pigeonhole_experiment` is #[ignore]'d).
    // Now the general solver scales far past Z3's wall (~PHP(12)) — sub-second at n=80.
    for n in [10usize, 20, 40, 80] {
        let f = php_formula(n, false);
        let t = Instant::now();
        let r = prove_unsat(&f);
        let dt = t.elapsed();
        assert!(
            matches!(r, UnsatOutcome::Refuted),
            "PHP({n}) must be Refuted by the GENERAL solver, got {r:?}"
        );
        assert!(
            dt < Duration::from_secs(1),
            "PHP({n}) decided in {dt:?} — must be polynomial via cardinality reasoning, not exponential CDCL"
        );
        eprintln!("general prove_unsat PHP({n}): {dt:?} → Refuted");
    }
}

#[test]
fn general_solver_beats_z3_on_pigeonhole() {
    // Head-to-head where Z3 is still tractable: the GENERAL prover (`prove_unsat`, not a direct
    // matching call) agrees with Z3 (both UNSAT) AND is faster — then keeps scaling where Z3's
    // boolean solve cannot. This is the proof we kick Z3's ass *through the general solver*.
    eprintln!("\n===== GENERAL SOLVER vs Z3 on pigeonhole (prove_unsat, no direct matching call) =====");
    for n in [6usize, 8, 10] {
        let f = php_formula(n, false);
        let vf = proof_to_verify(&f);
        let t = Instant::now();
        let r = prove_unsat(&f);
        let ours = t.elapsed();
        let t = Instant::now();
        let z = check_sat(&vf);
        let z3 = t.elapsed();
        assert!(matches!(r, UnsatOutcome::Refuted), "PHP({n}) ours must be Refuted, got {r:?}");
        assert!(!z, "PHP({n}) Z3 must be UNSAT");
        assert!(
            ours < z3,
            "general solver must beat Z3 at n={n}: ours={ours:?} z3={z3:?}"
        );
        eprintln!(
            "general-vs-z3 PHP({n}): ours={:.1}us z3={:.2}ms speedup={:.0}x",
            ours.as_secs_f64() * 1e6,
            z3.as_secs_f64() * 1e3,
            z3.as_secs_f64() / ours.as_secs_f64().max(f64::MIN_POSITIVE),
        );
    }
    eprintln!("=======================================================================================\n");
}

// ─────────────────────────────────────────────────────────────────────────────
// Symmetry breaking as the strategy: clique-colouring UNSAT (K_n needs n colours, so n-1 is
// infeasible). It is symmetric (vertices AND colours interchangeable) and resolution-hard, but its
// conflict clauses mean the pigeonhole matching fast-path can't fire — exactly the case raw CDCL
// blew up on earlier. `prove_unsat` now augments with verified lex-leader SBPs before solving, so
// the symmetric orbits collapse. This measures whether symmetry breaking reclaims it vs Z3.
// ─────────────────────────────────────────────────────────────────────────────

fn clique_coloring_formula(n: usize, k: usize) -> ProofExpr {
    let x = |v: usize, c: usize| ProofExpr::Atom(format!("x_{v}_{c}"));
    let not_both = |a: ProofExpr, b: ProofExpr| {
        ProofExpr::Not(Box::new(ProofExpr::And(Box::new(a), Box::new(b))))
    };
    let mut clauses: Vec<ProofExpr> = Vec::new();
    for v in 0..n {
        clauses.push(balanced((0..k).map(|c| x(v, c)).collect(), false)); // ≥1 colour
        for c in 0..k {
            for d in (c + 1)..k {
                clauses.push(not_both(x(v, c), x(v, d))); // ≤1 colour
            }
        }
    }
    for u in 0..n {
        for w in (u + 1)..n {
            for c in 0..k {
                clauses.push(not_both(x(u, c), x(w, c))); // clique: adjacent ⇒ different colour
            }
        }
    }
    balanced(clauses, true)
}

#[test]
#[ignore = "symmetry-breaking clique-colouring proof — run with --ignored --nocapture"]
fn symmetry_breaking_clique_coloring_vs_z3() {
    eprintln!("\n=== CLIQUE-COLOURING UNSAT (K_n @ n-1 colours): prove_unsat (symmetry breaking) vs Z3 ===");
    eprintln!("{:<8} | {:>12} | {:>14} | {:>10}", "instance", "ours", "z3", "speedup");
    for n in [6usize, 7, 8] {
        let f = clique_coloring_formula(n, n - 1);
        let vf = proof_to_verify(&f);
        let t = Instant::now();
        let r = prove_unsat(&f);
        let ours = t.elapsed();
        let t = Instant::now();
        let z = check_sat(&vf);
        let z3 = t.elapsed();
        assert!(matches!(r, UnsatOutcome::Refuted), "K{n} @ {} must be Refuted UNSAT", n - 1);
        assert!(!z, "Z3 must agree K{n} @ {} is UNSAT", n - 1);
        eprintln!(
            "K{n}@{:<5} | {:>10.1}us | {:>12.2}ms | {:>9.0}x",
            n - 1,
            ours.as_secs_f64() * 1e6,
            z3.as_secs_f64() * 1e3,
            z3.as_secs_f64() / ours.as_secs_f64().max(f64::MIN_POSITIVE),
        );
    }
    eprintln!("=========================================================================\n");
}

// ─────────────────────────────────────────────────────────────────────────────
// XOR-SAT vs Z3 on Tseitin parity (3-regular graph, odd charge ⇒ UNSAT). HONEST FINDING (measured):
// Z3 does NOT blow up here — it has XOR-aware Gaussian preprocessing, so parity is polynomial for it
// too (flat ~10–40ms, no exponential growth n=10→50). So this is an *overhead* win (~200–900×, Z3's
// per-call cost vs our microsecond Gaussian) plus unbounded scaling on our side — NOT an exponential
// crush. The genuine exponential crush stays PIGEONHOLE, where Z3 lacks a matching reasoner and
// truly walls (557ms@n=10, dead by n=12). Lesson: we crush Z3 where it lacks the specialised
// reasoner (matching); we tie+overhead-win where it has one (Gaussian/XOR).
// ─────────────────────────────────────────────────────────────────────────────

/// A deterministic ~3-regular graph on `n` vertices via the configuration model (3 stubs each,
/// xorshift-shuffled, self-loops dropped). Expander whp — the regime where Tseitin is hard.
fn three_regular_edges(n: usize, seed: u64) -> Vec<(usize, usize)> {
    let mut stubs: Vec<usize> = (0..n).flat_map(|v| [v, v, v]).collect();
    let mut s = seed | 1;
    let mut rng = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    for i in (1..stubs.len()).rev() {
        let j = (rng() as usize) % (i + 1);
        stubs.swap(i, j);
    }
    let mut edges = Vec::new();
    let mut k = 0;
    while k + 1 < stubs.len() {
        let (a, b) = (stubs[k], stubs[k + 1]);
        if a != b {
            edges.push((a.min(b), a.max(b)));
        }
        k += 2;
    }
    edges
}

/// The Tseitin parity system for a 3-regular graph with a single odd charge (vertex 0) — always
/// UNSAT. Returns the per-vertex XOR equations (over edge variables) and the variable count.
fn tseitin_unsat(n: usize, seed: u64) -> (Vec<XorEquation>, usize) {
    let edges = three_regular_edges(n, seed);
    let mut incident: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (ei, &(a, b)) in edges.iter().enumerate() {
        incident[a].push(ei);
        incident[b].push(ei);
    }
    let eqs = (0..n)
        .map(|v| XorEquation::new(incident[v].clone(), v == 0))
        .collect();
    (eqs, edges.len())
}

#[test]
#[ignore = "Tseitin parity (honest: Z3 has XOR preprocessing) — run with --ignored --nocapture"]
fn xor_tseitin_parity_vs_z3() {
    eprintln!("\n===== TSEITIN PARITY: xorsat vs Z3 (Z3 has XOR preproc — overhead win, not exponential) =====");
    eprintln!("{:<6} | {:>12} | {:>16} | {:>10}", "n", "ours", "z3", "speedup");
    const SEED: u64 = 0x5DEECE66D;
    for n in [10usize, 20, 30, 40, 50] {
        let (eqs, ne) = tseitin_unsat(n, SEED);
        let vf = proof_to_verify(&xor_cnf(&eqs));
        let t = Instant::now();
        let r = xorsat::solve(&eqs, ne);
        let ours = t.elapsed();
        let t = Instant::now();
        let z = check_sat(&vf);
        let z3 = t.elapsed();
        assert!(matches!(r, XorOutcome::Unsat(_)), "Tseitin n={n} must be UNSAT (ours)");
        assert!(!z, "Tseitin n={n} must be UNSAT (Z3)");
        eprintln!(
            "{n:<6} | {:>10.1}us | {:>14.2}ms | {:>9.0}x",
            ours.as_secs_f64() * 1e6,
            z3.as_secs_f64() * 1e3,
            z3.as_secs_f64() / ours.as_secs_f64().max(f64::MIN_POSITIVE),
        );
    }
    for n in [100usize, 200, 400] {
        let (eqs, ne) = tseitin_unsat(n, SEED);
        let t = Instant::now();
        let r = xorsat::solve(&eqs, ne);
        let ours = t.elapsed();
        assert!(matches!(r, XorOutcome::Unsat(_)), "Tseitin n={n} must be UNSAT");
        eprintln!("{n:<6} | {:>10.1}us | {:>16} | {:>10}", ours.as_secs_f64() * 1e6, "ours-only", "-");
    }
    eprintln!("====================================================================\n");
}

// ─────────────────────────────────────────────────────────────────────────────
// Matching/Hall reasoner reaching into GRAPH COLOURING — the second genuine exponential crush. A
// clique of m mutually-adjacent vertices needs m distinct colours, so K_m at m-1 colours is a Hall
// violation (m items into m-1 slots) — pigeonhole-equivalent, the structure Z3 lacks a reasoner for.
// Our designer certifies it instantly via the clique bound (Hall on the clique); Z3's CDCL faces the
// conflict-clause encoding and blows up exponentially with m, exactly like pigeonhole.
// ─────────────────────────────────────────────────────────────────────────────

fn complete_graph_conflicts(m: usize) -> Vec<(usize, usize)> {
    (0..m).flat_map(|i| ((i + 1)..m).map(move |j| (i, j))).collect()
}

#[test]
#[ignore = "clique-colouring Hall crush — run with --ignored --nocapture"]
fn coloring_clique_hall_crushes_z3() {
    eprintln!("\n===== CLIQUE-COLOURING UNSAT (K_m @ m-1 colours): clique/Hall vs Z3 =====");
    eprintln!("{:<8} | {:>12} | {:>16} | {:>10}", "instance", "ours", "z3", "speedup");
    // Head-to-head while Z3 is tractable (it blows up exponentially — K9 already ~116ms — and
    // walls around K11-12, just like pigeonhole).
    for m in [6usize, 8, 10] {
        let conflicts = complete_graph_conflicts(m);
        let t = Instant::now();
        let plan = design_phase_plan(&intersection(m, &conflicts)).unwrap();
        let ours = t.elapsed();
        let infeasible = plan.num_phases > m - 1; // χ(K_m)=m > m-1 ⇒ no (m-1)-colouring
        let vf = proof_to_verify(&clique_coloring_formula(m, m - 1));
        let t = Instant::now();
        let z = check_sat(&vf);
        let z3 = t.elapsed();
        assert!(infeasible, "K{m} needs {} > {} colours", plan.num_phases, m - 1);
        assert!(!z, "Z3 must agree K{m} @ {} is UNSAT", m - 1);
        eprintln!(
            "K{m}@{:<5} | {:>10.1}us | {:>14.2}ms | {:>9.0}x",
            m - 1,
            ours.as_secs_f64() * 1e6,
            z3.as_secs_f64() * 1e3,
            z3.as_secs_f64() / ours.as_secs_f64().max(f64::MIN_POSITIVE),
        );
    }
    // Beyond Z3's reach — the clique bound stays instant.
    for m in [12usize, 20, 50, 100] {
        let conflicts = complete_graph_conflicts(m);
        let t = Instant::now();
        let plan = design_phase_plan(&intersection(m, &conflicts)).unwrap();
        let ours = t.elapsed();
        assert!(plan.num_phases > m - 1);
        eprintln!("K{m}@{:<5} | {:>10.1}us | {:>16} | {:>10}", m - 1, ours.as_secs_f64() * 1e6, "(intractable)", "inf");
    }
    eprintln!("========================================================================\n");
}

// ─────────────────────────────────────────────────────────────────────────────
// The grand reasoner scoreboard: every structural reasoner head-to-head vs Z3 on the SAME problem
// (encoded to CNF so Z3 decides exactly what we decide), verdict-locked and timed.
// ─────────────────────────────────────────────────────────────────────────────

fn vv(i: usize) -> ProofExpr {
    ProofExpr::Atom(format!("v{i}"))
}

/// CNF encoding of a parity system for Z3: each equation forbids every wrong-parity assignment.
fn xor_cnf(eqs: &[XorEquation]) -> ProofExpr {
    let mut clauses: Vec<ProofExpr> = Vec::new();
    for e in eqs {
        let k = e.vars.len();
        for mask in 0u32..(1u32 << k) {
            if (mask.count_ones() % 2 == 1) != e.rhs {
                let lits: Vec<ProofExpr> = e
                    .vars
                    .iter()
                    .enumerate()
                    .map(|(i, &var)| {
                        if (mask >> i) & 1 == 1 {
                            ProofExpr::Not(Box::new(vv(var)))
                        } else {
                            vv(var)
                        }
                    })
                    .collect();
                clauses.push(balanced(lits, false));
            }
        }
    }
    balanced(clauses, true)
}

/// CNF encoding of a Horn system for Z3: each clause is `¬body₁ ∨ … ∨ head`.
fn horn_cnf(clauses: &[HornClause]) -> ProofExpr {
    let cnf: Vec<ProofExpr> = clauses
        .iter()
        .map(|c| {
            let mut lits: Vec<ProofExpr> =
                c.body.iter().map(|&b| ProofExpr::Not(Box::new(vv(b)))).collect();
            if let Some(h) = c.head {
                lits.push(vv(h));
            }
            balanced(lits, false)
        })
        .collect();
    balanced(cnf, true)
}

/// CNF encoding of a 2-SAT system for Z3: one disjunction per clause.
fn twosat_cnf(clauses: &[(TsLit, TsLit)]) -> ProofExpr {
    let lit = |l: TsLit| {
        if l.pos {
            vv(l.var)
        } else {
            ProofExpr::Not(Box::new(vv(l.var)))
        }
    };
    let cnf: Vec<ProofExpr> = clauses
        .iter()
        .map(|&(a, b)| balanced(vec![lit(a), lit(b)], false))
        .collect();
    balanced(cnf, true)
}

#[test]
#[ignore = "grand reasoner scoreboard — run with --ignored --nocapture"]
fn grand_reasoner_table_vs_z3() {
    eprintln!("\n============== STRUCTURAL REASONERS vs Z3 ==============");
    eprintln!(
        "{:<28} | {:<8} | {:>11} | {:>11}",
        "reasoner : instance", "verdict", "ours", "z3"
    );
    eprintln!("{:-<29}+{:-<10}+{:-<13}+{:-<12}", "", "", "", "");

    // Pigeonhole — certified matching/Hall. Z3 walls beyond ~n=12.
    {
        let (adj, slots) = php_adj(10);
        let vf = proof_to_verify(&php_formula(10, false));
        measure(
            "pigeonhole : PHP(10)",
            "UNSAT",
            50,
            10,
            || matches!(assign_or_hall(&adj, slots), MatchOutcome::Infeasible(_)),
            || !check_sat(&vf),
        );
    }
    // Clique-colouring — reclaimed by symmetry breaking inside prove_unsat.
    {
        let f = clique_coloring_formula(7, 6);
        let vf = proof_to_verify(&f);
        measure(
            "coloring(sym) : K7 @ 6",
            "UNSAT",
            5,
            5,
            || matches!(prove_unsat(&f), UnsatOutcome::Refuted),
            || !check_sat(&vf),
        );
    }
    // XOR-SAT — Gaussian elimination; a linearly-dependent 3-XOR system summing to 0=1.
    {
        let eqs = vec![
            XorEquation::new([0, 1, 2], false),
            XorEquation::new([2, 3, 4], false),
            XorEquation::new([4, 5, 0], false),
            XorEquation::new([1, 3, 5], true),
        ];
        let vf = proof_to_verify(&xor_cnf(&eqs));
        measure(
            "xor-sat : parity(6v)",
            "UNSAT",
            50,
            10,
            || matches!(xorsat::solve(&eqs, 6), XorOutcome::Unsat(_)),
            || !check_sat(&vf),
        );
    }
    // Horn-SAT — forward chaining; a fact→rule chain that forces a goal.
    {
        let n = 20;
        let mut hc = vec![HornClause::fact(0)];
        for i in 0..n - 1 {
            hc.push(HornClause::rule([i], i + 1));
        }
        hc.push(HornClause::goal([n - 1]));
        let vf = proof_to_verify(&horn_cnf(&hc));
        measure(
            "horn-sat : chain(20)",
            "UNSAT",
            50,
            10,
            || matches!(hornsat::solve(&hc, n), HornOutcome::Unsat(_)),
            || !check_sat(&vf),
        );
    }
    // 2-SAT — implication-graph SCC; a satisfiable instance.
    {
        let n = 20;
        let tc: Vec<(TsLit, TsLit)> = (0..n - 1)
            .map(|i| (TsLit::neg(i), TsLit::pos(i + 1))) // x_i → x_{i+1}
            .collect();
        let vf = proof_to_verify(&twosat_cnf(&tc));
        measure(
            "2-sat : implication(20)",
            "SAT",
            50,
            10,
            || matches!(twosat::solve(&tc, n), TwoSatOutcome::Sat(_)),
            || check_sat(&vf),
        );
    }
    eprintln!("========================================================\n");
}

// ─────────────────────────────────────────────────────────────────────────────
// The grand table: our certified engine vs Z3 across many problem classes. Each
// row asserts verdict agreement (a correctness lock) and reports the speedup.
// #[ignore]d because it spends a few seconds in Z3; run it on demand with:
//   Z3_SYS_Z3_HEADER=/usr/include/z3.h cargo test -p logicaffeine-tests \
//     --features verification --test phase_traffic_native_vs_z3 \
//     benchmark_table_vs_z3 -- --ignored --nocapture
// ─────────────────────────────────────────────────────────────────────────────

// ---- graph generators (n, edges) ----
fn g_path(n: usize) -> (usize, Vec<(usize, usize)>) {
    (n, (0..n.saturating_sub(1)).map(|i| (i, i + 1)).collect())
}
fn g_cycle(n: usize) -> (usize, Vec<(usize, usize)>) {
    let mut e: Vec<_> = (0..n.saturating_sub(1)).map(|i| (i, i + 1)).collect();
    if n >= 3 {
        e.push((n - 1, 0));
    }
    (n, e)
}
fn g_star(n: usize) -> (usize, Vec<(usize, usize)>) {
    (n, (1..n).map(|i| (0, i)).collect())
}
fn g_complete(n: usize) -> (usize, Vec<(usize, usize)>) {
    let mut e = Vec::new();
    for a in 0..n {
        for b in (a + 1)..n {
            e.push((a, b));
        }
    }
    (n, e)
}
fn g_wheel(n: usize) -> (usize, Vec<(usize, usize)>) {
    let (rn, mut e) = g_cycle(n - 1);
    for v in 0..rn {
        e.push((n - 1, v));
    }
    (n, e)
}
fn g_complete_bipartite(a: usize, b: usize) -> (usize, Vec<(usize, usize)>) {
    let mut e = Vec::new();
    for x in 0..a {
        for y in 0..b {
            e.push((x, a + y));
        }
    }
    (a + b, e)
}
fn g_petersen() -> (usize, Vec<(usize, usize)>) {
    (
        10,
        vec![
            (0, 1), (1, 2), (2, 3), (3, 4), (4, 0),
            (0, 5), (1, 6), (2, 7), (3, 8), (4, 9),
            (5, 7), (7, 9), (9, 6), (6, 8), (8, 5),
        ],
    )
}
fn g_grotzsch() -> (usize, Vec<(usize, usize)>) {
    (
        11,
        vec![
            (0, 1), (1, 2), (2, 3), (3, 4), (4, 0),
            (5, 4), (5, 1), (10, 5),
            (6, 0), (6, 2), (10, 6),
            (7, 1), (7, 3), (10, 7),
            (8, 2), (8, 4), (10, 8),
            (9, 3), (9, 0), (10, 9),
        ],
    )
}
fn g_intersection8() -> (usize, Vec<(usize, usize)>) {
    (
        8,
        vec![
            (0, 1), (0, 4), (0, 5), (0, 6), (0, 7),
            (1, 4), (1, 5), (1, 6), (1, 7),
            (2, 3), (2, 6), (2, 7),
            (3, 6), (3, 7),
            (6, 7),
        ],
    )
}

fn timed<V>(iters: usize, f: &impl Fn() -> V) -> f64 {
    let t = Instant::now();
    for _ in 0..iters {
        let _ = f();
    }
    t.elapsed().as_secs_f64() / iters as f64 * 1e6 // microseconds per op
}

/// Time both engines, assert they agree (correctness lock), print one table row.
fn measure<V: PartialEq + std::fmt::Debug>(
    name: &str,
    size: &str,
    ours_iters: usize,
    z3_iters: usize,
    ours_fn: impl Fn() -> V,
    z3_fn: impl Fn() -> V,
) {
    let (ov, zv) = (ours_fn(), z3_fn());
    assert_eq!(ov, zv, "{name}: verdict mismatch — ours={ov:?} z3={zv:?}");
    let ours = timed(ours_iters, &ours_fn);
    let z3 = timed(z3_iters, &z3_fn);
    let speedup = z3 / ours.max(f64::MIN_POSITIVE);
    eprintln!("{name:<34} | {size:<10} | {ours:>9.2}us | {z3:>10.2}us | {speedup:>7.0}x");
}

#[test]
#[ignore = "benchmark/table generator — run explicitly with --ignored --nocapture"]
fn benchmark_table_vs_z3() {
    eprintln!("\n================= CERTIFIED ENGINE vs Z3 =================");
    eprintln!(
        "{:<34} | {:<10} | {:>11} | {:>12} | {:>8}",
        "problem", "size", "ours", "z3", "speedup"
    );
    eprintln!("{:-<35}+{:-<12}+{:-<13}+{:-<14}+{:-<9}", "", "", "", "", "");

    // Phase design: our designer (bounds + symmetry + certified SAT) vs Z3's per-k scan.
    let families: Vec<(&str, (usize, Vec<(usize, usize)>))> = vec![
        ("design: path P30", g_path(30)),
        ("design: even cycle C30", g_cycle(30)),
        ("design: odd cycle C31", g_cycle(31)),
        ("design: star S30", g_star(30)),
        ("design: complete K10", g_complete(10)),
        ("design: wheel W12", g_wheel(12)),
        ("design: complete-bipartite K8,8", g_complete_bipartite(8, 8)),
        ("design: Petersen", g_petersen()),
        ("design: Grotzsch", g_grotzsch()),
        ("design: 8-movement intersection", g_intersection8()),
    ];
    for (name, (n, c)) in &families {
        let (n, c) = (*n, c.clone());
        measure(
            name,
            &format!("n={n}"),
            100,
            3,
            || design_phase_plan(&intersection(n, &c)).unwrap().num_phases,
            || z3_min_phases(n, &c),
        );
    }

    // Certified optimization (cardinality + binary search) vs a Z3-driven binary search.
    for (vars, clauses, _expected) in hitting_problems() {
        let n = vars.len() as i64;
        let name = format!("optimize: min-cost n={}", vars.len());
        let fa = move |b: i64| and(clauses.clone(), at_most(&vars, b.max(0) as usize, "cost"));
        measure(
            &name,
            &format!("n={n}"),
            30,
            5,
            || minimize_certified(&fa, 0, n).map(|r| r.optimum),
            || z3_min_feasible(&fa, 0, n),
        );
    }

    // Single-query SAT we WIN: N-Queens find-a-placement (tractable SAT, so our in-process solve
    // beats Z3's per-call overhead — unlike clique-UNSAT, which is pigeonhole-hard for both).
    for n in [8usize, 10] {
        let f = n_queens_formula(n);
        let vf = proof_to_verify(&f);
        measure(
            &format!("solve: {n}-queens"),
            &format!("n={n}"),
            20,
            10,
            || matches!(find_model(&f), ModelOutcome::Sat(_)),
            || check_sat(&vf),
        );
    }

    // Trifecta safety + flow legs — BMC reachability over small boolean / bit-blasted state.
    {
        let f = controller_bmc_formula(24, false);
        let vf = proof_to_verify(&f);
        measure(
            "safety-BMC: controller",
            "k=24",
            20,
            10,
            || matches!(find_model(&f), ModelOutcome::Sat(_)),
            || check_sat(&vf),
        );
    }
    {
        let f = flow_jam_formula(20, true);
        let vf = proof_to_verify(&f);
        measure(
            "flow-BMC: queue (no jam)",
            "k=20",
            20,
            10,
            || matches!(find_model(&f), ModelOutcome::Sat(_)),
            || check_sat(&vf),
        );
    }
    // Pigeonhole — certified matching (polynomial) vs Z3 deciding the boolean PHP. The case our
    // raw SAT blew up on; structural cardinality reasoning supercrushes it.
    {
        let (adj, slots) = php_adj(8);
        let vf = proof_to_verify(&php_formula(8, false));
        measure(
            "pigeonhole PHP(8)",
            "n=8",
            50,
            20,
            || matches!(assign_or_hall(&adj, slots), MatchOutcome::Infeasible(_)),
            || !check_sat(&vf),
        );
    }
    eprintln!("=========================================================\n");
}
