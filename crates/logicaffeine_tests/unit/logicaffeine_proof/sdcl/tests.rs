use super::*;
use crate::families;

fn sat_brute(num_vars: usize, clauses: &[Vec<Lit>]) -> bool {
    (0u32..(1u32 << num_vars)).any(|mask| {
        let model: Vec<bool> = (0..num_vars).map(|v| (mask >> v) & 1 == 1).collect();
        clauses
            .iter()
            .all(|c| c.iter().any(|l| model[l.var() as usize] == l.is_positive()))
    })
}

#[test]
fn sdcl_rediscovers_a_pigeonhole_pr_clause_with_no_symmetry_input() {
    // No automorphism, no Heule construction — given only the stuck attempt "pigeon 0 in the
    // last hole", SDCL's positive reduct DISCOVERS that ¬x(0, last_hole) is propagation-
    // redundant: the very clause certified symmetry breaking adds, found from structure alone.
    for n in 3..=4 {
        let (cnf, _) = families::php(n);
        let last_hole = n - 2;
        let stuck = vec![Lit::pos(last_hole as Var)]; // x(0, last_hole) = 0*(n-1)+last_hole
        let (c, w) = find_pr_clause(cnf.num_vars, &cnf.clauses, &stuck)
            .expect("SDCL must discover a PR clause");
        assert_eq!(
            c,
            vec![Lit::neg(last_hole as Var)],
            "rediscovered ¬x(0, last_hole)"
        );
        assert!(
            is_pr(cnf.num_vars, &cnf.clauses, &c, &w),
            "and it is genuinely PR"
        );
    }
}

#[test]
fn sdcl_refutes_pigeonhole_fully_automatically() {
    // THE payoff: no symmetry, no Heule construction, no hint — sdcl_refute discovers the
    // certified Extended-Frege refutation of pigeonhole entirely on its own, and the composed
    // proof re-checks against the original formula.
    for n in 2..=4 {
        let (cnf, _) = families::php(n);
        let r = sdcl_refute(cnf.num_vars, &cnf.clauses);
        assert!(r.refuted, "SDCL must auto-refute PHP({n})");
        assert!(
            check_pr_refutation(cnf.num_vars, &cnf.clauses, &r.steps),
            "the self-discovered proof must independently re-check"
        );
        // PHP(2) is directly RUP-refutable (no PR needed); from n≥3 SDCL must discover PR clauses.
        if n >= 3 {
            assert!(
                r.sbp_clauses >= 1,
                "PHP({n}) needs self-discovered PR clauses"
            );
        }
    }
}

#[test]
#[ignore = "scaling SDCL — auto-refutes larger pigeonhole via discovered substitution witnesses"]
fn sdcl_refute_scales_with_symmetry_fallback() {
    use crate::cdcl::{SolveResult, Solver};
    for n in 5..=7 {
        let (cnf, _) = families::php(n);
        let r = sdcl_refute(cnf.num_vars, &cnf.clauses);
        assert!(r.refuted, "scaling SDCL must auto-refute PHP({n})");
        assert!(r.sbp_clauses >= 1, "discovered PR clauses at scale");
        assert!(check_pr_refutation(cnf.num_vars, &cnf.clauses, &r.steps));
        // How much search was left after the self-discovered breaking?
        let sbp: Vec<Vec<Lit>> = r
            .steps
            .iter()
            .filter_map(|s| {
                if let ProofStep::Pr { clause, .. } = s {
                    Some(clause.clone())
                } else {
                    None
                }
            })
            .collect();
        let mut solver = Solver::new(cnf.num_vars);
        for c in cnf.clauses.iter().chain(sbp.iter()) {
            solver.add_clause(c.clone());
        }
        assert_eq!(solver.solve(), SolveResult::Unsat);
        println!(
            "SDCL PHP({n}): {} self-discovered PR clauses (no symmetry input) → {} conflicts left, CERTIFIED",
            r.sbp_clauses,
            solver.conflicts()
        );
    }
}

