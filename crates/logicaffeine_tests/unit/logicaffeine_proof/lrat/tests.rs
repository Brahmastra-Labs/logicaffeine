use super::*;
use crate::cdcl::{SolveResult, Solver};
use crate::proof_emit::check_lrat;
use crate::rup::check_refutation;

fn p(v: u32) -> Lit {
    Lit::pos(v)
}
fn n(v: u32) -> Lit {
    Lit::neg(v)
}

/// The learned clauses of a CDCL refutation of `cnf`, or `None` when it is satisfiable.
fn solve(num_vars: usize, cnf: &[Vec<Lit>]) -> Option<Vec<Vec<Lit>>> {
    let mut s = Solver::new(num_vars);
    for c in cnf {
        s.add_clause(c.clone());
    }
    match s.solve() {
        SolveResult::Sat(_) => None,
        SolveResult::Unsat => Some(s.learned().iter().map(|c| c.lits.clone()).collect()),
    }
}

/// Pigeons `0..=holes` into `holes` holes: variable `i * holes + h` says pigeon `i` sits in hole `h`.
fn pigeonhole(holes: u32) -> (usize, Vec<Vec<Lit>>) {
    let pigeons = holes + 1;
    let var = |i: u32, h: u32| i * holes + h;
    let mut cnf = Vec::new();
    for i in 0..pigeons {
        cnf.push((0..holes).map(|h| p(var(i, h))).collect());
    }
    for h in 0..holes {
        for i in 0..pigeons {
            for j in i + 1..pigeons {
                cnf.push(vec![n(var(i, h)), n(var(j, h))]);
            }
        }
    }
    ((pigeons * holes) as usize, cnf)
}

/// A fixed-seed random 3-CNF over `vars` variables.
fn random_3cnf(seed: u64, vars: u32, clauses: usize) -> Vec<Vec<Lit>> {
    let mut s = seed | 1;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    (0..clauses)
        .map(|_| {
            (0..3)
                .map(|_| {
                    let r = next();
                    Lit::new((r % vars as u64) as u32, (r >> 32) & 1 == 0)
                })
                .collect()
        })
        .collect()
}

#[test]
fn a_cdcl_refutation_becomes_a_checked_lrat_proof() {
    let (nv, cnf) = pigeonhole(4);
    let learned = solve(nv, &cnf).expect("five pigeons do not fit four holes");
    let proof = refutation(nv, &cnf, &learned).expect("every learned clause is RUP in order");
    assert!(check(nv, &cnf, &proof));
    assert!(proof.steps.last().unwrap().clause.is_empty(), "the proof ends in the empty clause");
}

#[test]
fn the_text_form_is_accepted_by_the_reference_lrat_checker() {
    let (nv, cnf) = pigeonhole(3);
    let learned = solve(nv, &cnf).unwrap();
    let proof = refutation(nv, &cnf, &learned).unwrap();
    assert!(check_lrat(nv, &cnf, &proof.to_text()));
}

#[test]
fn hint_ids_number_the_formula_from_one_and_the_learned_clauses_after_it() {
    let (nv, cnf) = pigeonhole(3);
    let learned = solve(nv, &cnf).unwrap();
    let proof = refutation(nv, &cnf, &learned).unwrap();
    let n = cnf.len() as u64;
    let mut last = n;
    for step in &proof.steps {
        assert!(step.id > last, "ids increase past the formula's");
        let learned_index = (step.id - n - 1) as usize;
        if !step.clause.is_empty() {
            assert_eq!(step.clause, learned[learned_index], "a step's id names its learned clause");
        } else {
            assert_eq!(step.id, n + learned.len() as u64 + 1, "the empty clause takes the id after them all");
        }
        for &h in &step.hints {
            assert!(h >= 1 && h < step.id, "a hint names an earlier clause");
        }
        last = step.id;
    }
}

#[test]
fn a_learned_clause_no_later_step_uses_is_trimmed_away() {
    // x0 and ¬x0 refute alone; the learned (x1 ∨ x2) is RUP (from x1 ∨ x2 itself) but unused.
    let cnf = vec![vec![p(0)], vec![n(0)], vec![p(1), p(2)]];
    let learned = vec![vec![p(1), p(2)]];
    let proof = refutation(3, &cnf, &learned).unwrap();
    assert_eq!(proof.steps.len(), 1, "only the empty clause remains");
    assert_eq!(proof.steps[0].hints, vec![1, 2]);
    assert!(check(3, &cnf, &proof));
}

