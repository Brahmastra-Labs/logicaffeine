use super::*;
use crate::cdcl::{SolveResult, Solver};
use crate::families;
use crate::pr::check_pr_refutation;

fn sat_brute(num_vars: usize, clauses: &[Vec<Lit>]) -> bool {
    (0u32..(1u32 << num_vars)).any(|mask| {
        let model: Vec<bool> = (0..num_vars).map(|v| (mask >> v) & 1 == 1).collect();
        clauses
            .iter()
            .all(|c| c.iter().any(|l| model[l.var() as usize] == l.is_positive()))
    })
}

#[test]
fn bve_preserves_satisfiability_brute_force() {
    // Over many seeded random small CNFs, BVE must preserve satisfiability exactly.
    let mut state = 0xB5E_1234_5678u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..2000 {
        let nv = 3 + (next() % 5) as usize; // 3..7
        let nc = (next() % 14) as usize;
        let clauses: Vec<Vec<Lit>> = (0..nc)
            .map(|_| {
                let len = 1 + (next() % 3) as usize;
                let mut c = Vec::new();
                for _ in 0..len {
                    let l = Lit::new((next() % nv as u64) as Var, next() & 1 == 0);
                    if !c.contains(&l) && !c.contains(&l.negated()) {
                        c.push(l);
                    }
                }
                c
            })
            .filter(|c: &Vec<Lit>| !c.is_empty())
            .collect();
        let (simplified, _) = bve(nv, &clauses);
        assert_eq!(
            sat_brute(nv, &clauses),
            sat_brute(nv, &simplified),
            "BVE changed satisfiability for {clauses:?}"
        );
    }
}

#[test]
fn bve_produces_a_certified_refutation_of_php() {
    // BVE-simplify PHP(n), solve the result, and verify the COMPOSED proof (BVE resolvents +
    // deletions + the solver's RUP learned clauses) refutes the ORIGINAL formula.
    for n in 2..=4 {
        let (cnf, _) = families::php(n);
        let (simplified, mut steps) = bve(cnf.num_vars, &cnf.clauses);

        let mut solver = Solver::new(cnf.num_vars);
        for c in &simplified {
            solver.add_clause(c.clone());
        }
        assert_eq!(
            solver.solve(),
            SolveResult::Unsat,
            "simplified PHP({n}) is UNSAT"
        );
        for lc in solver.learned() {
            steps.push(ProofStep::Rup(lc.lits.clone()));
        }
        assert!(
            check_pr_refutation(cnf.num_vars, &cnf.clauses, &steps),
            "BVE-composed proof must refute the original PHP({n})"
        );
    }
}

#[test]
fn eliminate_variable_yields_the_resolvent() {
    // (x ∨ a) ∧ (¬x ∨ b): eliminating x yields the single resolvent (a ∨ b), emitting one RUP
    // step and two deletions. (Full `bve` would then also clear a, b as pure literals — this
    // pins the single-variable step, where the resolvent is observable.)
    let (x, a, b) = (0u32, 1u32, 2u32);
    let clauses = vec![
        vec![Lit::pos(x), Lit::pos(a)],
        vec![Lit::neg(x), Lit::pos(b)],
    ];
    let (simplified, steps) = eliminate_variable(&clauses, x).expect("x is eliminable");
    assert!(
        simplified.iter().all(|c| !c.iter().any(|l| l.var() == x)),
        "x is gone"
    );
    assert!(
        simplified.contains(&vec![Lit::pos(a), Lit::pos(b)]),
        "resolvent (a ∨ b) present"
    );
    assert!(
        steps.iter().any(|s| matches!(s, ProofStep::Rup(_))),
        "emits the RUP resolvent"
    );
    assert_eq!(
        steps
            .iter()
            .filter(|s| matches!(s, ProofStep::Delete(_)))
            .count(),
        2,
        "deletes both x-clauses"
    );
}

#[test]
fn vivify_strengthens_via_propagation() {
    // {(a ∨ b), (¬a ∨ b)} together imply b, so (a ∨ b) vivifies down to the unit (b).
    let (a, b) = (0u32, 1u32);
    let db = vec![
        vec![Lit::pos(a), Lit::pos(b)],
        vec![Lit::neg(a), Lit::pos(b)],
    ];
    let c2 = vivify_clause(2, &db, &db[0]).expect("the clause is strengthenable");
    assert_eq!(c2, vec![Lit::pos(b)], "vivified to the unit (b)");
}

#[test]
fn vivify_preserves_satisfiability_brute_force() {
    let mut state = 0x171F_1ED_9999u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..2000 {
        let nv = 3 + (next() % 5) as usize;
        let nc = (next() % 14) as usize;
        let clauses: Vec<Vec<Lit>> = (0..nc)
            .map(|_| {
                let len = 1 + (next() % 3) as usize;
                let mut c = Vec::new();
                for _ in 0..len {
                    let l = Lit::new((next() % nv as u64) as Var, next() & 1 == 0);
                    if !c.contains(&l) && !c.contains(&l.negated()) {
                        c.push(l);
                    }
                }
                c
            })
            .filter(|c: &Vec<Lit>| !c.is_empty())
            .collect();
        let (simplified, _) = vivify(nv, &clauses);
        assert_eq!(
            sat_brute(nv, &clauses),
            sat_brute(nv, &simplified),
            "vivify changed SAT for {clauses:?}"
        );
    }
}

#[test]
fn vivify_produces_a_certified_refutation_of_php() {
    for n in 2..=4 {
        let (cnf, _) = families::php(n);
        let (simplified, mut steps) = vivify(cnf.num_vars, &cnf.clauses);
        let mut solver = Solver::new(cnf.num_vars);
        for c in &simplified {
            solver.add_clause(c.clone());
        }
        assert_eq!(solver.solve(), SolveResult::Unsat);
        for lc in solver.learned() {
            steps.push(ProofStep::Rup(lc.lits.clone()));
        }
        assert!(
            check_pr_refutation(cnf.num_vars, &cnf.clauses, &steps),
            "vivify-composed proof must refute the original PHP({n})"
        );
    }
}

#[test]
fn symmetry_then_bve_then_vivify_compose_into_one_certified_refutation() {
    // THE ultimate stack, all in one machine-checked proof: certified symmetry breaking, then
    // certified BVE, then certified vivification, then the solver — refuting PHP against the
    // original formula.
    for n in 2..=4 {
        let (cnf, _) = families::php(n);
        let gens = crate::symmetry_detect::find_generators(cnf.num_vars, &cnf.clauses);
        // 1) certified symmetry breaking (PR steps over the original).
        let (sb_db, nv, mut steps) =
            crate::sym_certify::symmetry_break_certified(cnf.num_vars, &cnf.clauses, &gens);
        // 2) certified BVE on top.
        let (bve_db, bve_steps) = bve(nv, &sb_db);
        steps.extend(bve_steps);
        // 3) certified vivification.
        let (viv_db, viv_steps) = vivify(nv, &bve_db);
        steps.extend(viv_steps);
        // 4) solve and append RUP.
        let mut solver = Solver::new(nv);
        for c in &viv_db {
            solver.add_clause(c.clone());
        }
        assert_eq!(
            solver.solve(),
            SolveResult::Unsat,
            "fully-simplified PHP({n}) is UNSAT"
        );
        for lc in solver.learned() {
            steps.push(ProofStep::Rup(lc.lits.clone()));
        }
        assert!(
            check_pr_refutation(nv, &cnf.clauses, &steps),
            "symmetry + BVE + vivify + solve must compose into ONE certified refutation of PHP({n})"
        );
    }
}