#[test]
fn solve_certified_is_a_complete_certified_solver() {
    use crate::pr::check_pr_refutation;
    // UNSAT side: pigeonhole → a machine-checked refutation that re-checks against the original.
    let (php, _) = families::php(4);
    match solve_certified(php.num_vars, &php.clauses) {
        CertifiedOutcome::Unsat { steps, discovered } => {
            assert!(discovered >= 1, "SDCL discovered breaking clauses");
            assert!(
                check_pr_refutation(php.num_vars, &php.clauses, &steps),
                "PHP(4) refutation re-checks"
            );
        }
        CertifiedOutcome::Sat(_) => panic!("PHP(4) is UNSAT"),
    }
    // SAT side: (a ∨ b) ∧ (¬a ∨ b) forces b → satisfiable, and the model must satisfy it.
    let sat = vec![
        vec![Lit::pos(0), Lit::pos(1)],
        vec![Lit::neg(0), Lit::pos(1)],
    ];
    match solve_certified(2, &sat) {
        CertifiedOutcome::Sat(m) => {
            assert!(
                sat.iter()
                    .all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())),
                "valid model"
            );
        }
        CertifiedOutcome::Unsat { .. } => panic!("formula is satisfiable"),
    }
}

#[test]
fn solve_certified_matches_brute_force_with_checkable_proofs() {
    use crate::pr::check_pr_refutation;
    // The whole unified engine, verdict-invariant: over many random formulas it must agree with
    // brute force, every SAT answer carrying a valid model, every UNSAT answer a proof that
    // re-checks against the original. Soundness end to end.
    let mut state = 0x501_E_CE271F1Eu64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..600 {
        let nv = 3 + (next() % 4) as usize; // 3..6
        let nc = (next() % 12) as usize;
        let clauses: Vec<Vec<Lit>> = (0..nc)
            .map(|_| {
                (0..2 + (next() % 2) as usize)
                    .map(|_| Lit::new((next() % nv as u64) as Var, next() & 1 == 0))
                    .collect::<Vec<_>>()
            })
            .filter(|c: &Vec<Lit>| !c.is_empty())
            .collect();
        let expected = sat_brute(nv, &clauses);
        match solve_certified(nv, &clauses) {
            CertifiedOutcome::Sat(m) => {
                assert!(
                    expected,
                    "solver said SAT but brute force says UNSAT: {clauses:?}"
                );
                assert!(
                    clauses
                        .iter()
                        .all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())),
                    "model invalid"
                );
            }
            CertifiedOutcome::Unsat { steps, .. } => {
                assert!(
                    !expected,
                    "solver said UNSAT but a model exists: {clauses:?}"
                );
                assert!(
                    check_pr_refutation(nv, &clauses, &steps),
                    "UNSAT proof must re-check: {clauses:?}"
                );
            }
        }
    }
}

#[test]
fn sdcl_discovered_clauses_preserve_satisfiability() {
    // Robustness: over many seeded random formulas and stuck assignments, whenever SDCL returns
    // a clause, adding it preserves satisfiability exactly (brute force). The PR gate guarantees
    // it; this proves it.
    let mut state = 0x5DC1_9999u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let mut found = 0;
    for _ in 0..4000 {
        let nv = 3 + (next() % 4) as usize; // 3..6
        let nc = (next() % 12) as usize;
        let clauses: Vec<Vec<Lit>> = (0..nc)
            .map(|_| {
                (0..2 + (next() % 2) as usize)
                    .map(|_| Lit::new((next() % nv as u64) as Var, next() & 1 == 0))
                    .collect::<Vec<_>>()
            })
            .filter(|c: &Vec<Lit>| !c.is_empty())
            .collect();
        // A random partial assignment as the "stuck" state.
        let mut alpha: Vec<Lit> = Vec::new();
        for v in 0..nv as Var {
            if next() & 1 == 0 {
                alpha.push(Lit::new(v, next() & 1 == 0));
            }
        }
        if alpha.is_empty() {
            continue;
        }
        if let Some((c, _)) = find_pr_clause(nv, &clauses, &alpha) {
            found += 1;
            let before = sat_brute(nv, &clauses);
            let mut with = clauses.clone();
            with.push(c.clone());
            assert_eq!(
                before,
                sat_brute(nv, &with),
                "SDCL clause changed satisfiability: {clauses:?} C={c:?}"
            );
        }
    }
    assert!(found > 0, "the discovery path must actually be exercised");
}