#[test]
fn an_empty_clause_in_the_formula_is_its_own_refutation() {
    let cnf = vec![vec![p(0), p(1)], vec![]];
    let proof = refutation(2, &cnf, &[]).unwrap();
    assert_eq!(proof.steps, vec![LratStep { id: 3, clause: vec![], hints: vec![2] }]);
    assert!(check(2, &cnf, &proof));
}

#[test]
fn a_unit_clause_falsified_by_the_negated_clause_closes_the_chain() {
    // (x0) and (¬x0 ∨ x1): the learned unit x1 is RUP; with (¬x1) the empty clause follows.
    let cnf = vec![vec![p(0)], vec![n(0), p(1)], vec![n(1)]];
    let learned = vec![vec![p(1)]];
    let proof = refutation(2, &cnf, &learned).unwrap();
    assert!(check(2, &cnf, &proof));
    assert!(check_lrat(2, &cnf, &proof.to_text()));
}

#[test]
fn duplicate_and_tautological_clauses_are_handled_as_the_rup_checker_reads_them() {
    let cnf = vec![vec![p(0), p(0)], vec![n(0), p(1), n(0)], vec![n(1)], vec![p(2), n(2)]];
    let learned = vec![vec![p(3), n(3)], vec![p(1), p(1)]];
    let proof = refutation(4, &cnf, &learned).unwrap();
    assert!(check(4, &cnf, &proof));
    assert!(check_lrat(4, &cnf, &proof.to_text()));
}

#[test]
fn a_satisfiable_formula_has_no_refutation() {
    let cnf = vec![vec![p(0), p(1)], vec![n(0)]];
    assert_eq!(refutation(2, &cnf, &[]), None);
}

#[test]
fn a_learned_clause_that_is_not_rup_refuses_the_whole_trace() {
    // (x0 ∨ x1), (¬x0 ∨ x1), (x0 ∨ ¬x1), (¬x0 ∨ ¬x1) is unsatisfiable, but x2 is not RUP from it.
    let cnf = vec![vec![p(0), p(1)], vec![n(0), p(1)], vec![p(0), n(1)], vec![n(0), n(1)]];
    let learned = vec![vec![p(2)], vec![p(1)]];
    // The trimmed proof never needs x2 — the empty clause comes from x1 and the formula.
    let trimmed = refutation(3, &cnf, &learned).unwrap();
    assert!(trimmed.steps.iter().all(|s| s.clause != vec![p(2)]));
    assert!(check(3, &cnf, &trimmed));
    // A refutation whose only route runs through a non-RUP step is refused.
    let cnf2 = vec![vec![n(2)], vec![p(0), p(1)]];
    assert_eq!(refutation(3, &cnf2, &[vec![p(2)]]), None);
}

#[test]
fn the_checker_refuses_a_corrupted_hint() {
    let (nv, cnf) = pigeonhole(3);
    let learned = solve(nv, &cnf).unwrap();
    let proof = refutation(nv, &cnf, &learned).unwrap();
    let k = proof.steps.iter().position(|s| s.hints.len() >= 2).unwrap();
    let mut bad = proof.clone();
    bad.steps[k].hints[0] = bad.steps[k].hints[0] % cnf.len() as u64 + 1;
    if bad.steps[k].hints[0] == proof.steps[k].hints[0] {
        bad.steps[k].hints[0] += 1;
    }
    assert!(!check(nv, &cnf, &bad));
}

#[test]
fn the_checker_refuses_reordered_hints() {
    let (nv, cnf) = pigeonhole(3);
    let learned = solve(nv, &cnf).unwrap();
    let proof = refutation(nv, &cnf, &learned).unwrap();
    let k = proof.steps.iter().position(|s| s.hints.len() >= 3).unwrap();
    let mut bad = proof.clone();
    bad.steps[k].hints.reverse();
    assert!(!check(nv, &cnf, &bad));
}

#[test]
fn the_checker_refuses_a_proof_without_the_empty_clause() {
    let (nv, cnf) = pigeonhole(3);
    let learned = solve(nv, &cnf).unwrap();
    let mut proof = refutation(nv, &cnf, &learned).unwrap();
    proof.steps.pop();
    assert!(!check(nv, &cnf, &proof));
}

