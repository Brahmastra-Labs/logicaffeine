use super::*;
use crate::cdcl::{SolveResult, Solver};

/// Brute-force satisfiability over `num_vars` variables — the independent oracle.
fn sat_brute(num_vars: usize, clauses: &[Vec<Lit>]) -> bool {
    for mask in 0u32..(1u32 << num_vars) {
        let model: Vec<bool> = (0..num_vars).map(|v| (mask >> v) & 1 == 1).collect();
        if clauses
            .iter()
            .all(|c| c.iter().any(|l| model[l.var() as usize] == l.is_positive()))
        {
            return true;
        }
    }
    false
}

#[test]
fn sel_never_reports_pigeonhole_satisfiable() {
    // Regression guard: PHP is UNSAT at every size. A larger instance runs long enough to
    // trigger clause-DB reduction inside the budgeted solve — which once deleted the original
    // clauses and produced a bogus SAT. SEL must return UNSAT (or honest Unknown), NEVER SAT.
    for n in 7..=7 {
        let (cnf, _) = crate::families::php(n);
        match sel_refute(cnf.num_vars, &cnf.clauses) {
            SelOutcome::Sat(_) => panic!("PHP({n}) reported SATISFIABLE — soundness violation"),
            SelOutcome::Unsat { steps, .. } => {
                assert!(crate::pr::check_pr_refutation_fast(
                    cnf.num_vars,
                    &cnf.clauses,
                    &steps
                ));
            }
            SelOutcome::Unknown { .. } => {}
        }
    }
}

#[test]
fn sel_certifies_pigeonhole() {
    // SEL must refute PHP and the accumulated RUP proof must independently check.
    for n in 3..=6 {
        let (cnf, _) = crate::families::php(n);
        match sel_refute(cnf.num_vars, &cnf.clauses) {
            SelOutcome::Unsat { steps, .. } => {
                assert!(
                    crate::pr::check_pr_refutation_fast(cnf.num_vars, &cnf.clauses, &steps),
                    "PHP({n}) SEL proof must check"
                );
            }
            other => panic!("PHP({n}) must be refuted, got {other:?}"),
        }
    }
}

#[test]
fn sel_amplification_cuts_conflicts_on_pigeonhole() {
    // The power metric: total conflicts under SEL must be strictly below plain CDCL on a
    // symmetry-rich instance — the symmetric twins of each lemma come for free.
    let (cnf, _) = crate::families::php(6);
    let mut plain = Solver::new(cnf.num_vars);
    for c in &cnf.clauses {
        plain.add_clause(c.clone());
    }
    assert_eq!(plain.solve(), SolveResult::Unsat);
    let plain_conflicts = plain.conflicts();

    match sel_refute(cnf.num_vars, &cnf.clauses) {
        SelOutcome::Unsat {
            conflicts,
            amplified,
            ..
        } => {
            assert!(
                amplified > 0,
                "symmetry amplification must actually fire on PHP"
            );
            eprintln!(
                "PHP(6): plain CDCL = {plain_conflicts} conflicts, SEL = {conflicts} conflicts ({amplified} symmetric clauses), {:.1}x fewer",
                plain_conflicts as f64 / conflicts.max(1) as f64
            );
            assert!(
                conflicts < plain_conflicts,
                "SEL conflicts ({conflicts}) must beat plain CDCL ({plain_conflicts})"
            );
        }
        other => panic!("expected refutation, got {other:?}"),
    }
}

#[test]
fn sel_never_returns_a_wrong_verdict_random() {
    // Soundness to the point of absurdity: over many seeded random small formulas, SEL must
    // never contradict brute force — a `Unsat` only on truly UNSAT instances (with a checking
    // proof), a `Sat(m)` only with a real model. `Unknown` is always permitted.
    let mut state = 0xC0FFEE123456789Au64;
    let mut next = || {
        state = state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    };
    let num_vars = 5usize;
    for _ in 0..1500 {
        let nclauses = next() as usize % 12;
        let clauses: Vec<Vec<Lit>> = (0..nclauses)
            .map(|_| {
                let len = 1 + (next() as usize % 3);
                let mut c = Vec::new();
                for _ in 0..len {
                    let v = (next() as u32) % num_vars as u32;
                    let lit = Lit::new(v, next() & 1 == 0);
                    if !c.contains(&lit) && !c.contains(&lit.negated()) {
                        c.push(lit);
                    }
                }
                c
            })
            .filter(|c| !c.is_empty())
            .collect();
        let truth = sat_brute(num_vars, &clauses);
        match sel_refute(num_vars, &clauses) {
            SelOutcome::Unsat { steps, .. } => {
                assert!(!truth, "SEL refuted a satisfiable formula: {clauses:?}");
                assert!(
                    crate::pr::check_pr_refutation_fast(num_vars, &clauses, &steps),
                    "SEL Unsat proof must check: {clauses:?}"
                );
            }
            SelOutcome::Sat(model) => {
                assert!(
                    truth,
                    "SEL claimed SAT on an unsatisfiable formula: {clauses:?}"
                );
                assert!(
                    clauses
                        .iter()
                        .all(|c| c.iter().any(|l| model[l.var() as usize] == l.is_positive())),
                    "SEL returned an invalid model"
                );
            }
            SelOutcome::Unknown { .. } => {}
        }
    }
}
