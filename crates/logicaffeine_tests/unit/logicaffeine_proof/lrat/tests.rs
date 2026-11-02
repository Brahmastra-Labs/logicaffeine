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