#[test]
fn the_checker_refuses_an_id_that_does_not_increase() {
    let cnf = vec![vec![p(0)], vec![n(0)]];
    let proof = Lrat { steps: vec![LratStep { id: 2, clause: vec![], hints: vec![1, 2] }] };
    assert!(!check(1, &cnf, &proof), "an addition cannot reuse a formula id");
}

#[test]
fn the_checker_refuses_a_hint_to_a_clause_not_yet_added() {
    let cnf = vec![vec![p(0)], vec![n(0), p(1)], vec![n(1)]];
    let proof = Lrat {
        steps: vec![
            LratStep { id: 4, clause: vec![], hints: vec![1, 5] },
            LratStep { id: 5, clause: vec![p(1)], hints: vec![1, 2] },
        ],
    };
    assert!(!check(2, &cnf, &proof));
}

#[test]
fn the_checker_refuses_a_literal_past_the_variable_count() {
    let cnf = vec![vec![p(0)], vec![n(0)]];
    let proof = Lrat {
        steps: vec![
            LratStep { id: 3, clause: vec![p(7)], hints: vec![1, 2] },
            LratStep { id: 4, clause: vec![], hints: vec![1, 2] },
        ],
    };
    assert!(!check(1, &cnf, &proof));
    let wide = vec![vec![p(9)], vec![n(0)]];
    let proof = Lrat { steps: vec![LratStep { id: 3, clause: vec![], hints: vec![1, 2] }] };
    assert!(!check(1, &wide, &proof), "a formula literal past the count is refused too");
}

#[test]
fn the_checker_refuses_a_satisfied_hint() {
    // Under ¬(empty) nothing is assumed; the unit (x0) propagates, so (x0 ∨ x1) is satisfied, not unit.
    let cnf = vec![vec![p(0)], vec![p(0), p(1)], vec![n(0)]];
    let proof = Lrat { steps: vec![LratStep { id: 4, clause: vec![], hints: vec![1, 2, 3] }] };
    assert!(!check(2, &cnf, &proof));
}

#[test]
fn random_unsatisfiable_formulas_agree_with_the_rup_checker() {
    let mut refuted = 0;
    for seed in 1..=200u64 {
        let vars = 12 + (seed % 9) as u32;
        let cnf = random_3cnf(seed, vars, (vars as usize) * 5);
        let Some(learned) = solve(vars as usize, &cnf) else { continue };
        refuted += 1;
        let naive = check_refutation(vars as usize, &cnf, &learned);
        let proof = refutation(vars as usize, &cnf, &learned);
        assert_eq!(proof.is_some(), naive, "seed {seed}: the watched and the naive replay disagree");
        let proof = proof.unwrap();
        assert!(check(vars as usize, &cnf, &proof), "seed {seed}");
        assert!(check_lrat(vars as usize, &cnf, &proof.to_text()), "seed {seed}");
    }
    assert!(refuted >= 100, "the corpus holds enough refutations to mean something: {refuted}");
}

/// (a ∨ b), (¬a ∨ c), (¬b ∨ c): c follows, and so does (a ∨ c) on the way.
fn diamond() -> Vec<Vec<Lit>> {
    vec![vec![p(0), p(1)], vec![n(0), p(2)], vec![n(1), p(2)]]
}

#[test]
fn a_derivation_carries_each_target_and_only_what_it_uses() {
    // The diamond beside (d ∨ e), (¬d ∨ e): (e) is RUP but shares no variable with the target (c).
    let mut f = diamond();
    f.extend([vec![p(3), p(4)], vec![n(3), p(4)]]);
    let log = vec![vec![p(4)], vec![p(2)]];
    let proof = derivation(5, &f, &log, &[1]).expect("c is RUP from the formula");
    assert_eq!(proof.steps.len(), 1, "the unused clause is trimmed: {proof:?}");
    assert_eq!(proof.steps[0].clause, vec![p(2)]);
    assert_eq!(proof.steps[0].id, 7, "a log clause keeps its place in the numbering: formula 1..=5, log 6..");
    assert!(check_derivation(5, &f, &proof, &[vec![p(2)]]));
    assert!(proof.steps.iter().all(|s| !s.clause.is_empty()), "a derivation ends in its targets, not the empty clause");
}

#[test]
fn a_target_that_needs_an_earlier_log_clause_keeps_it() {
    // x ⇒ y ⇒ z: the log derives (¬x ∨ y) then (¬x ∨ z), the second through the first only.
    let f = vec![vec![n(0), p(3)], vec![n(3), p(1)], vec![n(1), p(2)]];
    let log = vec![vec![n(0), p(1)], vec![n(0), p(2)]];
    let proof = derivation(4, &f, &log, &[1]).unwrap();
    assert!(check_derivation(4, &f, &proof, &[vec![n(0), p(2)]]));
    let both = derivation(4, &f, &log, &[0, 1]).unwrap();
    assert!(check_derivation(4, &f, &both, &[vec![p(1), n(0)], vec![n(0), p(2)]]), "a claim is its literals in any order");
}

#[test]
fn a_target_that_is_not_rup_has_no_derivation() {
    assert!(derivation(3, &diamond(), &[vec![n(2)]], &[0]).is_none(), "¬c does not follow");
    assert!(derivation(3, &diamond(), &[vec![p(0)]], &[0]).is_none(), "nor does a alone");
}

#[test]
fn a_derivation_check_refuses_a_claim_it_never_derives_and_a_tampered_step() {
    let f = diamond();
    let proof = derivation(3, &f, &[vec![p(2)]], &[0]).unwrap();
    assert!(!check_derivation(3, &f, &proof, &[vec![p(0)]]), "a claim no step derives");
    assert!(!check_derivation(3, &f, &proof, &[vec![p(2)], vec![n(2)]]), "every claim, not some");
    let mut bad = proof.clone();
    bad.steps[0].hints.reverse();
    assert!(!check_derivation(3, &f, &bad, &[vec![p(2)]]), "hints out of order do not replay");
    let mut stronger = proof.clone();
    stronger.steps[0].clause = vec![];
    assert!(!check_derivation(3, &f, &stronger, &[vec![p(2)]]), "a step claiming more than its hints show");
    assert!(check_derivation(3, &f, &proof, &[vec![p(0), p(1)]]), "a formula clause is claimed with no step");
    assert!(check_derivation(3, &f, &Lrat { steps: Vec::new() }, &[]), "no claims, nothing to derive");
}

#[test]
fn a_cdcl_trace_and_its_lemmas_derive_what_the_solver_concluded() {
    // A satisfiable formula with a consequence the solver must learn: the pigeonhole's every-hole-taken clause set
    // without one pigeon, and the claim that pigeon 0 sits somewhere.
    let (vars, cnf) = pigeonhole(4);
    let mut s = Solver::new(vars);
    for c in &cnf {
        s.add_clause(c.clone());
    }
    assert_eq!(s.solve(), SolveResult::Unsat);
    let mut log: Vec<Vec<Lit>> = s.learned().iter().map(|c| c.lits.clone()).collect();
    log.push(Vec::new());
    let last = log.len() - 1;
    let proof = derivation(vars, &cnf, &log, &[last]).expect("the refutation's empty clause is a target like any");
    assert!(check_derivation(vars, &cnf, &proof, &[Vec::new()]));
    assert!(check(vars, &cnf, &proof), "a derivation of the empty clause is a refutation");
}

/// A fixed-seed random formula rich in short clauses over `vars` variables: units, binaries and ternaries in turn.
fn short_clauses(seed: u64, vars: u32, clauses: usize) -> Vec<Vec<Lit>> {
    let mut s = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    (0..clauses)
        .map(|k| {
            let width = [1, 2, 2, 3, 3, 3, 3][k % 7];
            (0..width).map(|_| Lit::new((next() % vars as u64) as u32, next() & 1 == 0)).collect()
        })
        .collect()
}

#[test]
fn a_log_clause_whose_negation_holds_a_literal_the_formula_forces_leaves_that_literal_s_reason_out() {
    // x0, x0 ⇒ x1, x1 ∧ y ∧ w ⇒ z, x1 ∧ z ⇒ ¬y. The formula alone forces x1; (¬x1 ∨ ¬y ∨ ¬w) is RUP through the last two
    // clauses with x1 its negation's own. A chain that took x1 from (¬x0 ∨ x1) would hint a clause the negation already
    // satisfies.
    let f = vec![vec![p(0)], vec![n(0), p(1)], vec![n(1), n(2), n(3), p(4)], vec![n(1), n(4), n(2)]];
    let log = vec![vec![n(1), n(2), n(3)]];
    let proof = derivation(5, &f, &log, &[0]).expect("RUP");
    let mut hints = proof.steps[0].hints.clone();
    hints.sort_unstable();
    assert_eq!(hints, vec![3, 4], "the two clauses the conflict needs, and nothing that forced x1");
    assert!(check_derivation(5, &f, &proof, &log));
}

#[test]
fn a_log_clause_holding_a_literal_the_formula_forces_closes_at_the_clause_that_forced_it() {
    // x0, x0 ⇒ x1, x1 ⇒ ¬x3, x1 ∧ x2 ⇒ x3: the formula forces ¬x2, so (¬x2) and (x1 ∨ ¬x2) are RUP, each refuted at
    // the clause that forced its literal.
    let f = vec![vec![p(0)], vec![n(0), p(1)], vec![n(1), n(3)], vec![n(1), n(2), p(3)]];
    let log = vec![vec![n(2)], vec![p(1), n(2)]];
    let proof = derivation(4, &f, &log, &[0, 1]).expect("RUP");
    assert!(check_derivation(4, &f, &proof, &log));
    assert_eq!(proof.steps[0].hints.last(), Some(&4), "¬x2 was forced by (¬x1 ∨ ¬x2 ∨ x3)");
    assert_eq!(proof.steps[1].hints.last(), Some(&2), "x1 was forced by (¬x0 ∨ x1)");
}

#[test]
fn a_log_clause_holding_two_literals_the_formula_forces_closes_at_the_one_forced_first() {
    // (¬x1) forces ¬x1, then (x1 ∨ ¬x0) forces ¬x0: the log clause (¬x0 ∨ ¬x1) holds both. Its negation sets x0 and x1,
    // so the chain closes at (¬x1), the clause that forced the earlier of them; one through (x1 ∨ ¬x0), whose reason
    // (¬x1) the negation falsifies first, does not replay.
    let f = vec![vec![n(1)], vec![p(1), n(0)]];
    let log = vec![vec![n(0), n(1)]];
    let proof = derivation(2, &f, &log, &[0]).expect("RUP");
    assert_eq!(proof.steps[0].hints, vec![1], "the unit that forced ¬x1 first");
    assert!(check_derivation(2, &f, &proof, &log));
}

#[test]
fn once_the_formula_conflicts_every_log_clause_is_derived_from_the_conflict() {
    // x0, x0 ⇒ x1, x1 ⇒ x2, x1 ⇒ ¬x2: the formula refutes itself by propagation, so every clause is RUP, a negation that
    // holds x2 taking it as its own.
    let f = vec![vec![p(0)], vec![n(0), p(1)], vec![n(1), p(2)], vec![n(1), n(2)]];
    let log = vec![vec![n(2), p(3)], vec![p(3)], vec![n(0), n(3)], Vec::new()];
    let proof = derivation(4, &f, &log, &[0, 1, 2, 3]).expect("everything follows from a contradiction");
    assert!(check_derivation(4, &f, &proof, &log));
    assert!(check(4, &f, &proof));
}

#[test]
fn random_formulas_rich_in_units_trim_to_proofs_that_check() {
    // Random 3-CNF near the threshold beside units and binaries: propagation decides much of each before any learned
    // clause, the fixpoint a trim starts every chain from, and the search learns the rest. Every refutation checks and
    // agrees with the naive replay, and so does every derivation of a learned clause in the middle of the trace.
    let (mut refuted, mut middles) = (0, 0);
    for seed in 1..=300u64 {
        let vars = 30 + (seed % 21) as u32;
        let mut cnf = random_3cnf(seed, vars, (vars as usize) * 4);
        cnf.extend(short_clauses(seed ^ 0x5555, vars, (vars as usize) / 3));
        let Some(learned) = solve(vars as usize, &cnf) else { continue };
        refuted += 1;
        let proof = refutation(vars as usize, &cnf, &learned);
        assert_eq!(proof.is_some(), check_refutation(vars as usize, &cnf, &learned), "seed {seed}");
        let proof = proof.unwrap();
        assert!(check(vars as usize, &cnf, &proof), "seed {seed}");
        assert!(check_lrat(vars as usize, &cnf, &proof.to_text()), "seed {seed}");
        if learned.len() >= 2 {
            let mid = learned.len() / 2;
            let part = derivation(vars as usize, &cnf, &learned, &[mid]).expect("a learned clause is RUP in its place");
            assert!(check_derivation(vars as usize, &cnf, &part, &[learned[mid].clone()]), "seed {seed}");
            middles += 1;
        }
    }
    assert!(refuted >= 150 && middles >= 20, "{refuted} refuted, {middles} derived from the middle");
}

#[test]
fn a_trim_propagates_what_the_formula_forces_once_not_once_a_learned_clause() {
    // A random 3-CNF core of 200 variables, each clause guarded by t, which the units force through (u) and (¬u ∨ t),
    // beside 50,000 units that force two literals each: the fixpoint holds 150,000 literals, and every chain of the
    // core's thousands of learned clauses rests on t alone. Propagating the fixpoint afresh for each learned clause
    // costs minutes; once, well under a second.
    let (v, noise) = (200u32, 50_000u32);
    let (u, t, first) = (v, v + 1, v + 2);
    let mut cnf: Vec<Vec<Lit>> = random_3cnf(7, v, 5 * v as usize);
    for c in &mut cnf {
        c.push(n(t));
    }
    for i in 0..noise {
        let x = first + 3 * i;
        cnf.extend([vec![p(x)], vec![n(x), p(x + 1)], vec![n(x), p(x + 2)]]);
    }
    cnf.extend([vec![p(u)], vec![n(u), p(t)]]);
    let vars = (first + 3 * noise) as usize;
    let learned = solve(vars, &cnf).expect("the core is unsatisfiable");
    assert!(learned.len() >= 1000, "a trace long enough to mean something: {}", learned.len());
    let started = std::time::Instant::now();
    let proof = refutation(vars, &cnf, &learned).expect("a refutation");
    let took = started.elapsed();
    assert!(check(vars, &cnf, &proof));
    assert!(took < std::time::Duration::from_secs(5), "{} learned clauses trimmed in {took:?}", learned.len());
}

/// The unit-propagation fixpoint of `clauses` by rescanning them all until nothing changes: the literals it sets, or
/// `None` when it falsifies a clause.
fn fixpoint(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<std::collections::BTreeSet<(u32, bool)>> {
    let mut value: Vec<Option<bool>> = vec![None; num_vars];
    loop {
        let mut changed = false;
        for c in clauses {
            if c.iter().any(|l| value[l.var() as usize] == Some(l.is_positive())) {
                continue;
            }
            let mut open: Vec<Lit> = c.iter().copied().filter(|l| value[l.var() as usize].is_none()).collect();
            open.dedup();
            open.sort_by_key(|l| (l.var(), l.is_positive()));
            open.dedup();
            match open.as_slice() {
                [] => return None,
                [u] => {
                    value[u.var() as usize] = Some(u.is_positive());
                    changed = true;
                }
                _ => {}
            }
        }
        if !changed {
            return Some(value.iter().enumerate().filter_map(|(v, b)| b.map(|b| (v as u32, b))).collect());
        }
    }
}

/// What the trimmer holds: its trail's literals, or `None` once its fixpoint conflicts.
fn held(t: &Trimmer) -> Option<std::collections::BTreeSet<(u32, bool)>> {
    match t.conflict {
        Some(_) => None,
        None => Some(t.trail.iter().map(|l| (l.var(), l.is_positive())).collect()),
    }
}

#[test]
fn the_trimmer_holds_the_fixpoint_of_the_clauses_attached_and_withdrawing_cuts_it_back() {
    // Random formulas rich in units: after each clause is attached the trail is the fixpoint of the clauses so far, or
    // the conflict is; after each is withdrawn, last first, the fixpoint of those left; and a chain asked on the way
    // leaves the trail as it found it.
    let (mut conflicted, mut forced) = (0, 0);
    for seed in 1..=200u64 {
        let vars = 6 + (seed % 9) as u32;
        let clauses = short_clauses(seed, vars, (vars as usize) * 3);
        let mut t = Trimmer::new(vars as usize);
        for i in 0..clauses.len() {
            t.attach(&clauses[i]);
            assert_eq!(held(&t), fixpoint(vars as usize, &clauses[..=i]), "seed {seed}: attached {}", i + 1);
        }
        conflicted += usize::from(t.conflict.is_some());
        for i in (0..clauses.len()).rev() {
            t.withdraw(i);
            let want = fixpoint(vars as usize, &clauses[..i]);
            assert_eq!(held(&t), want, "seed {seed}: {i} left");
            forced += want.as_ref().map_or(0, |w| w.len());
            let before = held(&t);
            let probe = [Lit::new((seed as u32 + i as u32) % vars, i % 2 == 0)];
            let _ = t.chain(&probe);
            assert_eq!(held(&t), before, "seed {seed}: a chain moved the trail");
        }
    }
    assert!(conflicted >= 50 && forced >= 1000, "{conflicted} formulas conflicted, {forced} literals forced");
}
